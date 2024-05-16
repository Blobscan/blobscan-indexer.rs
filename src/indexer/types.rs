use super::error::IndexerError;

pub type IndexerResult<T> = Result<T, IndexerError>;

pub enum IndexerTaskMessage {
    Done,
    Error(IndexerError),
}
