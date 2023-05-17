use anyhow::{anyhow, Context as AnyhowContext};
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
    max_threads_length: u32,
}

impl SlotsProcessor {
    pub fn try_new(context: Context) -> Result<Self, SlotsProcessorError> {
        let max_threads_length = std::thread::available_parallelism()
            .with_context(|| "Failed to get maximum thread length")?
            .get() as u32;

        Ok(Self {
            context,
            max_threads_length,
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

        let slots_chunk = end_slot - start_slot;
        let slots_per_thread = slots_chunk / self.max_threads_length;
        let threads_length = if slots_per_thread > 0 {
            self.max_threads_length
        } else {
            slots_chunk
        };
        let mut handles: Vec<JoinHandle<Result<(), SlotsChunkThreadError>>> = vec![];
        let mut current_slot = start_slot;

        for i in 0..threads_length {
            let thread_slots_chunk = if i == 0 {
                slots_per_thread + slots_chunk % self.max_threads_length
            } else {
                slots_per_thread
            };

            let thread_context = self.context.clone();
            let thread_initial_slot = current_slot;
            let thread_final_slot = current_slot + thread_slots_chunk;

            let thread_slots_span = tracing::info_span!(
                "slots_chunk_processor",
                chunk_initial_slot = thread_initial_slot,
                chunk_final_slot = thread_final_slot
            );

            let handle = tokio::spawn(
                async move {
                    let slot_processor = SlotProcessor::new(thread_context);

                    for current_slot in thread_initial_slot..thread_final_slot {
                        let slot_span = tracing::info_span!("slot_processor", slot = current_slot);

                        let result = slot_processor
                            .process_slot(current_slot)
                            .instrument(slot_span)
                            .await;

                        if let Err(error) = result {
                            error!("Failed to process slot {current_slot}: {error}");

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

            current_slot += thread_slots_chunk;
        }

        let handle_outputs = join_all(handles).await;

        let mut errors = vec![];

        for handle in handle_outputs {
            match handle {
                Ok(thread_result) => match thread_result {
                    Ok(_) => (),
                    Err(error) => errors.push(error),
                },
                Err(join_error) => {
                    let err = anyhow!(format!("Slots processor thread panicked: {:?}", join_error));
                    errors.push(SlotsChunkThreadError::Other(err));
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
