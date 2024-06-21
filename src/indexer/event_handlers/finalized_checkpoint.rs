use ethers::providers::Http as HttpProvider;
use tracing::info;

use crate::{
    clients::{
        beacon::types::{BlockId, FinalizedCheckpointEventData},
        blobscan::types::BlockchainSyncState,
        common::ClientError,
    },
    context::CommonContext,
    utils::web3::get_full_hash,
};

#[derive(Debug, thiserror::Error)]
pub enum FinalizedCheckpointEventHandlerError {
    #[error(transparent)]
    EventDeserializationFailure(#[from] serde_json::Error),
    #[error("failed to retrieve block {0}")]
    BlockRetrievalError(String, #[source] ClientError),
    #[error("block \"{0}\" not found")]
    BlockNotFound(String),
    #[error("failed to update last finalized block")]
    BlobscanFinalizedBlockUpdateFailure(#[source] ClientError),
}

pub struct FinalizedCheckpointHandler<T> {
    context: Box<dyn CommonContext<T>>,
}

impl FinalizedCheckpointHandler<HttpProvider> {
    pub fn new(context: Box<dyn CommonContext<HttpProvider>>) -> Self {
        FinalizedCheckpointHandler { context }
    }

    pub async fn handle(
        &self,
        event_data: String,
    ) -> Result<(), FinalizedCheckpointEventHandlerError> {
        let finalized_checkpoint_data =
            serde_json::from_str::<FinalizedCheckpointEventData>(&event_data)?;
        let block_hash = finalized_checkpoint_data.block;
        let full_block_hash = get_full_hash(&block_hash);
        let last_finalized_block_number = match self
            .context
            .beacon_client()
            .get_block(&BlockId::Hash(block_hash))
            .await
            .map_err(|err| {
                FinalizedCheckpointEventHandlerError::BlockRetrievalError(
                    full_block_hash.clone(),
                    err,
                )
            })? {
            Some(block) => match block.message.body.execution_payload {
                Some(execution_payload) => execution_payload.block_number,
                None => {
                    return Err(FinalizedCheckpointEventHandlerError::BlockNotFound(
                        full_block_hash,
                    ))
                }
            },
            None => {
                return Err(FinalizedCheckpointEventHandlerError::BlockNotFound(
                    full_block_hash,
                ))
            }
        };

        self.context
            .blobscan_client()
            .update_sync_state(BlockchainSyncState {
                last_lower_synced_slot: None,
                last_upper_synced_slot: None,
                last_finalized_block: Some(last_finalized_block_number),
            })
            .await
            .map_err(FinalizedCheckpointEventHandlerError::BlobscanFinalizedBlockUpdateFailure)?;

        info!(
            finalized_execution_block = last_finalized_block_number,
            "Finalized checkpoint event received. Updated last finalized block number"
        );

        Ok(())
    }
}
