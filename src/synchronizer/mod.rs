use anyhow::anyhow;
use futures::future::join_all;
use tokio::task::JoinHandle;
use tracing::{debug, debug_span, info, Instrument};

use crate::{
    clients::{beacon::types::BlockId, blobscan::types::BlockchainSyncState, common::ClientError},
    context::Context,
    slots_processor::{error::SlotsProcessorError, SlotsProcessor},
};

use self::error::{SlotsChunksErrors, SynchronizerError};

pub mod error;

#[derive(Debug)]
pub struct SynchronizerBuilder {
    num_threads: u32,
    min_slots_per_thread: u32,
    slots_checkpoint: u32,
    disable_checkpoints: bool,
}

pub struct Synchronizer {
    context: Context,
    num_threads: u32,
    min_slots_per_thread: u32,
    slots_checkpoint: u32,
    disable_checkpoints: bool,
}

impl Default for SynchronizerBuilder {
    fn default() -> Self {
        SynchronizerBuilder {
            num_threads: 1,
            min_slots_per_thread: 50,
            slots_checkpoint: 1000,
            disable_checkpoints: false,
        }
    }
}

impl SynchronizerBuilder {
    pub fn new() -> Self {
        SynchronizerBuilder::default()
    }

    pub fn with_disable_checkpoints(&mut self, disable_checkpoints: bool) -> &mut Self {
        self.disable_checkpoints = disable_checkpoints;

        self
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
            min_slots_per_thread: self.min_slots_per_thread,
            slots_checkpoint: self.slots_checkpoint,
            disable_checkpoints: self.disable_checkpoints,
        }
    }
}

impl Synchronizer {
    pub async fn run(
        &mut self,
        initial_block_id: &BlockId,
        final_block_id: &BlockId,
    ) -> Result<(), SynchronizerError> {
        let initial_slot = self._resolve_to_slot(initial_block_id).await?;
        let mut final_slot = self._resolve_to_slot(final_block_id).await?;

        loop {
            if self.disable_checkpoints {
                self._sync_slots(initial_slot, final_slot).await?;
            } else {
                self._sync_slots_by_checkpoints(initial_slot, final_slot)
                    .await?;
            }

            let latest_final_slot = self._resolve_to_slot(final_block_id).await?;

            if final_slot == latest_final_slot {
                return Ok(());
            }

            final_slot = latest_final_slot;
        }
    }

    async fn _sync_slots(&mut self, from_slot: u32, to_slot: u32) -> Result<(), SynchronizerError> {
        let is_reverse_sync = to_slot < from_slot;
        let unprocessed_slots = to_slot.abs_diff(from_slot) + 1;
        let slots_per_thread = std::cmp::max(
            self.min_slots_per_thread,
            unprocessed_slots / self.num_threads,
        );
        let num_threads = std::cmp::max(1, unprocessed_slots / slots_per_thread);
        let remaining_slots = unprocessed_slots % num_threads;

        let mut handles: Vec<JoinHandle<Result<(), SlotsProcessorError>>> = vec![];

        for i in 0..num_threads {
            let mut slots_processor = SlotsProcessor::new(self.context.clone());
            let thread_total_slots = slots_per_thread
                + if i == num_threads - 1 {
                    remaining_slots
                } else {
                    0
                };
            let thread_initial_slot = if is_reverse_sync {
                from_slot - i * slots_per_thread
            } else {
                from_slot + i * slots_per_thread
            };
            let thread_final_slot = if is_reverse_sync {
                thread_initial_slot - thread_total_slots + 1
            } else {
                thread_initial_slot + thread_total_slots - 1
            };

            let synchronizer_thread_span = tracing::trace_span!(
                "synchronizer_thread",
                chunk_initial_slot = thread_initial_slot,
                chunk_final_slot = thread_final_slot
            );

            let handle = tokio::spawn(
                async move {
                    slots_processor
                        .process_slots(thread_initial_slot, thread_final_slot)
                        .await?;

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
                    Ok(()) => {}
                    Err(error) => errors.push(error),
                },
                Err(error) => {
                    let err = anyhow!("Synchronizer thread panicked: {:?}", error);

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
        &mut self,
        initial_slot: u32,
        final_slot: u32,
    ) -> Result<(), SynchronizerError> {
        let is_reverse_sync = final_slot < initial_slot;
        let mut current_slot = initial_slot;
        let mut unprocessed_slots = final_slot.abs_diff(current_slot) + 1;

        info!(
            target = "synchronizer",
            initial_slot,
            final_slot,
            reverse_sync = is_reverse_sync,
            "Processing {unprocessed_slots} slots…"
        );

        while unprocessed_slots > 0 {
            let slots_chunk = std::cmp::min(unprocessed_slots, self.slots_checkpoint);
            let initial_chunk_slot = current_slot;
            let final_chunk_slot = if is_reverse_sync {
                current_slot - slots_chunk + 1
            } else {
                current_slot + slots_chunk - 1
            };

            let sync_slots_chunk_span = debug_span!(
                "synchronizer",
                initial_slot = initial_chunk_slot,
                final_slot = final_chunk_slot
            );

            self._sync_slots(initial_chunk_slot, final_chunk_slot)
                .instrument(sync_slots_chunk_span)
                .await?;

            let last_slot = Some(final_chunk_slot);
            let last_lower_synced_slot = if is_reverse_sync { last_slot } else { None };
            let last_upper_synced_slot = if is_reverse_sync { None } else { last_slot };

            if let Err(error) = self
                .context
                .blobscan_client()
                .update_sync_state(BlockchainSyncState {
                    last_finalized_block: None,
                    last_lower_synced_slot,
                    last_upper_synced_slot,
                })
                .await
            {
                let new_synced_slot = match last_lower_synced_slot {
                    Some(slot) => slot,
                    None => match last_upper_synced_slot {
                        Some(slot) => slot,
                        None => {
                            return Err(SynchronizerError::Other(anyhow!(
                                "Failed to get new last synced slot: last_lower_synced_slot and last_upper_synced_slot are both None"
                            )))
                        }
                    },
                };

                return Err(SynchronizerError::FailedSlotCheckpointSave {
                    slot: new_synced_slot,
                    error,
                });
            }

            if unprocessed_slots >= self.slots_checkpoint {
                debug!(
                    target = "synchronizer",
                    new_last_lower_synced_slot = last_lower_synced_slot,
                    new_last_upper_synced_slot = last_upper_synced_slot,
                    "Checkpoint reached. Last synced slot saved…"
                );
            }

            current_slot = if is_reverse_sync {
                current_slot - slots_chunk
            } else {
                current_slot + slots_chunk
            };

            unprocessed_slots -= slots_chunk;
        }

        Ok(())
    }

    async fn _resolve_to_slot(&self, block_id: &BlockId) -> Result<u32, SynchronizerError> {
        let beacon_client = self.context.beacon_client();

        let resolved_block_id: Result<u32, ClientError> = match block_id {
            BlockId::Slot(slot) => Ok(*slot),
            _ => match beacon_client.get_block_header(block_id).await {
                Ok(None) => {
                    let err = anyhow!("Block ID {} not found", block_id);

                    Err(err.into())
                }
                Ok(Some(block_header)) => Ok(block_header.header.message.slot),
                Err(error) => Err(error),
            },
        };

        match resolved_block_id {
            Ok(slot) => Ok(slot),
            Err(error) => Err(SynchronizerError::FailedBlockIdResolution {
                block_id: block_id.clone(),
                error,
            }),
        }
    }
}
