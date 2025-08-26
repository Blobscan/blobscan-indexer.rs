use std::fmt::Debug;

use anyhow::anyhow;
use async_trait::async_trait;
use futures::future::join_all;
use tokio::task::JoinHandle;
use tracing::{debug, info, Instrument};

#[cfg(test)]
use mockall::automock;

use crate::{
    clients::{
        beacon::types::{BlockHeader, BlockId, BlockIdResolution},
        blobscan::types::BlockchainSyncState,
    },
    context::CommonContext,
    slots_processor::{error::SlotsProcessorError, SlotsProcessor},
};

use self::error::{SlotsChunksErrors, SynchronizerError};

pub mod error;

pub type SynchronizerResult = Result<(), SynchronizerError>;

#[async_trait]
#[cfg_attr(test, automock)]
pub trait CommonSynchronizer: Send + Sync {
    fn set_checkpoint(&mut self, checkpoint: Option<CheckpointType>);
    fn set_last_synced_block(&mut self, last_synced_block: Option<BlockHeader>);
    async fn sync_block(&mut self, block_id: BlockId) -> SynchronizerResult;
    async fn sync_blocks(
        &mut self,
        initial_block_id: BlockId,
        final_block_id: BlockId,
    ) -> SynchronizerResult;
}

#[derive(Debug)]
pub struct SynchronizerBuilder {
    min_slots_per_thread: u32,
    checkpoint: Option<CheckpointType>,
    last_synced_block: Option<BlockHeader>,
}

pub struct Synchronizer {
    context: Box<dyn CommonContext>,
    min_slots_per_thread: u32,
    checkpoint: Option<CheckpointType>,
    last_synced_block: Option<BlockHeader>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CheckpointType {
    Lower,
    Upper,
}

impl Default for SynchronizerBuilder {
    fn default() -> Self {
        SynchronizerBuilder {
            min_slots_per_thread: 50,
            checkpoint: Some(CheckpointType::Upper),
            last_synced_block: None,
        }
    }
}

impl SynchronizerBuilder {
    pub fn new() -> Self {
        SynchronizerBuilder::default()
    }

    pub fn with_checkpoint(&mut self, checkpoint: Option<CheckpointType>) -> &mut Self {
        self.checkpoint = checkpoint;

        self
    }

    pub fn with_last_synced_block(&mut self, last_synced_block: BlockHeader) -> &mut Self {
        self.last_synced_block = Some(last_synced_block);

        self
    }

    pub fn build(&self, context: Box<dyn CommonContext>) -> Synchronizer {
        Synchronizer {
            context,
            min_slots_per_thread: self.min_slots_per_thread,
            checkpoint: self.checkpoint,
            last_synced_block: self.last_synced_block.clone(),
        }
    }
}

impl Synchronizer {
    async fn process_slots(
        &mut self,
        from_slot: u32,
        to_slot: u32,
    ) -> Result<(), SynchronizerError> {
        let is_reverse_sync = to_slot < from_slot;
        let unprocessed_slots = to_slot.abs_diff(from_slot);
        let min_slots_per_thread = std::cmp::min(unprocessed_slots, self.min_slots_per_thread);
        let slots_per_thread = std::cmp::max(
            min_slots_per_thread,
            unprocessed_slots / self.context.syncing_settings().concurrency,
        );
        let num_threads = std::cmp::max(1, unprocessed_slots / slots_per_thread);
        let remaining_slots = unprocessed_slots % num_threads;

        let mut handles: Vec<JoinHandle<Result<Option<BlockHeader>, SlotsProcessorError>>> = vec![];

        for i in 0..num_threads {
            let is_first_thread = i == 0;
            let is_last_thread = i == num_threads - 1;
            let thread_total_slots =
                slots_per_thread + if is_last_thread { remaining_slots } else { 0 };
            let thread_initial_slot = if is_reverse_sync {
                from_slot - i * slots_per_thread
            } else {
                from_slot + i * slots_per_thread
            };
            let thread_final_slot = if is_reverse_sync {
                thread_initial_slot - thread_total_slots
            } else {
                thread_initial_slot + thread_total_slots
            };

            let synchronizer_thread_span = tracing::debug_span!(
                parent:  &tracing::Span::current(),
                "thread",
                thread = i,
                chunk_initial_slot = thread_initial_slot,
                chunk_final_slot = thread_final_slot
            );

            let last_processed_block_header = if is_first_thread {
                self.last_synced_block.clone()
            } else {
                None
            };
            let mut slots_processor =
                SlotsProcessor::new(self.context.clone(), last_processed_block_header);

            let handle = tokio::spawn(
                async move {
                    slots_processor
                        .process_slots(thread_initial_slot, thread_final_slot)
                        .await?;

                    Ok(slots_processor.last_processed_block)
                }
                .instrument(synchronizer_thread_span)
                .in_current_span(),
            );

            handles.push(handle);
        }

        let handle_outputs = join_all(handles).await;

        let mut errors = vec![];
        let mut last_thread_block: Option<BlockHeader> = None;

        for handle in handle_outputs {
            match handle {
                Ok(thread_result) => match thread_result {
                    Ok(thread_block_header) => {
                        if let Some(block_header) = thread_block_header {
                            last_thread_block = Some(block_header);
                        }
                    }
                    Err(error) => errors.push(error),
                },
                Err(error) => {
                    let err = anyhow!("Synchronizer thread panicked: {:?}", error);

                    errors.push(err.into());
                }
            }
        }

        if !errors.is_empty() {
            return Err(SynchronizerError::FailedParallelSlotsProcessing {
                initial_slot: from_slot,
                final_slot: to_slot,
                chunk_errors: SlotsChunksErrors(errors),
            });
        }

        if let Some(last_thread_block) = last_thread_block {
            self.last_synced_block = Some(last_thread_block);
        }

        Ok(())
    }

    async fn process_slots_by_checkpoints(
        &mut self,
        initial_slot: u32,
        final_slot: u32,
    ) -> Result<(), SynchronizerError> {
        let is_reverse_sync = final_slot < initial_slot;
        let mut current_slot = initial_slot;
        let mut unprocessed_slots = final_slot.abs_diff(current_slot);

        if unprocessed_slots == 1 {
            info!(slot = initial_slot, "Syncing {unprocessed_slots} slot…");
        } else {
            info!(
                initial_slot,
                final_slot, "Syncing {unprocessed_slots} slots…"
            );
        }

        while unprocessed_slots > 0 {
            let checkpoint_size = self.context.syncing_settings().checkpoint_size;
            let slots_chunk = std::cmp::min(unprocessed_slots, checkpoint_size);
            let initial_chunk_slot = current_slot;
            let final_chunk_slot = if is_reverse_sync {
                current_slot - slots_chunk
            } else {
                current_slot + slots_chunk
            };

            let sync_slots_chunk_span = tracing::debug_span!(
                parent: &tracing::Span::current(),
                "checkpoint",
                checkpoint_initial_slot = initial_chunk_slot,
                checkpoint_final_slot = final_chunk_slot
            );

            self.process_slots(initial_chunk_slot, final_chunk_slot)
                .instrument(sync_slots_chunk_span)
                .await?;

            let last_slot = Some(if is_reverse_sync {
                final_chunk_slot + 1
            } else {
                final_chunk_slot - 1
            });

            let checkpointing_enabled = !self.context.syncing_settings().disable_checkpoints;

            if checkpointing_enabled {
                if let Some(checkpoint) = self.checkpoint {
                    let mut last_lower_synced_slot = None;
                    let mut last_upper_synced_slot = None;
                    let mut last_upper_synced_block_root = None;
                    let mut last_upper_synced_block_slot = None;

                    if checkpoint == CheckpointType::Lower {
                        last_lower_synced_slot = last_slot;
                    } else if checkpoint == CheckpointType::Upper {
                        last_upper_synced_slot = last_slot;
                        last_upper_synced_block_root =
                            self.last_synced_block.as_ref().map(|block| block.root);
                        last_upper_synced_block_slot =
                            self.last_synced_block.as_ref().map(|block| block.slot);
                    }

                    if let Err(error) = self
                        .context
                        .blobscan_client()
                        .update_sync_state(BlockchainSyncState {
                            last_finalized_block: None,
                            last_lower_synced_slot,
                            last_upper_synced_slot,
                            last_upper_synced_block_root,
                            last_upper_synced_block_slot,
                        })
                        .await
                    {
                        let new_synced_slot = match last_lower_synced_slot.or(last_upper_synced_slot) {
                                Some(slot) => slot,
                                None => return Err(SynchronizerError::Other(anyhow!(
                                    "Failed to get new last synced slot: last_lower_synced_slot and last_upper_synced_slot are both None"
                                )))
                            };

                        return Err(SynchronizerError::FailedSlotCheckpointSave {
                            slot: new_synced_slot,
                            error,
                        });
                    }

                    if unprocessed_slots >= checkpoint_size {
                        debug!(
                            new_last_lower_synced_slot = last_lower_synced_slot,
                            new_last_upper_synced_slot = last_upper_synced_slot,
                            "Checkpoint reached. Last synced slot saved…"
                        );
                    }
                }
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
}

#[async_trait]
impl CommonSynchronizer for Synchronizer {
    fn set_checkpoint(&mut self, checkpoint: Option<CheckpointType>) {
        self.checkpoint = checkpoint;
    }

    fn set_last_synced_block(&mut self, last_synced_block: Option<BlockHeader>) {
        self.last_synced_block = last_synced_block;
    }

    async fn sync_block(&mut self, block_id: BlockId) -> SynchronizerResult {
        let final_slot = block_id
            .resolve_to_slot(self.context.beacon_client())
            .await?;

        self.process_slots_by_checkpoints(final_slot, final_slot + 1)
            .await?;

        Ok(())
    }

    async fn sync_blocks(
        &mut self,
        initial_block_id: BlockId,
        final_block_id: BlockId,
    ) -> SynchronizerResult {
        let initial_slot = initial_block_id
            .resolve_to_slot(self.context.beacon_client())
            .await?;
        let mut final_slot = final_block_id
            .resolve_to_slot(self.context.beacon_client())
            .await?;

        if initial_slot == final_slot {
            return Ok(());
        }

        loop {
            self.process_slots_by_checkpoints(initial_slot, final_slot)
                .await?;

            let latest_final_slot = final_block_id
                .resolve_to_slot(self.context.beacon_client())
                .await?;

            if final_slot == latest_final_slot {
                return Ok(());
            }

            final_slot = latest_final_slot;
        }
    }
}
