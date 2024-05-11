use super::error::{IndexerError, IndexingTaskError};

pub type IndexerResult<T> = Result<T, IndexerError>;

pub enum IndexerTaskResult {
    Done(String),
    Error(IndexingTaskError),
}
