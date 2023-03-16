use async_trait::async_trait;
use ethers::types::{Block, Bytes, Transaction, H256};
use std::error::Error;

pub struct Blob {
    pub data: Bytes,
    pub commitment: String,
    pub versioned_hash: H256,
    pub index: u32,
}

#[async_trait]
pub trait DBManager {
    type Options;

    async fn start_transaction(&mut self) -> Result<(), Box<dyn Error>>;

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

    async fn commit_transaction(&mut self) -> Result<(), Box<dyn Error>>;
}
