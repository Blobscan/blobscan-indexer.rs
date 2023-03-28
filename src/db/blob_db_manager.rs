use anyhow::Result;
use async_trait::async_trait;

use crate::types::{Blob, BlockData, IndexerMetadata, TransactionData};

#[async_trait]
pub trait DBManager {
    type Options;

    async fn new(connection_uri: &str, db_name: &str) -> Result<Self>
    where
        Self: Sized;

    async fn commit_transaction(&self, options: Option<&mut Self::Options>) -> Result<()>;

    async fn insert_block(
        &self,
        block: &BlockData,
        options: Option<&mut Self::Options>,
    ) -> Result<()>;

    async fn insert_blob(&self, blob: &Blob, options: Option<&mut Self::Options>) -> Result<()>;

    async fn insert_tx(
        &self,
        tx: &TransactionData,
        options: Option<&mut Self::Options>,
    ) -> Result<()>;

    async fn start_transaction(&self, options: Option<&mut Self::Options>) -> Result<()>;

    async fn update_last_slot(&self, slot: u32, options: Option<&mut Self::Options>) -> Result<()>;

    async fn read_metadata(
        &self,
        options: Option<&mut Self::Options>,
    ) -> Result<Option<IndexerMetadata>>;
}
