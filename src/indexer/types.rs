use super::error::{IndexerError, IndexingTaskError};

pub type IndexerResult<T> = Result<T, IndexerError>;

pub type IndexerTaskResult = Result<(), IndexingTaskError>;
