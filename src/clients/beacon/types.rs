use std::{fmt, str::FromStr};

use alloy::primitives::{Bytes, B256};
use serde::{Deserialize, Serialize};

use crate::clients::common::ClientError;

use super::CommonBeaconClient;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub enum BlockId {
    Head,
    Finalized,
    Slot(u32),
    Hash(B256),
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Topic {
    Head,
    FinalizedCheckpoint,
}

#[derive(Deserialize, Debug)]
pub struct Block {
    pub blob_kzg_commitments: Option<Vec<String>>,
    pub execution_payload: Option<ExecutionPayload>,
    pub parent_root: B256,
    #[serde(deserialize_with = "deserialize_number")]
    pub slot: u32,
}

#[derive(Deserialize, Debug)]
pub struct ExecutionPayload {
    pub block_hash: B256,
    #[serde(deserialize_with = "deserialize_number")]
    pub block_number: u32,
}

#[derive(Deserialize, Debug)]
pub struct BlockBody {
    pub execution_payload: Option<ExecutionPayload>,
    pub blob_kzg_commitments: Option<Vec<String>>,
}
#[derive(Deserialize, Debug)]
pub struct BlockMessage {
    pub body: BlockBody,
    pub parent_root: B256,
    #[serde(deserialize_with = "deserialize_number")]
    pub slot: u32,
}

#[derive(Deserialize, Debug)]
pub struct BlockData {
    pub message: BlockMessage,
}

#[derive(Deserialize, Debug)]
pub struct BlockResponse {
    pub data: BlockData,
}

#[derive(Deserialize, Debug)]
pub struct Blob {
    pub kzg_commitment: String,
    pub kzg_proof: String,
    pub blob: Bytes,
}

#[derive(Deserialize, Debug)]
pub struct BlobsResponse {
    pub data: Vec<Blob>,
}

#[derive(Deserialize, Debug)]
pub struct BlockHeaderResponse {
    pub data: BlockHeaderData,
}

#[derive(Deserialize, Debug, Clone)]
pub struct BlockHeader {
    pub root: B256,
    pub parent_root: B256,
    pub slot: u32,
}

#[derive(Deserialize, Debug)]
pub struct BlockHeaderData {
    pub root: B256,
    pub header: InnerBlockHeader,
}
#[derive(Deserialize, Debug)]
pub struct InnerBlockHeader {
    pub message: BlockHeaderMessage,
}

#[derive(Deserialize, Debug)]
pub struct BlockHeaderMessage {
    pub parent_root: B256,
    #[serde(deserialize_with = "deserialize_number")]
    pub slot: u32,
}

#[derive(Deserialize, Debug)]
pub struct HeadEventData {
    #[serde(deserialize_with = "deserialize_number")]
    pub slot: u32,
    #[allow(dead_code)]
    pub block: B256,
}

#[derive(Deserialize, Debug)]
pub struct FinalizedCheckpointEventData {
    pub block: B256,
}

fn deserialize_number<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;

    value.parse::<u32>().map_err(serde::de::Error::custom)
}

impl BlockId {
    pub fn to_detailed_string(&self) -> String {
        match self {
            BlockId::Head => String::from("head"),
            BlockId::Finalized => String::from("finalized"),
            BlockId::Slot(slot) => slot.to_string(),
            BlockId::Hash(hash) => format!("0x{:x}", hash),
        }
    }
}

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BlockId::Head => write!(f, "head"),
            BlockId::Finalized => write!(f, "finalized"),
            BlockId::Slot(slot) => write!(f, "{}", slot),
            BlockId::Hash(hash) => write!(f, "{}", hash),
        }
    }
}

impl FromStr for BlockId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "head" => Ok(BlockId::Head),
            "finalized" => Ok(BlockId::Finalized),
            _ => match s.parse::<u32>() {
                Ok(num) => Ok(BlockId::Slot(num)),
                Err(_) => {
                    if s.starts_with("0x") {
                        match B256::from_str(s) {
                            Ok(hash) => Ok(BlockId::Hash(hash)),
                            Err(_) => Err(format!("Invalid block ID hash: {s}")),
                        }
                    } else {
                        Err(
                            format!("Invalid block ID: {s}. Expected 'head', 'finalized', a hash or a number."),
                        )
                    }
                }
            },
        }
    }
}

impl From<&Topic> for String {
    fn from(value: &Topic) -> Self {
        match value {
            Topic::Head => String::from("head"),
            Topic::FinalizedCheckpoint => String::from("finalized_checkpoint"),
        }
    }
}

impl From<B256> for BlockId {
    fn from(value: B256) -> Self {
        BlockId::Hash(value)
    }
}

impl From<u32> for BlockId {
    fn from(value: u32) -> Self {
        BlockId::Slot(value)
    }
}

impl From<BlockHeaderResponse> for BlockHeader {
    fn from(response: BlockHeaderResponse) -> Self {
        BlockHeader {
            root: response.data.root,
            parent_root: response.data.header.message.parent_root,
            slot: response.data.header.message.slot,
        }
    }
}

impl From<BlockResponse> for Block {
    fn from(response: BlockResponse) -> Self {
        Block {
            blob_kzg_commitments: response.data.message.body.blob_kzg_commitments,
            execution_payload: response.data.message.body.execution_payload,
            parent_root: response.data.message.parent_root,
            slot: response.data.message.slot,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BlockIdResolutionError {
    #[error("Block with id '{0}' not found")]
    BlockNotFound(BlockId),
    #[error("Failed to resolve block id '{block_id}'")]
    FailedBlockIdResolution {
        block_id: BlockId,
        #[source]
        error: ClientError,
    },
}

pub trait BlockIdResolution {
    async fn resolve_to_slot(
        &self,
        beacon_client: &dyn CommonBeaconClient,
    ) -> Result<u32, BlockIdResolutionError>;
}

impl BlockIdResolution for BlockId {
    async fn resolve_to_slot(
        &self,
        beacon_client: &dyn CommonBeaconClient,
    ) -> Result<u32, BlockIdResolutionError> {
        match self {
            BlockId::Slot(slot) => Ok(*slot),
            _ => match beacon_client
                .get_block_header(self.clone().into())
                .await
                .map_err(|err| BlockIdResolutionError::FailedBlockIdResolution {
                    block_id: self.clone(),
                    error: err,
                })? {
                Some(header) => Ok(header.slot),
                None => Err(BlockIdResolutionError::BlockNotFound(self.clone())),
            },
        }
    }
}
