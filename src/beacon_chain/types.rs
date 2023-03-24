use ethers::types::{Bytes, H256};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct ExecutionPayload {
    pub block_hash: H256,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BlockBody {
    pub execution_payload: Option<ExecutionPayload>,
    pub blob_kzg_commitments: Option<Vec<String>>,
}
#[derive(Serialize, Deserialize, Debug)]
pub struct BlockMessage {
    pub slot: String,
    pub body: BlockBody,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Block {
    pub message: BlockMessage,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BlockResponse {
    pub data: Block,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BlobsSidecar {
    pub blobs: Vec<Bytes>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BlobsSidecarResponse {
    pub data: BlobsSidecar,
}
