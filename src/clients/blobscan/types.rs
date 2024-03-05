use core::fmt;

use anyhow::{Context, Result};
use ethers::types::{
    Address, Block as EthersBlock, Bytes, Transaction as EthersTransaction, H256, U256, U64,
};
use serde::{Deserialize, Serialize};

use crate::{clients::beacon::types::Blob as BeaconBlob, utils::web3::calculate_versioned_hash};

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    pub number: U64,
    pub hash: H256,
    pub timestamp: U256,
    pub slot: u32,
    pub blob_gas_used: U256,
    pub excess_blob_gas: U256,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Transaction {
    pub hash: H256,
    pub from: Address,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<Address>,
    pub block_number: U64,
    pub gas_price: U256,
    pub max_fee_per_blob_gas: U256,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Blob {
    pub versioned_hash: H256,
    pub commitment: String,
    pub proof: String,
    pub data: Bytes,
    pub tx_hash: H256,
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
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BlockchainSyncStateResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_lower_synced_slot: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_upper_synced_slot: Option<u32>,
}

#[derive(Debug)]
pub struct BlockchainSyncState {
    pub last_finalized_block: Option<u32>,
    pub last_lower_synced_slot: Option<u32>,
    pub last_upper_synced_slot: Option<u32>,
}

#[derive(Serialize, Debug)]
pub struct IndexRequest {
    pub block: Block,
    pub transactions: Vec<Transaction>,
    pub blobs: Vec<Blob>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ReorgedSlotRequest {
    pub new_head_slot: u32,
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

impl<'a> TryFrom<(&'a EthersBlock<EthersTransaction>, u32)> for Block {
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
            blob_gas_used: match ethers_block.other.get("blobGasUsed") {
                Some(blob_gas_used) => {
                    let blob_gas_used = blob_gas_used.as_str().with_context(|| {
                        format!("Failed to convert `blobGasUsed` field in execution block {number}")
                    })?;

                    U256::from_str_radix(blob_gas_used, 16)?
                }
                None => {
                    return Err(anyhow::anyhow!(
                        "Missing `blobGasUsed` field in execution block {number}"
                    ))
                }
            },
            excess_blob_gas: match ethers_block.other.get("excessBlobGas") {
                Some(excess_gas_gas) => {
                    let excess_blob_gas = excess_gas_gas.as_str().with_context(|| {
                        format!(
                            "Failed to convert excess blob gas field in execution block {number}"
                        )
                    })?;

                    U256::from_str_radix(excess_blob_gas, 16)?
                }
                None => {
                    return Err(anyhow::anyhow!(
                        "Missing `excessBlobGas` field in execution block {number}"
                    ))
                }
            },
        })
    }
}

impl<'a> TryFrom<(&'a EthersTransaction, &'a EthersBlock<EthersTransaction>)> for Transaction {
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
            gas_price: ethers_tx.gas_price.with_context(|| {
                format!("Missing gas price field in transaction {hash}", hash = hash)
            })?,
            max_fee_per_blob_gas: match ethers_tx.other.get("maxFeePerBlobGas") {
                Some(max_fee_per_blob_gas) => {
                    let max_fee_per_blob_gas =
                        max_fee_per_blob_gas.as_str().with_context(|| {
                            format!(
                                "Failed to convert `maxFeePerBlobGas` field in transaction {hash}",
                                hash = hash
                            )
                        })?;

                    U256::from_str_radix(max_fee_per_blob_gas, 16)?
                }
                None => {
                    return Err(anyhow::anyhow!(
                        "Missing `maxFeePerBlobGas` field in transaction {hash}",
                        hash = hash
                    ))
                }
            },
        })
    }
}

impl<'a> TryFrom<(&'a BeaconBlob, u32, H256)> for Blob {
    type Error = anyhow::Error;

    fn try_from(
        (blob_data, index, tx_hash): (&'a BeaconBlob, u32, H256),
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

impl<'a> From<(&'a BeaconBlob, &'a H256, usize, &'a H256)> for Blob {
    fn from(
        (blob_data, versioned_hash, index, tx_hash): (&'a BeaconBlob, &'a H256, usize, &'a H256),
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
        }
    }
}

impl From<BlockchainSyncState> for BlockchainSyncStateRequest {
    fn from(sync_state: BlockchainSyncState) -> Self {
        Self {
            last_lower_synced_slot: sync_state.last_lower_synced_slot,
            last_upper_synced_slot: sync_state.last_upper_synced_slot,
            last_finalized_block: sync_state.last_finalized_block,
        }
    }
}
