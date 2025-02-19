use tokio::sync::mpsc::error::SendError;

use crate::{clients::common::ClientError, synchronizer::error::SynchronizerError};

use super::{
    event_handlers::{
        finalized_checkpoint::FinalizedCheckpointEventHandlerError, head::HeadEventHandlerError,
    },
    types::IndexerTaskMessage,
};

#[derive(Debug, thiserror::Error)]
pub enum IndexerError {
    #[error("failed to create indexer")]
    CreationFailure(#[source] anyhow::Error),
    #[error(transparent)]
    SyncingTaskError(#[from] IndexingError),
    #[error("failed to retrieve blobscan's sync state")]
    BlobscanSyncStateRetrievalError(#[from] ClientError),
    #[error("failed to send syncing task message")]
    SyncingTaskMessageSendFailure(#[from] SendError<IndexerTaskMessage>),
}

#[derive(Debug, thiserror::Error)]
pub enum IndexingError {
    #[error(transparent)]
    HistoricalIndexingFailure(#[from] HistoricalIndexingError),
    #[error(transparent)]
    LiveIndexingError(#[from] LiveIndexingError),
}

#[derive(Debug, thiserror::Error)]
pub enum HistoricalIndexingError {
    #[error(transparent)]
    SynchronizerError(#[from] SynchronizerError),
}

#[derive(Debug, thiserror::Error)]
pub enum LiveIndexingError {
    #[error("an error occurred while receiving beacon events")]
    BeaconEventsConnectionFailure(#[from] reqwest_eventsource::Error),
    #[error("failed to subscribe to beacon events")]
    BeaconEventsSubscriptionError(#[source] ClientError),
    #[error("unexpected event \"{0}\" received")]
    UnexpectedBeaconEvent(String),
    #[error("failed to handle beacon event")]
    BeaconEventHandlingError(#[from] EventHandlerError),
}

#[derive(Debug, thiserror::Error)]
pub enum EventHandlerError {
    #[error(transparent)]
    HeadEventHandlerError(#[from] HeadEventHandlerError),
    #[error(transparent)]
    FinalizedCheckpointHandlerError(#[from] FinalizedCheckpointEventHandlerError),
}

impl From<HeadEventHandlerError> for LiveIndexingError {
    fn from(err: HeadEventHandlerError) -> Self {
        LiveIndexingError::BeaconEventHandlingError(EventHandlerError::HeadEventHandlerError(err))
    }
}

impl From<FinalizedCheckpointEventHandlerError> for LiveIndexingError {
    fn from(err: FinalizedCheckpointEventHandlerError) -> Self {
        LiveIndexingError::BeaconEventHandlingError(
            EventHandlerError::FinalizedCheckpointHandlerError(err),
        )
    }
}
