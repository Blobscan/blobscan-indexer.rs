use std::{fmt, str::FromStr};

use ethers::types::{Bytes, H256};
use serde::{Deserialize, Serialize};

use crate::slots_processor::BlockData;

#[derive(Serialize, Debug, Clone)]
pub enum BlockId {
    Head,
    Finalized,
    Slot(u32),
    Hash(H256),
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Topic {
    Head,
    FinalizedCheckpoint,
    ChainReorg,
}

#[derive(Deserialize, Debug)]
pub struct ExecutionPayload {
    pub block_hash: H256,
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
    #[serde(deserialize_with = "deserialize_number")]
    pub slot: u32,
    pub body: BlockBody,
    pub parent_root: H256,
}

#[derive(Deserialize, Debug)]
pub struct Block {
    pub message: BlockMessage,
}

#[derive(Deserialize, Debug)]
pub struct BlockResponse {
    pub data: Block,
}

#[derive(Deserialize, Debug)]
pub struct Blob {
    pub index: String,
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
    pub data: BlockHeader,
}

#[derive(Deserialize, Debug)]
pub struct BlockHeader {
    pub root: H256,
    pub header: InnerBlockHeader,
}
#[derive(Deserialize, Debug)]
pub struct InnerBlockHeader {
    pub message: BlockHeaderMessage,
}

#[derive(Deserialize, Debug)]
pub struct BlockHeaderMessage {
    pub parent_root: H256,
    #[serde(deserialize_with = "deserialize_number")]
    pub slot: u32,
}

#[derive(Deserialize, Debug)]
pub struct ChainReorgEventData {
    pub old_head_block: H256,
    pub new_head_block: H256,
    #[serde(deserialize_with = "deserialize_number")]
    pub slot: u32,
    #[serde(deserialize_with = "deserialize_number")]
    pub depth: u32,
}

#[derive(Deserialize, Debug)]
pub struct HeadEventData {
    #[serde(deserialize_with = "deserialize_number")]
    pub slot: u32,
    pub block: H256,
}

#[derive(Deserialize, Debug)]
pub struct FinalizedCheckpointEventData {
    pub block: H256,
}

fn deserialize_number<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;

    value.parse::<u32>().map_err(serde::de::Error::custom)
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
                    Err("Invalid block ID. Expected 'head', 'finalized' or a number.".to_string())
                }
            },
        }
    }
}

impl From<&Topic> for String {
    fn from(value: &Topic) -> Self {
        match value {
            Topic::ChainReorg => String::from("chain_reorg"),
            Topic::Head => String::from("head"),
            Topic::FinalizedCheckpoint => String::from("finalized_checkpoint"),
        }
    }
}

impl From<HeadEventData> for BlockData {
    fn from(event_data: HeadEventData) -> Self {
        Self {
            root: event_data.block,
            slot: event_data.slot,
        }
    }
}
