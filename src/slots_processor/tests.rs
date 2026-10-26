//! `index_block` against mocked clients. Blocks of all forks get their raw blobs from
//! `/eth/v1/beacon/blobs`, and the indexer computes the KZG proofs. Gloas blocks also wait for
//! their payload, which the builder reveals after the block.

use std::sync::{Arc, Mutex};

use alloy::{
    eips::eip4844::{kzg_to_versioned_hash, HeapBlob},
    network::Ethereum,
    providers::{Provider, ProviderBuilder},
    transports::mock::Asserter,
};
use c_kzg::{ethereum_kzg_settings, Blob as KzgBlob};
use serde_json::{json, Value};

use super::*;
use crate::{
    clients::{
        beacon::{
            types::{Block as BeaconBlock, BlockExecutionPayload},
            MockCommonBeaconClient,
        },
        blobscan::MockCommonBlobscanClient,
    },
    context::{CommonContext, SyncingSettings},
    network::{Network, NetworkName},
    utils::kzg::{compute_blob_proof, PLACEHOLDER_PROOF},
};

const EXECUTION_BLOCK: &str = include_str!("test_data/sepolia_execution_block.json");
const SLOT: u32 = 11384159;

fn ready<T: Send + 'static>(
    value: T,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>> {
    Box::pin(std::future::ready(value))
}

#[derive(Clone)]
struct TestContext {
    beacon: Arc<MockCommonBeaconClient>,
    blobscan: Arc<MockCommonBlobscanClient>,
    provider: Arc<dyn Provider<Ethereum>>,
    network: Network,
    settings: Arc<SyncingSettings>,
    gloas_payload_wait: Duration,
}

impl CommonContext for TestContext {
    fn beacon_client(&self) -> &dyn crate::clients::beacon::CommonBeaconClient {
        self.beacon.as_ref()
    }

    fn blobscan_client(&self) -> &dyn crate::clients::blobscan::CommonBlobscanClient {
        self.blobscan.as_ref()
    }

    fn network(&self) -> &Network {
        &self.network
    }

    fn provider(&self) -> &dyn Provider<Ethereum> {
        self.provider.as_ref()
    }

    fn syncing_settings(&self) -> &SyncingSettings {
        &self.settings
    }

    fn gloas_payload_wait(&self) -> Duration {
        self.gloas_payload_wait
    }
}

/// What the indexer sent to Blobscan for the blobs.
#[derive(Debug, Clone, PartialEq)]
struct IndexedBlob {
    commitment: B256,
    proof: B256x48,
    index: u32,
}
type B256x48 = alloy::primitives::FixedBytes<48>;

fn blob(seed: u8) -> HeapBlob {
    let mut bytes = vec![0u8; 131_072];

    for (i, chunk) in bytes.chunks_mut(32).enumerate() {
        chunk[1] = seed;
        chunk[2] = (i % 251) as u8;
    }

    HeapBlob::new(&bytes).unwrap()
}

fn commitment_of(blob: &HeapBlob) -> KzgCommitment {
    let kzg_blob = KzgBlob::from_bytes(blob.inner().as_ref()).unwrap();

    KzgCommitment::from_slice(
        ethereum_kzg_settings(0)
            .blob_to_kzg_commitment(&kzg_blob)
            .unwrap()
            .to_bytes()
            .as_ref(),
    )
}

/// The real sepolia block 11942470 (the payload of slot 11384159), trimmed to two blob
/// transactions, with their versioned hashes replaced by those of `commitments`.
fn execution_block_json(commitments: &[KzgCommitment]) -> Value {
    let mut block: Value = serde_json::from_str(EXECUTION_BLOCK).unwrap();

    for (tx, commitment) in block["transactions"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .zip(commitments)
    {
        tx["blobVersionedHashes"] = json!([kzg_to_versioned_hash(commitment.as_ref())]);
    }

    block
}

fn gloas_block(commitments: Vec<KzgCommitment>) -> BeaconBlock {
    BeaconBlock {
        blob_kzg_commitments: Some(commitments),
        execution_payload: Some(BlockExecutionPayload::Bid {
            block_hash: B256::repeat_byte(0x07),
        }),
        parent_root: B256::repeat_byte(0x01),
        slot: SLOT,
    }
}

struct Harness {
    processor: SlotsProcessor,
    indexed: Arc<Mutex<Vec<Vec<IndexedBlob>>>>,
    provider_asserter: Asserter,
}

/// One slot, the default wait for a Gloas payload on most networks.
const PAYLOAD_WAIT: Duration = Duration::from_secs(12);

fn harness(beacon: MockCommonBeaconClient, execution_responses: Vec<Value>) -> Harness {
    harness_with_payload_wait(beacon, execution_responses, PAYLOAD_WAIT)
}

fn harness_with_payload_wait(
    beacon: MockCommonBeaconClient,
    execution_responses: Vec<Value>,
    gloas_payload_wait: Duration,
) -> Harness {
    let indexed = Arc::new(Mutex::new(vec![]));
    let sink = indexed.clone();
    let mut blobscan = MockCommonBlobscanClient::new();

    blobscan.expect_index().returning(move |_, _, blobs| {
        sink.lock().unwrap().push(
            blobs
                .iter()
                .map(|b| IndexedBlob {
                    commitment: B256::from_slice(&b.commitment.as_slice()[..32]),
                    proof: b.proof,
                    index: b.index,
                })
                .collect(),
        );

        ready(Ok(()))
    });

    let provider_asserter = Asserter::new();

    for response in execution_responses {
        provider_asserter.push_success(&response);
    }

    let provider = ProviderBuilder::new().connect_mocked_client(provider_asserter.clone());
    let context = TestContext {
        beacon: Arc::new(beacon),
        blobscan: Arc::new(blobscan),
        provider: Arc::new(provider),
        network: Network {
            name: NetworkName::Devnet,
            chain_id: 11155111,
            dencun_fork_slot: 0,
            epoch: 0,
        },
        settings: Arc::new(SyncingSettings {
            concurrency: 1,
            checkpoint_size: 1,
            disable_checkpoints: true,
        }),
        gloas_payload_wait,
    };

    Harness {
        processor: SlotsProcessor::new(Box::new(context), None),
        indexed,
        provider_asserter,
    }
}

fn proof_for(blob: &HeapBlob, commitment: &KzgCommitment) -> B256x48 {
    compute_blob_proof(blob, commitment).unwrap()
}

#[tokio::test(start_paused = true)]
async fn gloas_block_is_indexed_with_computed_proofs() {
    let blobs = vec![blob(1), blob(2)];
    let commitments: Vec<_> = blobs.iter().map(commitment_of).collect();
    let (raw, block_commitments) = (blobs.clone(), commitments.clone());
    let mut beacon = MockCommonBeaconClient::new();

    beacon
        .expect_get_block()
        .returning(move |_| ready(Ok(Some(gloas_block(block_commitments.clone())))));
    beacon
        .expect_get_blobs()
        .times(1)
        .returning(move |_| ready(Ok(Some(raw.clone()))));

    let h = harness(beacon, vec![execution_block_json(&commitments)]);

    h.processor
        .index_block(B256::repeat_byte(0xaa))
        .await
        .unwrap();

    let indexed = h.indexed.lock().unwrap();

    assert_eq!(indexed.len(), 1, "the block must be indexed exactly once");
    assert_eq!(indexed[0].len(), 2);

    for (i, indexed_blob) in indexed[0].iter().enumerate() {
        assert_eq!(indexed_blob.proof, proof_for(&blobs[i], &commitments[i]));
        assert_ne!(
            indexed_blob.proof, PLACEHOLDER_PROOF,
            "a real proof, not the infinity placeholder"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn gloas_payload_that_arrives_late_is_still_indexed() {
    let blobs = vec![blob(3), blob(4)];
    let commitments: Vec<_> = blobs.iter().map(commitment_of).collect();
    let (raw, block_commitments) = (blobs.clone(), commitments.clone());
    let mut beacon = MockCommonBeaconClient::new();

    beacon
        .expect_get_block()
        .returning(move |_| ready(Ok(Some(gloas_block(block_commitments.clone())))));
    beacon
        .expect_get_blobs()
        .returning(move |_| ready(Ok(Some(raw.clone()))));

    // The execution client answers `null` (unknown hash) a few times before the builder reveals.
    let mut responses = vec![Value::Null; 5];
    responses.push(execution_block_json(&commitments));
    let h = harness(beacon, responses);

    h.processor
        .index_block(B256::repeat_byte(0xaa))
        .await
        .unwrap();

    assert_eq!(h.indexed.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn gloas_payload_that_is_never_revealed_is_skipped_not_fatal() {
    let commitments = vec![commitment_of(&blob(5))];
    let mut beacon = MockCommonBeaconClient::new();

    beacon
        .expect_get_block()
        .returning(move |_| ready(Ok(Some(gloas_block(commitments.clone())))));
    beacon.expect_get_blobs().never();

    let h = harness(
        beacon,
        vec![Value::Null; retry_attempts_for(PAYLOAD_WAIT) as usize],
    );

    h.processor
        .index_block(B256::repeat_byte(0xaa))
        .await
        .expect("a withheld payload must not stop the indexer");

    assert!(h.indexed.lock().unwrap().is_empty());
    assert!(
        h.provider_asserter.read_q().is_empty(),
        "every attempt must have asked the execution client"
    );
}

#[test]
fn the_payload_wait_becomes_attempts_every_half_second() {
    assert_eq!(retry_attempts_for(Duration::from_secs(12)), 24);
    assert_eq!(retry_attempts_for(Duration::from_secs(5)), 10);
    assert_eq!(retry_attempts_for(Duration::from_millis(100)), 1);
    assert_eq!(retry_attempts_for(Duration::ZERO), 1);
}

/// A shorter wait (Gnosis has 5 second slots) gives up after fewer attempts.
#[tokio::test(start_paused = true)]
async fn gloas_payload_wait_is_configurable() {
    let commitments = vec![commitment_of(&blob(5))];
    let mut beacon = MockCommonBeaconClient::new();

    beacon
        .expect_get_block()
        .returning(move |_| ready(Ok(Some(gloas_block(commitments.clone())))));

    let wait = Duration::from_secs(5);
    let attempts = retry_attempts_for(wait) as usize;
    // One more answer than attempts: it must stay unread.
    let h = harness_with_payload_wait(beacon, vec![Value::Null; attempts + 1], wait);

    h.processor
        .index_block(B256::repeat_byte(0xaa))
        .await
        .unwrap();

    assert_eq!(attempts, 10);
    assert_eq!(h.provider_asserter.read_q().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn gloas_blob_that_does_not_match_its_commitment_is_rejected() {
    let blobs = vec![blob(6), blob(7)];
    let commitments: Vec<_> = blobs.iter().map(commitment_of).collect();
    // Same count, but the node returned the blobs of another block at this slot (a reorg).
    let other_block_blobs = vec![blob(8), blob(9)];
    let block_commitments = commitments.clone();
    let mut beacon = MockCommonBeaconClient::new();

    beacon
        .expect_get_block()
        .returning(move |_| ready(Ok(Some(gloas_block(block_commitments.clone())))));
    beacon
        .expect_get_blobs()
        .returning(move |_| ready(Ok(Some(other_block_blobs.clone()))));

    let h = harness(beacon, vec![execution_block_json(&commitments)]);

    let err = h
        .processor
        .index_block(B256::repeat_byte(0xaa))
        .await
        .expect_err("mismatched blobs must not be indexed");

    assert!(format!("{err:?}").contains("does not match"), "{err:?}");
    assert!(h.indexed.lock().unwrap().is_empty());
}

/// A pre-Gloas block gets its blobs from the same endpoint as a Gloas block, and its proofs are
/// computed too.
#[tokio::test(start_paused = true)]
async fn pre_gloas_block_is_indexed_with_computed_proofs() {
    let blobs = vec![blob(10), blob(11)];
    let commitments: Vec<_> = blobs.iter().map(commitment_of).collect();
    let (raw, block_commitments) = (blobs.clone(), commitments.clone());
    let mut beacon = MockCommonBeaconClient::new();

    beacon.expect_get_block().returning(move |_| {
        ready(Ok(Some(BeaconBlock {
            execution_payload: Some(BlockExecutionPayload::InBody {
                block_hash: B256::repeat_byte(0x07),
                block_number: 11942470,
            }),
            ..gloas_block(block_commitments.clone())
        })))
    });
    beacon
        .expect_get_blobs()
        .times(1)
        .returning(move |_| ready(Ok(Some(raw.clone()))));

    let h = harness(beacon, vec![execution_block_json(&commitments)]);

    h.processor
        .index_block(B256::repeat_byte(0xaa))
        .await
        .unwrap();

    let indexed = h.indexed.lock().unwrap();

    assert_eq!(indexed.len(), 1);

    for (i, indexed_blob) in indexed[0].iter().enumerate() {
        assert_eq!(indexed_blob.proof, proof_for(&blobs[i], &commitments[i]));
        assert_ne!(indexed_blob.proof, PLACEHOLDER_PROOF);
    }
}

/// The node gave fewer blobs than the block has commitments: the block at the slot changed.
#[tokio::test(start_paused = true)]
async fn blobs_that_do_not_match_the_commitment_count_are_rejected() {
    let blobs = vec![blob(12), blob(13)];
    let commitments: Vec<_> = blobs.iter().map(commitment_of).collect();
    let (raw, block_commitments) = (vec![blobs[0].clone()], commitments.clone());
    let mut beacon = MockCommonBeaconClient::new();

    beacon
        .expect_get_block()
        .returning(move |_| ready(Ok(Some(gloas_block(block_commitments.clone())))));
    beacon
        .expect_get_blobs()
        .returning(move |_| ready(Ok(Some(raw.clone()))));

    let h = harness(beacon, vec![execution_block_json(&commitments)]);

    let err = h
        .processor
        .index_block(B256::repeat_byte(0xaa))
        .await
        .expect_err("a different number of blobs must not be indexed");

    assert!(format!("{err:?}").contains("possibly reorged"), "{err:?}");
    assert!(h.indexed.lock().unwrap().is_empty());
}
