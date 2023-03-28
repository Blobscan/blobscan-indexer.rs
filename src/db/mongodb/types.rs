use anyhow::{Context, Error, Result};
use ethers::types::{Address, Bytes, H256, U256};
use serde::{Deserialize, Serialize};

use crate::{
    db::utils::{build_blob_id, build_block_id, build_tx_id},
    types::{Blob, BlockData, IndexerMetadata, TransactionData},
};

#[derive(Serialize, Deserialize, Debug)]
pub struct BlockDocument {
    pub _id: String,
    pub hash: H256,
    pub number: u64,
    pub timestamp: u64,
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
    pub blob_versioned_hashes: Vec<H256>,
}
#[derive(Serialize, Deserialize, Debug)]
pub struct BlobDocument {
    pub _id: String,
    pub hash: H256,
    pub tx_hash: H256,
    pub commitment: String, // TODO: change to H384
    pub data: Bytes,
}

impl TryFrom<&BlockData<'_>> for BlockDocument {
    type Error = Error;

    fn try_from(block_data: &BlockData) -> Result<Self, Self::Error> {
        let block = block_data.block;

        let hash = block.hash.context("Block hash not found")?;
        let number = block.number.context("Block number not found")?.as_u64();

        Ok(Self {
            _id: build_block_id(&hash),
            hash,
            number,
            slot: block_data.slot,
            timestamp: block.timestamp.as_u64(),
            transactions: block_data.tx_to_versioned_hashes.keys().copied().collect(),
        })
    }
}

impl TryFrom<&TransactionData<'_>> for TransactionDocument {
    type Error = Error;

    fn try_from(tx_data: &TransactionData) -> Result<Self, Self::Error> {
        let tx = tx_data.tx;
        let to = tx.to.context("Transaction recipient not found")?;
        let block_hash = tx.block_hash.context("Transaction block hash not found")?;
        let block_number = tx
            .block_number
            .context("Transaction block number not found")?
            .as_u64();

        Ok(Self {
            _id: build_tx_id(&tx.hash),
            hash: tx.hash,
            from: tx.from,
            to,
            value: tx.value,
            block_hash,
            block_number,
            blob_versioned_hashes: tx_data.blob_versioned_hashes.clone(),
        })
    }
}

impl TryFrom<&Blob<'_>> for BlobDocument {
    type Error = Error;

    fn try_from(blob: &Blob) -> Result<Self, Self::Error> {
        Ok(Self {
            _id: build_blob_id(&blob.versioned_hash),
            hash: blob.versioned_hash,
            tx_hash: blob.tx_hash,
            commitment: blob.commitment.clone(),
            // Need to clone it as it's not possible to have a struct containing a reference field
            // as serde can't serialize it.
            data: blob.data.clone(),
        })
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IndexerMetadataDocument {
    pub _id: String,
    pub last_slot: u32,
}

impl TryFrom<IndexerMetadataDocument> for IndexerMetadata {
    type Error = Error;

    fn try_from(doc: IndexerMetadataDocument) -> Result<Self, Self::Error> {
        Ok(Self {
            last_slot: doc.last_slot,
        })
    }
}
