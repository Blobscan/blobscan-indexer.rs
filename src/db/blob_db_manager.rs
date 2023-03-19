use async_trait::async_trait;
use ethers::types::{Block, Transaction, H256};
use std::error::Error;

use crate::db::types::Blob;

use super::types::IndexerMetadata;
#[async_trait]
pub trait DBManager {
    type Options;

    async fn new(connection_uri: &String, db_name: &String) -> Result<Self, Box<dyn Error>>
    where
        Self: Sized;

    async fn commit_transaction(
        &mut self,
        options: Option<Self::Options>,
    ) -> Result<(), Box<dyn Error>>;

    async fn insert_block(
        &mut self,
        execution_block: &Block<H256>,
        blob_txs: &Vec<Transaction>,
        slot: u32,
        options: Option<Self::Options>,
    ) -> Result<(), Box<dyn Error>>;

    async fn insert_blob(
        &mut self,
        blob: &Blob,
        tx_hash: H256,
        options: Option<Self::Options>,
    ) -> Result<(), Box<dyn Error>>;

    async fn insert_tx(
        &mut self,
        tx: &Transaction,
        index: u32,
        options: Option<Self::Options>,
    ) -> Result<(), Box<dyn Error>>;

    async fn start_transaction(&mut self) -> Result<(), Box<dyn Error>>;

    async fn update_last_slot(
        &mut self,
        slot: u32,
        options: Option<Self::Options>,
    ) -> Result<(), Box<dyn Error>>;

    async fn read_metadata(
        &mut self,
        options: Option<Self::Options>,
    ) -> Result<Option<IndexerMetadata>, Box<dyn Error>>;
}
