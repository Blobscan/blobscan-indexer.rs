use ethers::types::{Address, H256, U256};
use serde::{Deserialize, Serialize};

use crate::db::types::IndexerMetadata;

#[derive(Serialize, Deserialize, Debug)]
pub struct BlockDocument {
    pub _id: String,
    pub hash: H256,
    pub parent_hash: H256,
    pub number: u64,
    pub timestamp: U256,
    pub slot: u32,
    pub transactions: Vec<H256>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TransactionDocument {
    pub _id: String,
    pub hash: H256,
    pub from: Address,
    pub to: Address,
    pub value: U256,
    pub block_hash: H256,
    pub block_number: u64,
    pub block_versioned_hashes: Vec<H256>,
    pub index: u32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BlobDocument {
    pub _id: String,
    pub hash: H256,
    pub tx_hash: H256,
    pub commitment: String, // TODO: change to H384
    pub index: u32,
    pub data: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IndexerMetadataDocument {
    pub _id: String,
    pub last_slot: u32,
}

impl From<IndexerMetadataDocument> for IndexerMetadata {
    fn from(metadata_document: IndexerMetadataDocument) -> Self {
        Self {
            last_slot: metadata_document.last_slot,
        }
    }
}
