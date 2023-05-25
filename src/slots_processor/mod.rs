use anyhow::anyhow;
use futures::future::join_all;
use tokio::task::JoinHandle;
use tracing::{error, Instrument};

use self::{
    error::{MultipleSlotChunkErrors, SlotsChunkThreadError, SlotsProcessorError},
    slot_processor::SlotProcessor,
};
use crate::context::Context;

mod error;
mod slot_processor;

pub struct SlotsProcessor {
    context: Context,
    config: Config,
}

pub struct Config {
    pub threads_length: u32,
}

impl SlotsProcessor {
    pub fn try_new(context: Context, config: Option<Config>) -> Result<Self, SlotsProcessorError> {
        Ok(Self {
            context,
            config: match config {
                Some(config) => config,
                None => Config {
                    threads_length: std::thread::available_parallelism()
                        .map_err(|err| anyhow!("Failed to default thread amount: {err}"))?
                        .get() as u32,
                },
            },
        })
    }

    pub async fn process_slots(
        &self,
        start_slot: u32,
        end_slot: u32,
    ) -> Result<(), SlotsProcessorError> {
        if start_slot == end_slot {
            return Ok(());
        }

        let unprocessed_slots = end_slot - start_slot;
        let num_threads = std::cmp::min(self.config.threads_length, unprocessed_slots);
        let slots_per_thread = unprocessed_slots / num_threads;
        let remaining_slots = unprocessed_slots % num_threads;
        let num_threads = if slots_per_thread > 0 {
            num_threads
        } else {
            unprocessed_slots
        };

        let mut handles: Vec<JoinHandle<Result<(), SlotsChunkThreadError>>> = vec![];

        for i in 0..num_threads {
            let slots_in_current_thread = if i == num_threads - 1 {
                slots_per_thread + remaining_slots
            } else {
                slots_per_thread
            };

            let thread_context = self.context.clone();
            let thread_initial_slot = start_slot + i * slots_per_thread;
            let thread_final_slot = thread_initial_slot + slots_in_current_thread;

            let thread_slots_span = tracing::trace_span!(
                "slots_chunk_processor",
                chunk_initial_slot = thread_initial_slot,
                chunk_final_slot = thread_final_slot
            );

            let handle = tokio::spawn(
                async move {
                    let slot_processor = SlotProcessor::new(thread_context);

                    for current_slot in thread_initial_slot..thread_final_slot {
                        let slot_span = tracing::trace_span!("slot_processor");

                        let result = slot_processor
                            .process_slot(current_slot)
                            .instrument(slot_span)
                            .await;

                        if let Err(error) = result {
                            error!(
                                target = "slots_processor",
                                current_slot,
                                ?error,
                                "Failed to process slot"
                            );

                            return Err(SlotsChunkThreadError::FailedChunkProcessing {
                                initial_slot: thread_initial_slot,
                                final_slot: thread_final_slot,
                                failed_slot: current_slot,
                                error,
                            });
                        }
                    }

                    Ok(())
                }
                .instrument(thread_slots_span),
            );

            handles.push(handle);
        }

        let handle_outputs = join_all(handles).await;

        let mut errors = vec![];

        for handle in handle_outputs {
            match handle {
                Ok(thread_result) => match thread_result {
                    Ok(_) => (),
                    Err(error) => errors.push(error),
                },
                Err(error) => {
                    let err = anyhow!("Slots processing thread panicked: {:?}", error);

                    error!(
                        target = "slots_processor",
                        ?error,
                        "Slots processing thread panicked"
                    );

                    errors.push(err.into());
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(SlotsProcessorError::FailedSlotsProcessing {
                initial_slot: start_slot,
                final_slot: end_slot,
                chunk_errors: MultipleSlotChunkErrors(errors),
            })
        }
    }
}
