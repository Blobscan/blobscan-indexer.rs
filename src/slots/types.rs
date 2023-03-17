use ethers::types::Bytes;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct ExecutionPayload {
    pub block_hash: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MessageBody {
    pub execution_payload: Option<ExecutionPayload>,
    pub blob_kzg_commitments: Option<Vec<String>>,
}
#[derive(Serialize, Deserialize, Debug)]
pub struct BlockMessage {
    pub slot: String,
    pub body: MessageBody,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ResponseData {
    pub message: BlockMessage,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BeaconAPIResponse {
    pub data: ResponseData,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SidecarData {
    pub blobs: Vec<Bytes>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BeaconSidecarResponse {
    pub data: SidecarData,
}
