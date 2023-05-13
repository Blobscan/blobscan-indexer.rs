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
#[serde(rename_all = "camelCase")]
pub struct TransactionEntity {
    pub hash: H256,
    pub from: Address,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<Address>,
    pub block_number: U64,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BlobEntity {
    pub versioned_hash: H256,
    pub commitment: String,
    pub data: Bytes,
    pub tx_hash: H256,
    pub index: u32,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FailedSlotsChunkEntity {
    pub initial_slot: u32,
    pub final_slot: u32,
}

#[derive(Serialize, Debug)]
pub struct FailedSlotsChunksRequest {
    pub chunks: Vec<FailedSlotsChunkEntity>,
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

impl From<(u32, u32)> for FailedSlotsChunkEntity {
    fn from((initial_slot, final_slot): (u32, u32)) -> Self {
        Self {
            initial_slot,
            final_slot,
        }
    }
}

impl<'a> TryFrom<(&'a EthersBlock<EthersTransaction>, u32)> for BlockEntity {
    type Error = anyhow::Error;

    fn try_from(
        (ethers_block, slot): (&'a EthersBlock<EthersTransaction>, u32),
    ) -> Result<Self, Self::Error> {
        let number = ethers_block
            .number
            .with_context(|| "Missing block number field in execution block".to_string())?;

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
                .with_context(|| "Missing block number field in execution block".to_string())?,
            hash,
            from: ethers_tx.from,
            to: ethers_tx.to,
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
            tx_hash: *tx_hash,
            index: index as u32,
            commitment: blob_data.kzg_commitment.clone(),
            data: blob_data.blob.clone(),
            versioned_hash: *versioned_hash,
        }
    }
}
