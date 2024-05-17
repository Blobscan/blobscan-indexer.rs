use tokio::sync::mpsc::error::SendError;

use crate::{clients::common::ClientError, synchronizer::error::SynchronizerError};

use super::types::IndexerTaskMessage;

#[derive(Debug, thiserror::Error)]
pub enum IndexerError {
    #[error("failed to create indexer")]
    CreationFailure(#[source] anyhow::Error),
    #[error(transparent)]
    SyncingTaskError(#[from] SyncingTaskError),
    #[error("failed to retrieve blobscan's sync state")]
    BlobscanSyncStateRetrievalError(#[source] ClientError),
    #[error("sync task message send failure")]
    SyncingTaskMessageSendFailure(#[from] SendError<IndexerTaskMessage>),
}

#[derive(Debug, thiserror::Error)]
pub enum SyncingTaskError {
    #[error("an error ocurred while syncing historical data")]
    HistoricalSyncingTaskError(#[from] HistoricalSyncingError),
    #[error("an error occurred while syncing realtime data")]
    RealtimeSyncingTaskError(#[from] RealtimeSyncingError),
}

#[derive(Debug, thiserror::Error)]
pub enum HistoricalSyncingError {
    #[error(transparent)]
    SynchronizerError(#[from] SynchronizerError),
}

#[derive(Debug, thiserror::Error)]
pub enum RealtimeSyncingError {
    #[error("an error ocurred while receiving beacon events")]
    BeaconEventsConnectionFailure(#[from] reqwest_eventsource::Error),
    #[error("failed to subscribe to beacon events")]
    BeaconEventsSubscriptionError(#[source] ClientError),
    #[error("unexpected event \"{0}\" received")]
    UnexpectedBeaconEvent(String),
    #[error(transparent)]
    BeaconEventProcessingError(#[from] BeaconEventError),
}

#[derive(Debug, thiserror::Error)]
pub enum BeaconEventError {
    #[error("failed to handle \"chain_reorged\" event")]
    ChainReorged(#[from] ChainReorgedEventHandlingError),
    #[error("failed to handle \"head\" event")]
    HeadBlock(#[from] HeadBlockEventHandlingError),
    #[error("failed to handle \"finalized_checkpoint\" event")]
    FinalizedCheckpoint(#[from] FinalizedBlockEventHandlingError),
}

#[derive(Debug, thiserror::Error)]
pub enum FinalizedBlockEventHandlingError {
    #[error(transparent)]
    EventDeserializationFailure(#[from] serde_json::Error),
    #[error("failed to retrieve finalized block {0}")]
    BlockRetrievalError(String, #[source] ClientError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
    #[error("failed to update blobscan's last finalized block")]
    BlobscanSyncStateUpdateError(#[source] ClientError),
}

#[derive(Debug, thiserror::Error)]
pub enum ChainReorgedEventHandlingError {
    #[error(transparent)]
    EventDeserializationFailure(#[from] serde_json::Error),
    #[error("failed to retrieve reorged block {0}")]
    BlockRetrievalError(String, #[source] ClientError),
    #[error("failed to handle reorged of depth {0} starting at block {1}")]
    ReorgedHandlingFailure(u32, String, #[source] ClientError),
}

#[derive(Debug, thiserror::Error)]
pub enum HeadBlockEventHandlingError {
    #[error(transparent)]
    EventDeserializationFailure(#[from] serde_json::Error),
    #[error(transparent)]
    SynchronizerError(#[from] SynchronizerError),
}
