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
