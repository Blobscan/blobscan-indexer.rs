use async_trait::async_trait;

use crate::types::{Blob, BlockData, IndexerMetadata, StdError, TransactionData};

#[async_trait]
pub trait DBManager {
    type Options;

    async fn new(connection_uri: &str, db_name: &str) -> Result<Self, StdError>
    where
        Self: Sized;

    async fn commit_transaction(&self, options: Option<&mut Self::Options>)
        -> Result<(), StdError>;

    async fn insert_block(
        &self,
        block: &BlockData,
        options: Option<&mut Self::Options>,
    ) -> Result<(), StdError>;

    async fn insert_blob(
        &self,
        blob: &Blob,
        options: Option<&mut Self::Options>,
    ) -> Result<(), StdError>;

    async fn insert_tx(
        &self,
        tx: &TransactionData,
        options: Option<&mut Self::Options>,
    ) -> Result<(), StdError>;

    async fn start_transaction(&self, options: Option<&mut Self::Options>) -> Result<(), StdError>;

    async fn update_last_slot(
        &self,
        slot: u32,
        options: Option<&mut Self::Options>,
    ) -> Result<(), StdError>;

    async fn read_metadata(
        &self,
        options: Option<&mut Self::Options>,
    ) -> Result<Option<IndexerMetadata>, StdError>;
}
