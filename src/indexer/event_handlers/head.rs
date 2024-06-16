use std::cmp;

use ethers::{providers::Http as HttpProvider, types::H256};
use tracing::info;

use crate::{
    clients::{
        beacon::types::{BlockHeader, BlockId, HeadEventData},
        blobscan::types::BlockchainSyncState,
        common::ClientError,
    },
    context::CommonContext,
    synchronizer::{error::SynchronizerError, CommonSynchronizer},
};

#[derive(Debug, thiserror::Error)]
pub enum HeadEventHandlerError {
    #[error(transparent)]
    EventDeserializationFailure(#[from] serde_json::Error),
    #[error("failed to retrieve header for block \"{0}\"")]
    BlockHeaderRetrievalError(BlockId, #[source] ClientError),
    #[error("header for block \"{0}\" not found")]
    BlockHeaderNotFound(BlockId),
    #[error("failed to index head block")]
    BlockSyncedError(#[from] SynchronizerError),
    #[error("failed to handle reorged slots")]
    BlobscanReorgedSlotsFailure(#[source] ClientError),
    #[error("failed to update blobscan's sync state")]
    BlobscanSyncStateUpdateError(#[source] ClientError),
}

pub struct HeadEventHandler<T> {
    context: Box<dyn CommonContext<T>>,
    synchronizer: Box<dyn CommonSynchronizer>,
    start_block_id: BlockId,
    last_block_hash: Option<H256>,
}

impl HeadEventHandler<HttpProvider> {
    pub fn new(
        context: Box<dyn CommonContext<HttpProvider>>,
        synchronizer: Box<dyn CommonSynchronizer>,
        start_block_id: BlockId,
    ) -> Self {
        HeadEventHandler {
            context,
            synchronizer,
            start_block_id,
            last_block_hash: None,
        }
    }

    pub async fn handle(&mut self, event_data: String) -> Result<(), HeadEventHandlerError> {
        let head_block_data = serde_json::from_str::<HeadEventData>(&event_data)?;

        let head_block_slot = head_block_data.slot;
        let head_block_hash = head_block_data.block;

        let head_block_id = BlockId::Slot(head_block_data.slot);
        let initial_block_id = if self.last_block_hash.is_none() {
            self.start_block_id.clone()
        } else {
            head_block_id.clone()
        };

        let head_block_header = self.get_block_header(&head_block_id).await?.header;

        if let Some(last_block_hash) = self.last_block_hash {
            if last_block_hash != head_block_header.message.parent_root {
                let parent_block_header = self
                    .get_block_header(&BlockId::Hash(head_block_header.message.parent_root))
                    .await?
                    .header;
                let parent_block_slot = parent_block_header.message.slot;
                let reorg_start_slot = parent_block_slot + 1;
                let reorg_final_slot = head_block_slot;
                let reorged_slots = (reorg_start_slot..reorg_final_slot).collect::<Vec<u32>>();

                let result: Result<(), HeadEventHandlerError> = async {
                    let total_updated_slots = self.context
                        .blobscan_client()
                        .handle_reorged_slots(reorged_slots.as_slice())
                        .await
                        .map_err(HeadEventHandlerError::BlobscanReorgedSlotsFailure)?;


                    info!(slot=head_block_slot, "Reorganization detected. Found the following reorged slots: {:#?}. Total slots marked as reorged: {total_updated_slots}", reorged_slots);

                    // Re-index parent block as it may be mark as reorged and not indexed
                    self.synchronizer
                        .run(
                            &BlockId::Slot(parent_block_slot),
                            &BlockId::Slot(parent_block_slot + 1),
                        )
                        .await?;

                    Ok(())
                }
                .await;

                if let Err(err) = result {
                    // If an error occurred while handling the reorg try to update the latest synced slot to the last known slot before the reorg
                    self.context
                        .blobscan_client()
                        .update_sync_state(BlockchainSyncState {
                            last_finalized_block: None,
                            last_lower_synced_slot: None,
                            last_upper_synced_slot: Some(cmp::max(parent_block_slot - 1, 0)),
                        })
                        .await
                        .map_err(HeadEventHandlerError::BlobscanSyncStateUpdateError)?;

                    return Err(err);
                }
            }
        }

        self.synchronizer
            .run(&initial_block_id, &BlockId::Slot(head_block_slot + 1))
            .await?;

        self.last_block_hash = Some(head_block_hash);

        Ok(())
    }

    async fn get_block_header(
        &self,
        block_id: &BlockId,
    ) -> Result<BlockHeader, HeadEventHandlerError> {
        match self
            .context
            .beacon_client()
            .get_block_header(block_id)
            .await
            .map_err(|err| {
                HeadEventHandlerError::BlockHeaderRetrievalError(block_id.clone(), err)
            })? {
            Some(block) => Ok(block),
            None => Err(HeadEventHandlerError::BlockHeaderNotFound(block_id.clone())),
        }
    }
}
