use super::error::{IndexerError, IndexingError};

pub type IndexerResult<T> = Result<T, IndexerError>;

pub enum IndexerTaskMessage {
    Done,
    Error(IndexingError),
}
