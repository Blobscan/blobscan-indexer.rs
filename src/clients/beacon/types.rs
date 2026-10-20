use std::{fmt, str::FromStr};

use alloy::{consensus::Bytes48, eips::eip4844::HeapBlob, primitives::B256};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::clients::common::ClientError;

use super::CommonBeaconClient;

pub type KzgCommitment = Bytes48;

pub type Proof = Bytes48;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub enum BlockId {
    Head,
    Finalized,
    Slot(u32),
    Hash(B256),
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum Topic {
    Head,
    FinalizedCheckpoint,
}

impl fmt::Display for Topic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Topic::Head => write!(f, "head"),
            Topic::FinalizedCheckpoint => write!(f, "finalized_checkpoint"),
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct SpecResponse {
    pub data: Spec,
}

#[derive(Deserialize, Debug)]
pub struct Spec {
    #[serde(rename = "DEPOSIT_NETWORK_ID", deserialize_with = "deserialize_u64")]
    pub deposit_network_id: u64,
}
#[derive(Debug)]
pub struct Block {
    pub blob_kzg_commitments: Option<Vec<KzgCommitment>>,
    pub execution_payload: Option<BlockExecutionPayload>,
    pub parent_root: B256,
    pub slot: u32,
}

/// What the indexer needs to know about the execution payload of a beacon block.
///
/// Before Gloas (EIP-7732, ePBS) the payload sits inside the beacon block body, so both the
/// hash and the block number are known. From Gloas on the body only carries a signed bid that
/// commits to the payload by hash: the number is not there (ask the execution client) and the
/// payload itself reaches the execution client later, or never if the builder withholds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockExecutionPayload {
    pub block_hash: B256,
    pub block_number: Option<u32>,
    /// True when the hash comes from a Gloas execution payload bid.
    pub from_bid: bool,
}

#[derive(Deserialize, Debug)]
pub struct ExecutionPayload {
    pub block_hash: B256,
    #[serde(deserialize_with = "deserialize_u32")]
    pub block_number: u32,
}

#[derive(Deserialize, Debug)]
pub struct ExecutionPayloadBid {
    pub block_hash: B256,
    pub blob_kzg_commitments: Vec<KzgCommitment>,
}

#[derive(Deserialize, Debug)]
pub struct SignedExecutionPayloadBid {
    pub message: ExecutionPayloadBid,
}

#[derive(Deserialize, Debug)]
pub struct BlockBody {
    pub execution_payload: Option<ExecutionPayload>,
    pub blob_kzg_commitments: Option<Vec<KzgCommitment>>,
    /// Gloas and later: replaces `execution_payload` and `blob_kzg_commitments`.
    pub signed_execution_payload_bid: Option<SignedExecutionPayloadBid>,
}
#[derive(Deserialize, Debug)]
pub struct BlockMessage {
    pub body: BlockBody,
    pub parent_root: B256,
    #[serde(deserialize_with = "deserialize_u32")]
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
    pub kzg_commitment: KzgCommitment,
    pub kzg_proof: Proof,
    pub blob: HeapBlob,
}

#[derive(Deserialize, Debug)]
pub struct BlobsResponse {
    pub data: Vec<Blob>,
}

/// Response of `GET /eth/v1/beacon/blobs/{block_id}`: the raw blobs of the block, in the order of
/// its KZG commitments, without commitments or proofs.
#[derive(Deserialize, Debug)]
pub struct BlobDataResponse {
    pub data: Vec<HeapBlob>,
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
    #[serde(deserialize_with = "deserialize_u32")]
    pub slot: u32,
}

#[derive(Deserialize, Debug)]
pub struct HeadEventData {
    #[serde(deserialize_with = "deserialize_u32")]
    pub slot: u32,
    #[allow(dead_code)]
    pub block: B256,
}

#[derive(Deserialize, Debug)]
pub struct FinalizedCheckpointEventData {
    pub block: B256,
}

fn deserialize_u32<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;

    value.parse::<u32>().map_err(serde::de::Error::custom)
}

fn deserialize_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;

    value.parse::<u64>().map_err(serde::de::Error::custom)
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
        let message = response.data.message;
        let body = message.body;

        let (execution_payload, blob_kzg_commitments) =
            match (body.execution_payload, body.signed_execution_payload_bid) {
                (Some(payload), _) => (
                    Some(BlockExecutionPayload {
                        block_hash: payload.block_hash,
                        block_number: Some(payload.block_number),
                        from_bid: false,
                    }),
                    body.blob_kzg_commitments,
                ),
                (None, Some(bid)) => (
                    Some(BlockExecutionPayload {
                        block_hash: bid.message.block_hash,
                        block_number: None,
                        from_bid: true,
                    }),
                    Some(bid.message.blob_kzg_commitments),
                ),
                (None, None) => (None, body.blob_kzg_commitments),
            };

        Block {
            blob_kzg_commitments,
            execution_payload,
            parent_root: message.parent_root,
            slot: message.slot,
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

#[async_trait]
pub trait BlockIdResolution: Send + Sync {
    async fn resolve_to_slot(
        &self,
        beacon_client: &dyn CommonBeaconClient,
    ) -> Result<u32, BlockIdResolutionError>;
}

#[async_trait]
impl BlockIdResolution for BlockId {
    async fn resolve_to_slot(
        &self,
        beacon_client: &dyn CommonBeaconClient,
    ) -> Result<u32, BlockIdResolutionError> {
        match self {
            BlockId::Slot(slot) => Ok(*slot),
            _ => match beacon_client
                .get_block_header(self.clone())
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

#[cfg(test)]
mod block_tests {
    use super::*;

    const BLOCK_HASH: &str = "0x0743067d9d07a963ba5904287029fd5d4910bb26fd3aa95ba92c691cdc04ae12";
    const COMMITMENT_0: &str = "0xa0938f80d83f02fae778eb3347ed524dcff94ad1f01dd0f617226054180c5aacb62356b0bf9bcabd8c668189643be18b";
    const COMMITMENT_1: &str = "0x85f6e8b0e18cc996fefdc260e28d7ed1860c2af5cad02c15721cfb1aea02a6173797e2e7cc8e04c171babf87a561825e";
    const PARENT_ROOT: &str = "0xef1481e62dd5c92d881e9a18ec3adf644d42a4ef6cef201daf0718ef02266d14";

    fn block_from(body: &str) -> Block {
        let json = format!(
            r#"{{"data":{{"message":{{"slot":"11384159","parent_root":"{PARENT_ROOT}","body":{body}}}}}}}"#
        );

        serde_json::from_str::<BlockResponse>(&json).unwrap().into()
    }

    /// Shape of a Gloas block on sepolia (slot 11384159, `version: gloas`): no `execution_payload`
    /// and no `blob_kzg_commitments` in the body, both live in the signed bid.
    #[test]
    fn gloas_block_takes_the_hash_and_commitments_from_the_bid() {
        let block = block_from(&format!(
            r#"{{"signed_execution_payload_bid":{{"message":{{"block_hash":"{BLOCK_HASH}","builder_index":"18446744073709551615","blob_kzg_commitments":["{COMMITMENT_0}","{COMMITMENT_1}"]}},"signature":"0x00"}}}}"#
        ));

        let payload = block
            .execution_payload
            .expect("the bid must yield a payload");

        assert_eq!(payload.block_hash, BLOCK_HASH.parse::<B256>().unwrap());
        assert_eq!(payload.block_number, None);
        assert!(payload.from_bid);
        assert_eq!(
            block.blob_kzg_commitments.unwrap(),
            vec![
                COMMITMENT_0.parse::<KzgCommitment>().unwrap(),
                COMMITMENT_1.parse::<KzgCommitment>().unwrap()
            ]
        );
        assert_eq!(block.slot, 11384159);
    }

    #[test]
    fn pre_gloas_block_keeps_its_payload_number_and_commitments() {
        let block = block_from(&format!(
            r#"{{"execution_payload":{{"block_hash":"{BLOCK_HASH}","block_number":"11942470"}},"blob_kzg_commitments":["{COMMITMENT_0}"]}}"#
        ));

        let payload = block.execution_payload.unwrap();

        assert_eq!(payload.block_number, Some(11942470));
        assert!(!payload.from_bid);
        assert_eq!(block.blob_kzg_commitments.unwrap().len(), 1);
    }

    #[test]
    fn a_body_with_the_payload_wins_over_a_bid() {
        let block = block_from(&format!(
            r#"{{"execution_payload":{{"block_hash":"{BLOCK_HASH}","block_number":"1"}},"blob_kzg_commitments":["{COMMITMENT_0}"],"signed_execution_payload_bid":{{"message":{{"block_hash":"{PARENT_ROOT}","blob_kzg_commitments":[]}}}}}}"#
        ));

        let payload = block.execution_payload.unwrap();

        assert_eq!(payload.block_hash, BLOCK_HASH.parse::<B256>().unwrap());
        assert!(!payload.from_bid);
    }

    /// Pre-Bellatrix blocks, or a fork this indexer does not parse yet: no payload information at
    /// all. The callers decide what to do; the parser must not invent a payload.
    #[test]
    fn a_body_without_payload_information_has_no_execution_payload() {
        let block = block_from(r#"{"attestations":[]}"#);

        assert!(block.execution_payload.is_none());
        assert!(block.blob_kzg_commitments.is_none());
    }
}
