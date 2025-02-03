use std::fmt::Debug;

use alloy::transports::http::ReqwestTransport;
use anyhow::anyhow;
use async_trait::async_trait;
use futures::future::join_all;
use tokio::task::JoinHandle;
use tracing::{debug, info, Instrument};

#[cfg(test)]
use mockall::automock;

use crate::{
    clients::{
        beacon::types::{BlockHeader, BlockId},
        blobscan::types::BlockchainSyncState,
        common::ClientError,
    },
    context::CommonContext,
    slots_processor::{error::SlotsProcessorError, SlotsProcessor},
};

use self::error::{SlotsChunksErrors, SynchronizerError};

pub mod error;

#[async_trait]
#[cfg_attr(test, automock)]
pub trait CommonSynchronizer: Send + Sync {
    fn clear_last_synced_block(&mut self);
    fn get_last_synced_block(&self) -> Option<BlockHeader>;
    async fn sync_block(&mut self, block_id: BlockId) -> Result<(), SynchronizerError>;
    async fn sync_blocks(
        &mut self,
        initial_block_id: BlockId,
        final_block_id: BlockId,
    ) -> Result<(), SynchronizerError>;
}

#[derive(Debug)]
pub struct SynchronizerBuilder {
    num_threads: u32,
    min_slots_per_thread: u32,
    slots_checkpoint: u32,
    checkpoint_type: CheckpointType,
    last_synced_block: Option<BlockHeader>,
}

pub struct Synchronizer<T> {
    context: Box<dyn CommonContext<T>>,
    num_threads: u32,
    min_slots_per_thread: u32,
    slots_checkpoint: u32,
    checkpoint_type: CheckpointType,
    last_synced_block: Option<BlockHeader>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CheckpointType {
    Disabled,
    Lower,
    Upper,
}

impl Default for SynchronizerBuilder {
    fn default() -> Self {
        SynchronizerBuilder {
            num_threads: 1,
            min_slots_per_thread: 50,
            slots_checkpoint: 1000,
            checkpoint_type: CheckpointType::Upper,
            last_synced_block: None,
        }
    }
}

impl SynchronizerBuilder {
    pub fn new() -> Self {
        SynchronizerBuilder::default()
    }

    pub fn with_checkpoint_type(&mut self, checkpoint_type: CheckpointType) -> &mut Self {
        self.checkpoint_type = checkpoint_type;

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

    pub fn with_last_synced_block(&mut self, last_synced_block: BlockHeader) -> &mut Self {
        self.last_synced_block = Some(last_synced_block);

        self
    }

    pub fn build(
        &self,
        context: Box<dyn CommonContext<ReqwestTransport>>,
    ) -> Synchronizer<ReqwestTransport> {
        Synchronizer {
            context,
            num_threads: self.num_threads,
            min_slots_per_thread: self.min_slots_per_thread,
            slots_checkpoint: self.slots_checkpoint,
            checkpoint_type: self.checkpoint_type,
            last_synced_block: self.last_synced_block.clone(),
        }
    }
}

impl Synchronizer<ReqwestTransport> {
    async fn process_slots(
        &mut self,
        from_slot: u32,
        to_slot: u32,
    ) -> Result<(), SynchronizerError> {
        let is_reverse_sync = to_slot < from_slot;
        let unprocessed_slots = to_slot.abs_diff(from_slot);
        let min_slots_per_thread = std::cmp::min(unprocessed_slots, self.min_slots_per_thread);
        let slots_per_thread =
            std::cmp::max(min_slots_per_thread, unprocessed_slots / self.num_threads);
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

        if errors.is_empty() {
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
            let slots_chunk = std::cmp::min(unprocessed_slots, self.slots_checkpoint);
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

            if self.checkpoint_type != CheckpointType::Disabled {
                let last_lower_synced_slot = if self.checkpoint_type == CheckpointType::Lower {
                    last_slot
                } else {
                    None
                };
                let last_upper_synced_slot = if self.checkpoint_type == CheckpointType::Upper {
                    last_slot
                } else {
                    None
                };

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

                if unprocessed_slots >= self.slots_checkpoint {
                    debug!(
                        new_last_lower_synced_slot = last_lower_synced_slot,
                        new_last_upper_synced_slot = last_upper_synced_slot,
                        "Checkpoint reached. Last synced slot saved…"
                    );
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

    async fn resolve_to_slot(&self, block_id: BlockId) -> Result<u32, SynchronizerError> {
        let beacon_client = self.context.beacon_client();

        let resolved_block_id: Result<u32, ClientError> = match block_id {
            BlockId::Slot(slot) => Ok(slot),
            _ => match beacon_client.get_block_header(block_id.clone()).await {
                Ok(None) => {
                    let err = anyhow!("Block ID {} not found", block_id);

                    Err(err.into())
                }
                Ok(Some(block_header)) => Ok(block_header.slot),
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

    pub fn clear_last_synced_block(&mut self) {
        self.last_synced_block = None;
    }
}

#[async_trait]
impl CommonSynchronizer for Synchronizer<ReqwestTransport> {
    fn clear_last_synced_block(&mut self) {
        self.clear_last_synced_block();
    }

    fn get_last_synced_block(&self) -> Option<BlockHeader> {
        self.last_synced_block.clone()
    }

    async fn sync_block(&mut self, block_id: BlockId) -> Result<(), SynchronizerError> {
        let final_slot = self.resolve_to_slot(block_id.clone()).await?;

        self.process_slots_by_checkpoints(final_slot, final_slot + 1)
            .await?;

        Ok(())
    }

    async fn sync_blocks(
        &mut self,
        initial_block_id: BlockId,
        final_block_id: BlockId,
    ) -> Result<(), SynchronizerError> {
        let initial_slot = self.resolve_to_slot(initial_block_id).await?;
        let mut final_slot = self.resolve_to_slot(final_block_id.clone()).await?;

        if initial_slot == final_slot {
            return Ok(());
        }

        loop {
            self.process_slots_by_checkpoints(initial_slot, final_slot)
                .await?;

            let latest_final_slot = self.resolve_to_slot(final_block_id.clone()).await?;

            if final_slot == latest_final_slot {
                return Ok(());
            }

            final_slot = latest_final_slot;
        }
    }
}
