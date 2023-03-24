use async_trait::async_trait;

use crate::types::{Blob, BlockData, IndexerMetadata, StdError, TransactionData};

#[async_trait]
pub trait DBManager {
    type Options;

    async fn new(connection_uri: &String, db_name: &String) -> Result<Self, StdError>
    where
        Self: Sized;

    async fn commit_transaction(&mut self, options: Option<Self::Options>) -> Result<(), StdError>;

    async fn insert_block(
        &mut self,
        block: &BlockData,
        options: Option<Self::Options>,
    ) -> Result<(), StdError>;

    async fn insert_blob(
        &mut self,
        blob: &Blob,
        options: Option<Self::Options>,
    ) -> Result<(), StdError>;

    async fn insert_tx(
        &mut self,
        tx: &TransactionData,
        options: Option<Self::Options>,
    ) -> Result<(), StdError>;

    async fn start_transaction(&mut self) -> Result<(), StdError>;

    async fn update_last_slot(
        &mut self,
        slot: u32,
        options: Option<Self::Options>,
    ) -> Result<(), StdError>;

    async fn read_metadata(
        &mut self,
        options: Option<Self::Options>,
    ) -> Result<Option<IndexerMetadata>, StdError>;
}
