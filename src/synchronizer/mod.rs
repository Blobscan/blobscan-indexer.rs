use std::{cmp::Ordering, thread};

use anyhow::anyhow;
use futures::future::join_all;
use tokio::task::JoinHandle;
use tracing::{debug, debug_span, error, info, Instrument};

use crate::{
    clients::beacon::types::{BlockHeader, BlockId},
    context::Context,
    slots_processor::{error::SlotsProcessorError, SlotsProcessor},
};

use self::error::{SlotsChunksErrors, SynchronizerError};

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
            enable_parallel_processing: self.num_threads > 1,
        }
    }
}

pub struct Synchronizer {
    context: Context,
    num_threads: u32,
    slots_checkpoint: u32,
    enable_parallel_processing: bool,
}

impl Synchronizer {
    pub async fn run(
        &self,
        initial_block_id: &BlockId,
        final_block_id: &BlockId,
    ) -> Result<BlockHeader, SynchronizerError> {
        let initial_block_slot = match initial_block_id {
            BlockId::Slot(slot) => *slot,
            _ => {
                self._fetch_block_header(initial_block_id)
                    .await?
                    .header
                    .message
                    .slot
            }
        };
        let mut final_block_header = self._fetch_block_header(final_block_id).await?;
        let final_block_slot = final_block_header.header.message.slot;

        loop {
            match initial_block_slot.cmp(&final_block_slot) {
                Ordering::Equal => {
                    return Ok(final_block_header);
                }
                Ordering::Less => {
                    self._sync_slots_by_checkpoints(initial_block_slot, final_block_slot)
                        .await?;
                }
                Ordering::Greater => {
                    let err = anyhow!("Initial block slot is greater than final one");

                    error!(
                        target = "synchronizer",
                        initial_block_slot,
                        final_block_slot,
                        "{}",
                        err.to_string()
                    );

                    return Err(err.into());
                }
            }

            /*
             * If provided final block ID is a slot, we can stop syncing once we reach it. Otherwise,
             * we need to keep fetching the latest block header to check if the final block slot has been
             * reached.
             */
            match final_block_id {
                BlockId::Slot(_) => return Ok(final_block_header),
                _ => {
                    let latest_final_block_header =
                        self._fetch_block_header(final_block_id).await?;

                    if latest_final_block_header.header.message.slot == final_block_slot {
                        return Ok(final_block_header);
                    }

                    final_block_header = latest_final_block_header;
                }
            }
        }
    }

    pub fn enable_parallel_processing(&mut self, enable_parallel_processing: bool) -> &mut Self {
        self.enable_parallel_processing = enable_parallel_processing;

        self
    }

    async fn _sync_slots_in_parallel(
        &self,
        from_slot: u32,
        to_slot: u32,
    ) -> Result<(), SynchronizerError> {
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

        let mut handles: Vec<JoinHandle<Result<(), SlotsProcessorError>>> = vec![];

        for i in 0..num_threads {
            let slots_in_current_thread = if i == num_threads - 1 {
                slots_per_thread + remaining_slots
            } else {
                slots_per_thread
            };

            let slots_processor = SlotsProcessor::new(self.context.clone());
            let thread_initial_slot = from_slot + i * slots_per_thread;
            let thread_final_slot = thread_initial_slot + slots_in_current_thread;

            let synchronizer_thread_span = tracing::trace_span!(
                "synchronizer_thread",
                chunk_initial_slot = thread_initial_slot,
                chunk_final_slot = thread_final_slot
            );

            let handle = tokio::spawn(
                async move {
                    slots_processor
                        .process_slots(thread_initial_slot, thread_final_slot)
                        .await
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
            Err(SynchronizerError::FailedParallelSlotsProcessing {
                initial_slot: from_slot,
                final_slot: to_slot,
                chunk_errors: SlotsChunksErrors(errors),
            })
        }
    }

    async fn _sync_slots_by_checkpoints(
        &self,
        initial_slot: u32,
        final_slot: u32,
    ) -> Result<(), SynchronizerError> {
        let blobscan_client = self.context.blobscan_client();

        let mut current_slot = initial_slot;
        let mut unprocessed_slots = final_slot - current_slot;

        info!(
            target = "synchronizer",
            initial_slot, final_slot, "Syncing {unprocessed_slots} slots…"
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

            if self.enable_parallel_processing {
                self._sync_slots_in_parallel(initial_chunk_slot, final_chunk_slot)
                    .instrument(sync_slots_chunk_span)
                    .await?;
            } else {
                let slots_processor = SlotsProcessor::new(self.context.clone());

                slots_processor
                    .process_slots(initial_chunk_slot, final_chunk_slot)
                    .instrument(sync_slots_chunk_span)
                    .await?;
            }

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

        Ok(())
    }

    async fn _fetch_block_header(
        &self,
        block_id: &BlockId,
    ) -> Result<BlockHeader, SynchronizerError> {
        let beacon_client = self.context.beacon_client();

        match beacon_client.get_block_header(block_id).await? {
            Some(block_header) => Ok(block_header),
            None => {
                let err = anyhow!("Block header not found for block ID {}", block_id);

                error!(target = "synchronizer", "{}", err.to_string());

                Err(err.into())
            }
        }
    }
}
