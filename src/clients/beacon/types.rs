use std::fmt;

use ethers::types::{Bytes, H256};
use serde::Deserialize;

pub enum BlockId {
    Head,
    Finalized,
    Slot(u32),
}

impl From<BlockId> for String {
    fn from(value: BlockId) -> Self {
        match value {
            BlockId::Head => String::from("head"),
            BlockId::Finalized => String::from("finalized"),
            BlockId::Slot(slot) => slot.to_string(),
        }
    }
}

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BlockId::Head => write!(f, "head"),
            BlockId::Finalized => write!(f, "finalized"),
            BlockId::Slot(slot) => write!(f, "{}", slot),
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct ExecutionPayload {
    pub block_hash: H256,
}

#[derive(Deserialize, Debug)]
pub struct BlockBody {
    pub execution_payload: Option<ExecutionPayload>,
    pub blob_kzg_commitments: Option<Vec<String>>,
}
#[derive(Deserialize, Debug)]
pub struct BlockMessage {
    pub slot: String,
    pub body: BlockBody,
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
    pub blob: Bytes,
}

#[derive(Deserialize, Debug)]
pub struct BlobsResponse {
    pub data: Vec<Blob>,
}
