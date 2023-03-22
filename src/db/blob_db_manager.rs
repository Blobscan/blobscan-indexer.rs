use async_trait::async_trait;
use ethers::types::{Block, Transaction, H256};

use crate::{db::types::Blob, types::StdError};

use super::types::IndexerMetadata;
#[async_trait]
pub trait DBManager {
    type Options;

    async fn new(connection_uri: &String, db_name: &String) -> Result<Self, StdError>
    where
        Self: Sized;

    async fn commit_transaction(&mut self, options: Option<Self::Options>) -> Result<(), StdError>;

    async fn insert_block(
        &mut self,
        execution_block: &Block<H256>,
        blob_txs: &Vec<Transaction>,
        slot: u32,
        options: Option<Self::Options>,
    ) -> Result<(), StdError>;

    async fn insert_blob(
        &mut self,
        blob: &Blob,
        tx_hash: H256,
        options: Option<Self::Options>,
    ) -> Result<(), StdError>;

    async fn insert_tx(
        &mut self,
        tx: &Transaction,
        index: u32,
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
