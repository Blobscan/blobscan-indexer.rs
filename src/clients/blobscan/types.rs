use core::fmt;

use alloy::consensus::Transaction as Consensus;
use alloy::primitives::{Address, BlockNumber, BlockTimestamp, Bytes, TxIndex, B256, U256};
use alloy::rpc::types::{Block as ExecutionBlock, Transaction as ExecutionTransaction};
use anyhow::{Context, Result};

use serde::{Deserialize, Serialize};

use crate::{clients::beacon::types::Blob as BeaconBlob, utils::web3::calculate_versioned_hash};

#[derive(Serialize, Deserialize, Debug)]
pub struct BlobscanBlock {
    pub hash: B256,
    pub number: u32,
    pub slot: u32,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    pub number: BlockNumber,
    pub hash: B256,
    pub timestamp: BlockTimestamp,
    pub slot: u32,
    pub blob_gas_used: U256,
    pub excess_blob_gas: U256,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Transaction {
    pub hash: B256,
    pub from: Address,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<Address>,
    pub block_number: BlockNumber,
    pub index: TxIndex,
    pub gas_price: U256,
    pub max_fee_per_blob_gas: U256,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Blob {
    pub versioned_hash: B256,
    pub commitment: String,
    pub proof: String,
    pub data: Bytes,
    pub tx_hash: B256,
    pub index: u32,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FailedSlotsChunk {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    pub initial_slot: u32,
    pub final_slot: u32,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BlockchainSyncStateRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_lower_synced_slot: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_upper_synced_slot: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_finalized_block: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_upper_synced_block_root: Option<B256>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_upper_synced_block_slot: Option<u32>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BlockchainSyncStateResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_lower_synced_slot: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_upper_synced_slot: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_upper_synced_block_root: Option<B256>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_upper_synced_block_slot: Option<u32>,
}

#[derive(Debug, PartialEq)]
pub struct BlockchainSyncState {
    pub last_finalized_block: Option<u32>,
    pub last_lower_synced_slot: Option<u32>,
    pub last_upper_synced_slot: Option<u32>,
    pub last_upper_synced_block_root: Option<B256>,
    pub last_upper_synced_block_slot: Option<u32>,
}

#[derive(Serialize, Debug)]
pub struct IndexRequest {
    pub block: Block,
    pub transactions: Vec<Transaction>,
    pub blobs: Vec<Blob>,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ReorgedBlocksRequestBody {
    pub forwarded_blocks: Vec<B256>,
    pub rewinded_blocks: Vec<B256>,
}

impl fmt::Debug for Blob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Blob {{ versioned_hash: {}, commitment: {}, tx_hash: {}, index: {}, data: [omitted] }}",
            self.versioned_hash, self.commitment, self.tx_hash, self.index
        )
    }
}

impl From<(u32, u32)> for FailedSlotsChunk {
    fn from((initial_slot, final_slot): (u32, u32)) -> Self {
        Self {
            id: None,
            initial_slot,
            final_slot,
        }
    }
}

impl<'a> TryFrom<(&'a ExecutionBlock<ExecutionTransaction>, u32)> for Block {
    type Error = anyhow::Error;

    fn try_from(
        (execution_block, slot): (&'a ExecutionBlock<ExecutionTransaction>, u32),
    ) -> Result<Self, Self::Error> {
        let number = execution_block.header.number;
        let hash = execution_block.header.hash;
        let timestamp = execution_block.header.timestamp;
        let blob_gas_used = match execution_block.header.blob_gas_used {
            Some(blob_gas_used) => U256::from::<u64>(blob_gas_used),
            None => {
                return Err(anyhow::anyhow!(
                    "Missing `blob_gas_used` field in execution block {hash} with number {number}",
                    hash = hash,
                    number = number
                ))
            }
        };
        let excess_blob_gas = match execution_block.header.excess_blob_gas {
            Some(excess_blob_gas) => U256::from::<u64>(excess_blob_gas),
            None => {
                return Err(anyhow::anyhow!(
                "Missing `excess_blob_gas` field in execution block {hash} with number {number}",
                hash = hash,
                number = number
            ))
            }
        };

        Ok(Self {
            number,
            hash,
            timestamp,
            slot,
            blob_gas_used,
            excess_blob_gas,
        })
    }
}

impl<'a>
    TryFrom<(
        &'a ExecutionTransaction,
        &'a ExecutionBlock<ExecutionTransaction>,
    )> for Transaction
{
    type Error = anyhow::Error;

    fn try_from(
        (execution_tx, execution_block): (
            &'a ExecutionTransaction,
            &'a ExecutionBlock<ExecutionTransaction>,
        ),
    ) -> Result<Self, Self::Error> {
        let block_number = execution_block.header.number;
        let hash = execution_tx
            .info()
            .hash
            .with_context(|| format!("Missing `hash` field in tx within block {block_number}"))?;
        let index = execution_tx.transaction_index.with_context(|| {
            format!("Missing `transaction_index` field in tx {hash} within block {block_number}")
        })?;
        let from = execution_tx.inner.signer();
        let to = Some(execution_tx.to().with_context(|| {
            format!("Missing `to` field in tx {hash} within block {block_number}")
        })?);
        let gas_price = U256::from::<u128>(execution_tx.effective_gas_price(None));

        let max_fee_per_blob_gas = match execution_tx.max_fee_per_blob_gas() {
            Some(max_fee_per_blob_gas) => U256::from::<u128>(max_fee_per_blob_gas),
            None => {
                return Err(anyhow::anyhow!(
                    "Missing `max_fee_per_blob_gas` field in tx {hash} within block {block_number}",
                    hash = hash,
                    block_number = block_number
                ))
            }
        };

        Ok(Self {
            block_number,
            index,
            hash,
            from,
            to,
            gas_price,
            max_fee_per_blob_gas,
        })
    }
}

impl<'a> TryFrom<(&'a BeaconBlob, u32, B256)> for Blob {
    type Error = anyhow::Error;

    fn try_from(
        (blob_data, index, tx_hash): (&'a BeaconBlob, u32, B256),
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            tx_hash,
            index,
            commitment: blob_data.kzg_commitment.clone(),
            proof: blob_data.kzg_proof.clone(),
            data: blob_data.blob.clone(),
            versioned_hash: calculate_versioned_hash(&blob_data.kzg_commitment)?,
        })
    }
}

impl<'a> From<(&'a BeaconBlob, &'a B256, usize, &'a B256)> for Blob {
    fn from(
        (blob_data, versioned_hash, index, tx_hash): (&'a BeaconBlob, &'a B256, usize, &'a B256),
    ) -> Self {
        Self {
            tx_hash: *tx_hash,
            index: index as u32,
            commitment: blob_data.kzg_commitment.clone(),
            proof: blob_data.kzg_proof.clone(),
            data: blob_data.blob.clone(),
            versioned_hash: *versioned_hash,
        }
    }
}

impl From<BlockchainSyncStateResponse> for BlockchainSyncState {
    fn from(response: BlockchainSyncStateResponse) -> Self {
        Self {
            last_finalized_block: None,
            last_lower_synced_slot: response.last_lower_synced_slot,
            last_upper_synced_slot: response.last_upper_synced_slot,
            last_upper_synced_block_root: response.last_upper_synced_block_root,
            last_upper_synced_block_slot: response.last_upper_synced_block_slot,
        }
    }
}

impl From<BlockchainSyncState> for BlockchainSyncStateRequest {
    fn from(sync_state: BlockchainSyncState) -> Self {
        Self {
            last_lower_synced_slot: sync_state.last_lower_synced_slot,
            last_upper_synced_slot: sync_state.last_upper_synced_slot,
            last_finalized_block: sync_state.last_finalized_block,
            last_upper_synced_block_root: sync_state.last_upper_synced_block_root,
            last_upper_synced_block_slot: sync_state.last_upper_synced_block_slot,
        }
    }
}
