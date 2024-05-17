use super::error::{IndexerError, SyncingTaskError};

pub type IndexerResult<T> = Result<T, IndexerError>;

pub enum IndexerTaskMessage {
    Done,
    Error(SyncingTaskError),
}
