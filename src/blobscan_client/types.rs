use anyhow::{Context, Result};
use ethers::types::{
    Address, Block as EthersBlock, Bytes, Transaction as EthersTransaction, H256, U256, U64,
};
use serde::{Deserialize, Serialize};

use crate::{beacon_client::types::BlobData, utils::web3::calculate_versioned_hash};

#[derive(Serialize, Deserialize, Debug)]
pub struct BlockEntity {
    pub number: U64,
    pub hash: H256,
    pub timestamp: U256,
    pub slot: u32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TransactionEntity {
    pub hash: H256,
    pub from: Address,
    pub to: Address,
    #[serde(rename = "blockNumber")]
    pub block_number: U64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BlobEntity {
    #[serde(rename = "versionedHash")]
    pub versioned_hash: H256,
    pub commitment: String,
    pub data: Bytes,
    #[serde(rename = "txHash")]
    pub tx_hash: H256,
    pub index: u32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SlotResponse {
    pub slot: u32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SlotRequest {
    pub slot: u32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IndexRequest {
    pub block: BlockEntity,
    pub transactions: Vec<TransactionEntity>,
    pub blobs: Vec<BlobEntity>,
}

#[derive(Debug, thiserror::Error)]
pub enum BlobscanClientError {
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),

    #[error("Blobscan client error: {0}")]
    BlobscanClientError(String),

    #[error(transparent)]
    JWTError(#[from] anyhow::Error),
}

pub type BlobscanClientResult<T> = Result<T, BlobscanClientError>;

impl<'a> TryFrom<(&'a EthersBlock<EthersTransaction>, u32)> for BlockEntity {
    type Error = anyhow::Error;

    fn try_from(
        (ethers_block, slot): (&'a EthersBlock<EthersTransaction>, u32),
    ) -> Result<Self, Self::Error> {
        let number = ethers_block
            .number
            .with_context(|| format!("Missing block number field in execution block"))?;

        Ok(Self {
            number,
            hash: ethers_block
                .hash
                .with_context(|| format!("Missing block hash field in execution block {number}"))?,
            timestamp: ethers_block.timestamp,
            slot,
        })
    }
}

impl<'a> TryFrom<(&'a EthersTransaction, &'a EthersBlock<EthersTransaction>)>
    for TransactionEntity
{
    type Error = anyhow::Error;

    fn try_from(
        (ethers_tx, ethers_block): (&'a EthersTransaction, &'a EthersBlock<EthersTransaction>),
    ) -> Result<Self, Self::Error> {
        let hash = ethers_tx.hash;

        Ok(Self {
            block_number: ethers_block
                .number
                .with_context(|| format!("Missing block number field in execution block"))?,
            hash,
            from: ethers_tx.from,
            to: ethers_tx
                .to
                .with_context(|| format!("Missing to field in transaction {hash}"))?,
        })
    }
}

impl<'a> TryFrom<(&'a BlobData, u32, H256)> for BlobEntity {
    type Error = anyhow::Error;

    fn try_from(
        (blob_data, index, tx_hash): (&'a BlobData, u32, H256),
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            tx_hash,
            index,
            commitment: blob_data.kzg_commitment.clone(),
            data: blob_data.blob.clone(),
            versioned_hash: calculate_versioned_hash(&blob_data.kzg_commitment)?,
        })
    }
}

impl<'a> From<(&'a BlobData, &'a H256, usize, &'a H256)> for BlobEntity {
    fn from(
        (blob_data, versioned_hash, index, tx_hash): (&'a BlobData, &'a H256, usize, &'a H256),
    ) -> Self {
        Self {
            tx_hash: tx_hash.clone(),
            index: index as u32,
            commitment: blob_data.kzg_commitment.clone(),
            data: blob_data.blob.clone(),
            versioned_hash: versioned_hash.clone(),
        }
    }
}
