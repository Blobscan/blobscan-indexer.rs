use crate::{
    clients::{beacon::types::BlockIdResolutionError, common::ClientError},
    indexer::tasks::sse_indexing::SSEIndexingError,
    synchronizer::error::SynchronizerError,
};

#[derive(Debug, thiserror::Error)]
pub enum IndexerError {
    #[error("failed to create indexer")]
    CreationFailure(#[source] anyhow::Error),
    #[error("failed to retrieve blobscan's sync state")]
    IndexerStateRetrievalError(#[from] ClientError),
    #[error("task \"{task_name}\" failed: {error}")]
    IndexingTaskError {
        task_name: String,
        error: IndexerTaskError,
    },
    #[error(transparent)]
    SynchronizerError(#[from] SynchronizerError),
    #[error(transparent)]
    BlockIdResolutionFailed(#[from] BlockIdResolutionError),
}

#[derive(Debug, thiserror::Error)]
pub enum IndexerTaskError {
    #[error(transparent)]
    SSEIndexingError(#[from] SSEIndexingError),
    #[error(transparent)]
    SynchronizerError(#[from] SynchronizerError),
}
