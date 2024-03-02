use crate::{clients::common::ClientError, synchronizer::error::SynchronizerError};

#[derive(Debug, thiserror::Error)]
pub enum IndexerError {
    #[error(transparent)]
    ReqwestEventSourceError(#[from] reqwest_eventsource::Error),
    #[error(transparent)]
    ClientError(#[from] ClientError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
    #[error(transparent)]
    SynchronizerError(#[from] SynchronizerError),
    #[error("{0}")]
    SerdeError(#[from] serde_json::Error),
    #[error("Unexpected event \"{event}\" received")]
    UnexpectedEvent { event: String },
}

#[derive(Debug, thiserror::Error)]
pub enum IndexingTaskError {
    #[error("Indexing task {task_name} failed: {error}")]
    FailedIndexingTask {
        task_name: String,
        error: IndexerError,
    },
}

impl From<IndexingTaskError> for IndexerError {
    fn from(error: IndexingTaskError) -> Self {
        match error {
            IndexingTaskError::FailedIndexingTask {
                task_name: _,
                error,
            } => error,
        }
    }
}
