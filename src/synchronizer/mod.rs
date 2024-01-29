use std::{cmp::Ordering, thread};

use anyhow::anyhow;
use futures::future::join_all;
use tokio::task::JoinHandle;
use tracing::{debug, debug_span, error, info, Instrument};

use crate::{context::Context, slot_processor::SlotProcessor};

use self::error::{MultipleSlotsChunkErrors, SynchronizerError, SynchronizerThreadError};

mod error;

#[derive(Debug)]
pub struct SynchronizerBuilder {
    num_threads: u32,
    slots_checkpoint: u32,
}

impl SynchronizerBuilder {
    pub fn new() -> Result<Self, anyhow::Error> {
        SynchronizerBuilder::default()
    }

    pub fn default() -> Result<Self, anyhow::Error> {
        Ok(Self {
            num_threads: thread::available_parallelism()
                .map_err(|err| anyhow!("Failed to get number of available threads: {:?}", err))?
                .get() as u32,
            slots_checkpoint: 1000,
        })
    }

    pub fn with_num_threads(&mut self, num_threads: u32) -> &mut Self {
        self.num_threads = num_threads;
        self
    }

    pub fn with_slots_checkpoint(&mut self, slots_checkpoint: u32) -> &mut Self {
        self.slots_checkpoint = slots_checkpoint;
        self
    }

    pub fn build(&self, context: Context) -> Synchronizer {
        Synchronizer {
            context,
            num_threads: self.num_threads,
            slots_checkpoint: self.slots_checkpoint,
        }
    }
}

pub struct Synchronizer {
    context: Context,
    num_threads: u32,
    slots_checkpoint: u32,
}

impl Synchronizer {
    async fn _sync_slots(&self, from_slot: u32, to_slot: u32) -> Result<(), SynchronizerError> {
        if from_slot == to_slot {
            return Ok(());
        }

        let unprocessed_slots = to_slot - from_slot;
        let num_threads = std::cmp::min(self.num_threads, unprocessed_slots);
        let slots_per_thread = unprocessed_slots / num_threads;
        let remaining_slots = unprocessed_slots % num_threads;
        let num_threads = if slots_per_thread > 0 {
            num_threads
        } else {
            unprocessed_slots
        };

        let mut handles: Vec<JoinHandle<Result<(), SynchronizerThreadError>>> = vec![];

        for i in 0..num_threads {
            let slots_in_current_thread = if i == num_threads - 1 {
                slots_per_thread + remaining_slots
            } else {
                slots_per_thread
            };

            let thread_context = self.context.clone();
            let thread_initial_slot = from_slot + i * slots_per_thread;
            let thread_final_slot = thread_initial_slot + slots_in_current_thread;

            let synchronizer_thread_span = tracing::trace_span!(
                "synchronizer_thread",
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
                                target = "synchronizer",
                                current_slot,
                                ?error,
                                "Failed to process slot"
                            );

                            return Err(SynchronizerThreadError::FailedSlotsChunkProcessing {
                                initial_slot: thread_initial_slot,
                                final_slot: thread_final_slot,
                                failed_slot: current_slot,
                                error,
                            });
                        }
                    }

                    Ok(())
                }
                .instrument(synchronizer_thread_span),
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
                    let err = anyhow!("Synchronizer thread panicked: {:?}", error);

                    error!(
                        target = "synchronizer",
                        ?error,
                        "Synchronizer thread panicked"
                    );

                    errors.push(err.into());
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(SynchronizerError::FailedSlotsProcessing {
                initial_slot: from_slot,
                final_slot: to_slot,
                chunk_errors: MultipleSlotsChunkErrors(errors),
            })
        }
    }

    pub async fn run(&self, from_slot: u32, to_slot: u32) -> Result<(), SynchronizerError> {
        match from_slot.cmp(&to_slot) {
            Ordering::Equal => {
                return Ok(());
            }
            Ordering::Greater => {
                let err =
                    anyhow!("Starting slot ({from_slot}) is greater than final slot ({to_slot})");

                error!(
                    target = "synchronizer",
                    current_slot = from_slot,
                    latest_slot = to_slot,
                    "{}",
                    err.to_string()
                );

                return Err(err.into());
            }
            Ordering::Less => {
                let blobscan_client = self.context.blobscan_client();
                let mut current_slot = from_slot;
                let mut unprocessed_slots = to_slot - current_slot;

                info!(
                    target = "synchronizer",
                    to_slot, from_slot, "Syncing {unprocessed_slots} slots…"
                );

                while unprocessed_slots > 0 {
                    let slots_chunk = std::cmp::min(unprocessed_slots, self.slots_checkpoint);
                    let initial_chunk_slot = current_slot;
                    let final_chunk_slot = current_slot + slots_chunk;

                    let sync_slots_chunk_span = debug_span!(
                        "synchronizer",
                        initial_slot = initial_chunk_slot,
                        final_slot = final_chunk_slot
                    );

                    self._sync_slots(initial_chunk_slot, final_chunk_slot)
                        .instrument(sync_slots_chunk_span)
                        .await?;

                    if let Err(error) = blobscan_client.update_slot(final_chunk_slot - 1).await {
                        error!(
                            target = "synchronizer",
                            new_latest_slot = final_chunk_slot - 1,
                            ?error,
                            "Failed to update indexer's latest slot"
                        );

                        return Err(error.into());
                    }

                    debug!(
                        target = "synchronizer",
                        latest_slot = final_chunk_slot - 1,
                        "Checkpoint reached. Latest indexed slot updated"
                    );

                    current_slot += slots_chunk;
                    unprocessed_slots -= slots_chunk;
                }
            }
        }

        Ok(())
    }
}
