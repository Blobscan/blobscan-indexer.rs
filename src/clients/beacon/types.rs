use std::{fmt, str::FromStr};

use alloy::{consensus::Bytes48, eips::eip4844::HeapBlob, primitives::B256};
use anyhow::anyhow;
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
#[derive(Debug, Clone)]
pub struct Block {
    pub blob_kzg_commitments: Option<Vec<KzgCommitment>>,
    pub execution_payload: Option<BlockExecutionPayload>,
    pub parent_root: B256,
    pub slot: u32,
}

/// What the indexer needs to know about the execution payload of a beacon block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockExecutionPayload {
    /// Before Gloas (EIP-7732, ePBS): the payload is in the beacon block body, so the hash and
    /// the block number are known.
    InBody { block_hash: B256, block_number: u32 },
    /// Gloas and later: the body has a signed bid that commits to the payload by hash. The bid
    /// does not have the block number (ask the execution client). The builder can withhold the
    /// payload, so only the child block tells if the payload became part of the chain.
    Bid {
        block_hash: B256,
        /// Hash of the last execution payload that is part of the chain before this block. It
        /// is the payload of the parent block if the parent is full, and an older payload if the
        /// parent is empty.
        parent_block_hash: B256,
    },
}

impl Block {
    /// Gloas: returns the hash of the execution payload of `parent` if this block builds on it.
    /// That is, the parent is full and its payload is part of the chain.
    ///
    /// Returns `None` if the parent is empty (its builder withheld the payload, or the payload
    /// came too late), or if one of the two blocks is not a Gloas block.
    pub fn full_parent_payload_hash(&self, parent: &Block) -> Option<B256> {
        match (&self.execution_payload, &parent.execution_payload) {
            (
                Some(BlockExecutionPayload::Bid {
                    parent_block_hash, ..
                }),
                Some(BlockExecutionPayload::Bid { block_hash, .. }),
            ) if parent_block_hash == block_hash => Some(*block_hash),
            _ => None,
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct ExecutionPayload {
    pub block_hash: B256,
    #[serde(deserialize_with = "deserialize_u32")]
    pub block_number: u32,
}

#[derive(Deserialize, Debug)]
pub struct ExecutionPayloadBid {
    pub parent_block_hash: B256,
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
    /// Name of the fork of the block (`deneb`, `fulu`, `gloas`, ...). It tells how to read the
    /// body.
    pub version: String,
    pub data: BlockData,
}

/// A blob with its KZG commitment and KZG proof.
#[derive(Debug)]
pub struct Blob {
    pub kzg_commitment: KzgCommitment,
    pub kzg_proof: Proof,
    pub blob: HeapBlob,
}

/// Response of `GET /eth/v1/beacon/blobs/{block_id}`: the raw blobs of the block, in the order of
/// its KZG commitments, without commitments or proofs.
#[derive(Deserialize, Debug)]
pub struct BlobsResponse {
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

/// Forks before Gloas. In these forks the execution payload is in the block body. All other
/// forks, also forks that are not known yet, must have an execution payload bid.
const PRE_GLOAS_FORKS: [&str; 7] = [
    "phase0",
    "altair",
    "bellatrix",
    "capella",
    "deneb",
    "electra",
    "fulu",
];

impl TryFrom<BlockResponse> for Block {
    type Error = anyhow::Error;

    fn try_from(response: BlockResponse) -> Result<Self, Self::Error> {
        let version = response.version;
        let message = response.data.message;
        let body = message.body;

        let (execution_payload, blob_kzg_commitments) =
            if PRE_GLOAS_FORKS.contains(&version.as_str()) {
                (
                    body.execution_payload
                        .map(|payload| BlockExecutionPayload::InBody {
                            block_hash: payload.block_hash,
                            block_number: payload.block_number,
                        }),
                    body.blob_kzg_commitments,
                )
            } else {
                let bid = body
                    .signed_execution_payload_bid
                    .ok_or_else(|| {
                        anyhow!(
                            "Block at slot {} of fork '{version}' has no execution payload bid",
                            message.slot
                        )
                    })?
                    .message;

                (
                    Some(BlockExecutionPayload::Bid {
                        block_hash: bid.block_hash,
                        parent_block_hash: bid.parent_block_hash,
                    }),
                    Some(bid.blob_kzg_commitments),
                )
            };

        Ok(Block {
            blob_kzg_commitments,
            execution_payload,
            parent_root: message.parent_root,
            slot: message.slot,
        })
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
    const PARENT_BLOCK_HASH: &str =
        "0x5b1a0c0e2c3f4a6b7d8e9f00112233445566778899aabbccddeeff0011223344";

    fn try_block_from(version: &str, body: &str) -> anyhow::Result<Block> {
        let json = format!(
            r#"{{"version":"{version}","data":{{"message":{{"slot":"11384159","parent_root":"{PARENT_ROOT}","body":{body}}}}}}}"#
        );

        serde_json::from_str::<BlockResponse>(&json)
            .unwrap()
            .try_into()
    }

    fn block_from(version: &str, body: &str) -> Block {
        try_block_from(version, body).unwrap()
    }

    fn bid_body() -> String {
        format!(
            r#"{{"signed_execution_payload_bid":{{"message":{{"parent_block_hash":"{PARENT_BLOCK_HASH}","block_hash":"{BLOCK_HASH}","builder_index":"18446744073709551615","blob_kzg_commitments":["{COMMITMENT_0}","{COMMITMENT_1}"]}},"signature":"0x00"}}}}"#
        )
    }

    fn payload_body() -> String {
        format!(
            r#"{{"execution_payload":{{"block_hash":"{BLOCK_HASH}","block_number":"11942470"}},"blob_kzg_commitments":["{COMMITMENT_0}"]}}"#
        )
    }

    /// Shape of a Gloas block on sepolia (slot 11384159, `version: gloas`): no `execution_payload`
    /// and no `blob_kzg_commitments` in the body. Both are in the signed bid.
    #[test]
    fn gloas_block_takes_the_hash_and_commitments_from_the_bid() {
        let block = block_from("gloas", &bid_body());

        assert_eq!(
            block.execution_payload,
            Some(BlockExecutionPayload::Bid {
                block_hash: BLOCK_HASH.parse().unwrap(),
                parent_block_hash: PARENT_BLOCK_HASH.parse().unwrap(),
            })
        );
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
        let block = block_from("fulu", &payload_body());

        assert_eq!(
            block.execution_payload,
            Some(BlockExecutionPayload::InBody {
                block_hash: BLOCK_HASH.parse().unwrap(),
                block_number: 11942470
            })
        );
        assert_eq!(block.blob_kzg_commitments.unwrap().len(), 1);
    }

    /// The version decides how to read the body, not the fields that are in it.
    #[test]
    fn a_pre_gloas_block_ignores_a_bid_field() {
        let body = format!(
            r#"{{"execution_payload":{{"block_hash":"{BLOCK_HASH}","block_number":"1"}},"signed_execution_payload_bid":{{"message":{{"parent_block_hash":"{PARENT_ROOT}","block_hash":"{PARENT_ROOT}","blob_kzg_commitments":[]}}}}}}"#
        );

        let block = block_from("electra", &body);

        assert_eq!(
            block.execution_payload,
            Some(BlockExecutionPayload::InBody {
                block_hash: BLOCK_HASH.parse().unwrap(),
                block_number: 1
            })
        );
    }

    #[test]
    fn a_gloas_block_without_a_bid_is_an_error() {
        let err = try_block_from("gloas", &payload_body()).unwrap_err();

        assert!(
            err.to_string().contains("has no execution payload bid"),
            "{err}"
        );
    }

    /// A fork after Gloas that this indexer does not know yet is read as a Gloas block.
    #[test]
    fn an_unknown_fork_is_read_like_gloas() {
        let block = block_from("heze", &bid_body());

        assert_eq!(
            block.execution_payload,
            Some(BlockExecutionPayload::Bid {
                block_hash: BLOCK_HASH.parse().unwrap(),
                parent_block_hash: PARENT_BLOCK_HASH.parse().unwrap(),
            })
        );
    }

    /// Blocks before Bellatrix have no execution payload. The parser must not make one.
    #[test]
    fn a_pre_bellatrix_block_has_no_execution_payload() {
        let block = block_from("altair", r#"{"attestations":[]}"#);

        assert!(block.execution_payload.is_none());
        assert!(block.blob_kzg_commitments.is_none());
    }

    fn bid_block(block_hash: u8, parent_block_hash: u8) -> Block {
        Block {
            blob_kzg_commitments: None,
            execution_payload: Some(BlockExecutionPayload::Bid {
                block_hash: B256::repeat_byte(block_hash),
                parent_block_hash: B256::repeat_byte(parent_block_hash),
            }),
            parent_root: B256::ZERO,
            slot: 0,
        }
    }

    #[test]
    fn a_child_that_builds_on_the_parent_payload_makes_the_parent_full() {
        let parent = bid_block(0x02, 0x01);
        let child = bid_block(0x03, 0x02);

        assert_eq!(
            child.full_parent_payload_hash(&parent),
            Some(B256::repeat_byte(0x02))
        );
    }

    /// The parent payload was withheld: the child builds on the payload before it.
    #[test]
    fn a_child_that_skips_the_parent_payload_makes_the_parent_empty() {
        let parent = bid_block(0x02, 0x01);
        let child = bid_block(0x03, 0x01);

        assert_eq!(child.full_parent_payload_hash(&parent), None);
    }

    /// The first Gloas block: the old code path already indexed the payload of its parent.
    #[test]
    fn a_pre_gloas_parent_is_never_full() {
        let parent = Block {
            execution_payload: Some(BlockExecutionPayload::InBody {
                block_hash: B256::repeat_byte(0x02),
                block_number: 1,
            }),
            ..bid_block(0, 0)
        };
        let child = bid_block(0x03, 0x02);

        assert_eq!(child.full_parent_payload_hash(&parent), None);
    }
}
