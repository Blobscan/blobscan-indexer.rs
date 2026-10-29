//! `index_block` and the reorg path against mocked clients. Blocks of all forks get their raw
//! blobs from `/eth/v1/beacon/blobs`, and the indexer computes the KZG proofs. A Gloas block
//! indexes the payload of its parent, if the parent is full.

use std::sync::{atomic::AtomicBool, Arc, Mutex};

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
            types::{Block as BeaconBlock, BlockExecutionPayload, KzgCommitment},
            MockCommonBeaconClient,
        },
        blobscan::{types::BlobscanBlock, MockCommonBlobscanClient},
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

const PARENT_ROOT: B256 = B256::repeat_byte(0xbb);
const CHILD_ROOT: B256 = B256::repeat_byte(0xcc);
const PAYLOAD_HASH: B256 = B256::repeat_byte(0x07);
const OLDER_PAYLOAD_HASH: B256 = B256::repeat_byte(0x06);

/// A Gloas block at `SLOT` whose bid commits to `PAYLOAD_HASH` with `commitments`.
fn gloas_block(commitments: Vec<KzgCommitment>) -> BeaconBlock {
    BeaconBlock {
        blob_kzg_commitments: Some(commitments),
        execution_payload: Some(BlockExecutionPayload::Bid {
            block_hash: PAYLOAD_HASH,
            parent_block_hash: OLDER_PAYLOAD_HASH,
        }),
        parent_root: B256::repeat_byte(0x01),
        slot: SLOT,
    }
}

/// A Gloas child of the block at `PARENT_ROOT` whose bid builds on `parent_block_hash`. Its own
/// commitments must never be indexed with it.
fn gloas_child(parent_block_hash: B256) -> BeaconBlock {
    BeaconBlock {
        blob_kzg_commitments: Some(vec![commitment_of(&blob(99))]),
        execution_payload: Some(BlockExecutionPayload::Bid {
            block_hash: B256::repeat_byte(0x08),
            parent_block_hash,
        }),
        parent_root: PARENT_ROOT,
        slot: SLOT + 1,
    }
}

fn pre_gloas_block(commitments: Vec<KzgCommitment>) -> BeaconBlock {
    BeaconBlock {
        execution_payload: Some(BlockExecutionPayload::InBody {
            block_hash: PAYLOAD_HASH,
            block_number: 11942470,
        }),
        ..gloas_block(commitments)
    }
}

/// Makes `get_block` answer with `blocks`, by block root. Other roots are not found.
fn expect_blocks(beacon: &mut MockCommonBeaconClient, blocks: Vec<(B256, BeaconBlock)>) {
    beacon.expect_get_block().returning(move |block_id| {
        let block = blocks
            .iter()
            .find(|(root, _)| BlockId::Hash(*root) == block_id)
            .map(|(_, block)| block.clone());

        ready(Ok(block))
    });
}

/// A full Gloas parent at `PARENT_ROOT` with `commitments`, and its child at `CHILD_ROOT`.
fn expect_full_parent(beacon: &mut MockCommonBeaconClient, commitments: Vec<KzgCommitment>) {
    expect_blocks(
        beacon,
        vec![
            (PARENT_ROOT, gloas_block(commitments)),
            (CHILD_ROOT, gloas_child(PAYLOAD_HASH)),
        ],
    );
}

struct Harness {
    processor: SlotsProcessor,
    indexed: Arc<Mutex<Vec<Vec<IndexedBlob>>>>,
    provider_asserter: Asserter,
}

fn harness(beacon: MockCommonBeaconClient, execution_responses: Vec<Value>) -> Harness {
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

/// The child builds on the payload of its parent, so the parent is full: the child indexes the
/// payload, the blobs and the commitments of the parent.
#[tokio::test(start_paused = true)]
async fn gloas_block_indexes_the_payload_of_its_full_parent() {
    let blobs = vec![blob(1), blob(2)];
    let commitments: Vec<_> = blobs.iter().map(commitment_of).collect();
    let raw = blobs.clone();
    let mut beacon = MockCommonBeaconClient::new();

    expect_full_parent(&mut beacon, commitments.clone());
    beacon
        .expect_get_blobs()
        .withf(|block_id| *block_id == BlockId::Slot(SLOT))
        .times(1)
        .returning(move |_| ready(Ok(Some(raw.clone()))));

    let h = harness(beacon, vec![execution_block_json(&commitments)]);

    h.processor.index_block(CHILD_ROOT).await.unwrap();

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

/// The child builds on an older payload, so the builder of the parent did not send its payload (or
/// it came too late). There is nothing to index, and the execution client is not asked.
#[tokio::test(start_paused = true)]
async fn gloas_block_with_an_empty_parent_indexes_nothing() {
    let commitments = vec![commitment_of(&blob(3))];
    let mut beacon = MockCommonBeaconClient::new();

    expect_blocks(
        &mut beacon,
        vec![
            (PARENT_ROOT, gloas_block(commitments)),
            (CHILD_ROOT, gloas_child(OLDER_PAYLOAD_HASH)),
        ],
    );
    beacon.expect_get_blobs().never();

    let h = harness(beacon, vec![]);

    h.processor.index_block(CHILD_ROOT).await.unwrap();

    assert!(h.indexed.lock().unwrap().is_empty());
}

/// The first Gloas block: the payload of its pre-Gloas parent was indexed with the parent.
#[tokio::test(start_paused = true)]
async fn first_gloas_block_does_not_index_its_pre_gloas_parent_again() {
    let commitments = vec![commitment_of(&blob(4))];
    let mut beacon = MockCommonBeaconClient::new();

    expect_blocks(
        &mut beacon,
        vec![
            (PARENT_ROOT, pre_gloas_block(commitments)),
            (CHILD_ROOT, gloas_child(PAYLOAD_HASH)),
        ],
    );
    beacon.expect_get_blobs().never();

    let h = harness(beacon, vec![]);

    h.processor.index_block(CHILD_ROOT).await.unwrap();

    assert!(h.indexed.lock().unwrap().is_empty());
}

/// The payload is part of the chain, but the execution client gets it a little after the beacon
/// node.
#[tokio::test(start_paused = true)]
async fn an_execution_client_that_is_a_little_behind_is_tried_again() {
    let blobs = vec![blob(5), blob(6)];
    let commitments: Vec<_> = blobs.iter().map(commitment_of).collect();
    let raw = blobs.clone();
    let mut beacon = MockCommonBeaconClient::new();

    expect_full_parent(&mut beacon, commitments.clone());
    beacon
        .expect_get_blobs()
        .returning(move |_| ready(Ok(Some(raw.clone()))));

    let mut responses = vec![Value::Null; RETRY_MAX_ATTEMPTS as usize - 1];
    responses.push(execution_block_json(&commitments));
    let h = harness(beacon, responses);

    h.processor.index_block(CHILD_ROOT).await.unwrap();

    assert_eq!(h.indexed.lock().unwrap().len(), 1);
}

/// The child shows that the payload is part of the chain. If the execution client does not know
/// it, the execution client is behind: this is an error, so that the block is tried again. It
/// must not be skipped as a payload that the builder did not send.
#[tokio::test(start_paused = true)]
async fn a_full_payload_that_the_execution_client_does_not_know_is_an_error() {
    let commitments = vec![commitment_of(&blob(7))];
    let mut beacon = MockCommonBeaconClient::new();

    expect_full_parent(&mut beacon, commitments);
    beacon.expect_get_blobs().never();

    let h = harness(beacon, vec![Value::Null; RETRY_MAX_ATTEMPTS as usize]);

    let err = h
        .processor
        .index_block(CHILD_ROOT)
        .await
        .expect_err("the block must not be skipped");

    assert!(format!("{err:?}").contains("not found"), "{err:?}");
    assert!(h.indexed.lock().unwrap().is_empty());
    assert!(
        h.provider_asserter.read_q().is_empty(),
        "every attempt must have asked the execution client"
    );
}

#[tokio::test(start_paused = true)]
async fn gloas_blob_that_does_not_match_its_commitment_is_rejected() {
    let blobs = [blob(8), blob(9)];
    let commitments: Vec<_> = blobs.iter().map(commitment_of).collect();
    // Same count, but the node returned the blobs of another block at this slot (a reorg).
    let other_block_blobs = vec![blob(10), blob(11)];
    let mut beacon = MockCommonBeaconClient::new();

    expect_full_parent(&mut beacon, commitments.clone());
    beacon
        .expect_get_blobs()
        .returning(move |_| ready(Ok(Some(other_block_blobs.clone()))));

    let h = harness(beacon, vec![execution_block_json(&commitments)]);

    let err = h
        .processor
        .index_block(CHILD_ROOT)
        .await
        .expect_err("mismatched blobs must not be indexed");

    assert!(format!("{err:?}").contains("does not match"), "{err:?}");
    assert!(h.indexed.lock().unwrap().is_empty());
}

/// A pre-Gloas block indexes its own payload. It gets its blobs from the same endpoint as a
/// Gloas block, and its proofs are computed too.
#[tokio::test(start_paused = true)]
async fn pre_gloas_block_is_indexed_with_computed_proofs() {
    let blobs = vec![blob(12), blob(13)];
    let commitments: Vec<_> = blobs.iter().map(commitment_of).collect();
    let raw = blobs.clone();
    let mut beacon = MockCommonBeaconClient::new();

    expect_blocks(
        &mut beacon,
        vec![(CHILD_ROOT, pre_gloas_block(commitments.clone()))],
    );
    beacon
        .expect_get_blobs()
        .times(1)
        .returning(move |_| ready(Ok(Some(raw.clone()))));

    let h = harness(beacon, vec![execution_block_json(&commitments)]);

    h.processor.index_block(CHILD_ROOT).await.unwrap();

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
    let blobs = [blob(14), blob(15)];
    let commitments: Vec<_> = blobs.iter().map(commitment_of).collect();
    let raw = vec![blobs[0].clone()];
    let mut beacon = MockCommonBeaconClient::new();

    expect_blocks(
        &mut beacon,
        vec![(CHILD_ROOT, pre_gloas_block(commitments.clone()))],
    );
    beacon
        .expect_get_blobs()
        .returning(move |_| ready(Ok(Some(raw.clone()))));

    let h = harness(beacon, vec![execution_block_json(&commitments)]);

    let err = h
        .processor
        .index_block(CHILD_ROOT)
        .await
        .expect_err("a different number of blobs must not be indexed");

    assert!(format!("{err:?}").contains("possibly reorged"), "{err:?}");
    assert!(h.indexed.lock().unwrap().is_empty());
}

/// A Gloas chain from slot 10 to slot 13. A is full (B builds on its payload), B is empty (C
/// builds on the payload of A), and C is full (H builds on its payload).
fn gloas_chain() -> Vec<(B256, BeaconBlock)> {
    let block = |slot: u32, parent: u8, hash: u8, parent_hash: u8| BeaconBlock {
        blob_kzg_commitments: None,
        execution_payload: Some(BlockExecutionPayload::Bid {
            block_hash: B256::repeat_byte(hash),
            parent_block_hash: B256::repeat_byte(parent_hash),
        }),
        parent_root: B256::repeat_byte(parent),
        slot,
    };

    vec![
        (B256::repeat_byte(0xa0), block(10, 0x90, 0x0a, 0x09)),
        (B256::repeat_byte(0xb0), block(11, 0xa0, 0x0b, 0x0a)),
        (B256::repeat_byte(0xc0), block(12, 0xb0, 0x0c, 0x0a)),
        (B256::repeat_byte(0xd0), block(13, 0xc0, 0x0d, 0x0c)),
    ]
}

async fn canonical_path_to(blobscan_block: BlobscanBlock) -> Option<Vec<(B256, B256)>> {
    let mut beacon = MockCommonBeaconClient::new();

    expect_blocks(&mut beacon, gloas_chain());

    let h = harness(beacon, vec![]);

    h.processor
        .get_canonical_block_path(&blobscan_block, B256::repeat_byte(0xd0))
        .await
        .unwrap()
        .map(|path| {
            path.iter()
                .map(|block| (block.root, block.execution_block_hash))
                .collect()
        })
}

/// The path has only the payloads that became part of the chain: C's payload (with H), not B's.
#[tokio::test]
async fn the_reorg_path_has_only_the_payloads_of_full_blocks() {
    let path = canonical_path_to(BlobscanBlock {
        hash: B256::repeat_byte(0x0a),
        number: 1,
        slot: 10,
    })
    .await;

    assert_eq!(
        path,
        Some(vec![(B256::repeat_byte(0xd0), B256::repeat_byte(0x0c))])
    );
}

/// B is empty, so its payload is not on the chain and is not a common ancestor.
#[tokio::test]
async fn the_payload_of_an_empty_block_is_not_a_common_ancestor() {
    let path = canonical_path_to(BlobscanBlock {
        hash: B256::repeat_byte(0x0b),
        number: 2,
        slot: 11,
    })
    .await;

    assert_eq!(path, None);
}

/// The head makes the payload of the Blobscan block part of the chain: it is the common ancestor,
/// and nothing must be forwarded.
#[tokio::test]
async fn the_payload_confirmed_by_the_head_is_a_common_ancestor() {
    let path = canonical_path_to(BlobscanBlock {
        hash: B256::repeat_byte(0x0c),
        number: 3,
        slot: 12,
    })
    .await;

    assert_eq!(path, Some(vec![]));
}

/// A block before Gloas does not need its parent to find its payload. Thus the match is found
/// also when the beacon node does not have the parent (for example, a checkpoint-synced node).
#[tokio::test]
async fn a_pre_gloas_match_does_not_need_the_parent() {
    let head_root = B256::repeat_byte(0xe0);
    let mut beacon = MockCommonBeaconClient::new();

    // The parent of the head is not in the list, so the beacon node does not find it.
    expect_blocks(&mut beacon, vec![(head_root, pre_gloas_block(vec![]))]);

    let h = harness(beacon, vec![]);

    let path = h
        .processor
        .get_canonical_block_path(
            &BlobscanBlock {
                hash: PAYLOAD_HASH,
                number: 11942470,
                slot: SLOT,
            },
            head_root,
        )
        .await
        .unwrap();

    assert_eq!(path.map(|path| path.len()), Some(0));
}

/// Indexes the blocks of `gloas_chain` at `roots`, in that order, and returns how many blocks
/// the processor got from the beacon node.
async fn beacon_requests_to_index(roots: &[u8]) -> usize {
    let requests = Arc::new(Mutex::new(0));
    let counter = requests.clone();
    let blocks = gloas_chain();
    let mut beacon = MockCommonBeaconClient::new();

    beacon.expect_get_block().returning(move |block_id| {
        *counter.lock().unwrap() += 1;

        let block = blocks
            .iter()
            .find(|(root, _)| BlockId::Hash(*root) == block_id)
            .map(|(_, block)| block.clone());

        ready(Ok(block))
    });

    let h = harness(beacon, vec![]);

    for root in roots {
        h.processor
            .index_block(B256::repeat_byte(*root))
            .await
            .unwrap();
    }

    let count = *requests.lock().unwrap();

    count
}

/// Forward: the parent of each block is the block before it, so the processor gets it only one
/// time.
#[tokio::test]
async fn going_forward_gets_each_block_one_time() {
    assert_eq!(beacon_requests_to_index(&[0xb0, 0xc0, 0xd0]).await, 4);
}

/// Back: the next block is the parent of the block before it, so the processor gets it only one
/// time.
#[tokio::test]
async fn going_back_gets_each_block_one_time() {
    assert_eq!(beacon_requests_to_index(&[0xd0, 0xc0, 0xb0]).await, 4);
}

/// A Gloas block for the reorg tests, with no blobs: `root`, `slot` and `parent` root, and the
/// `hash` and `parent_hash` of its bid. Each byte stands for a 32-byte value.
fn bid_block(root: u8, slot: u32, parent: u8, hash: u8, parent_hash: u8) -> (B256, BeaconBlock) {
    (
        B256::repeat_byte(root),
        BeaconBlock {
            blob_kzg_commitments: None,
            execution_payload: Some(BlockExecutionPayload::Bid {
                block_hash: B256::repeat_byte(hash),
                parent_block_hash: B256::repeat_byte(parent_hash),
            }),
            parent_root: B256::repeat_byte(parent),
            slot,
        },
    )
}

fn header(root: u8, parent: u8, slot: u32) -> BlockHeader {
    BlockHeader {
        root: B256::repeat_byte(root),
        parent_root: B256::repeat_byte(parent),
        slot,
    }
}

/// Runs `process_block_header` for `new_head`, after the processor processed `old_head`.
/// Blobscan has the payloads in `blobscan_blocks`, as (slot, hash). Returns the calls to
/// `handle_reorg`, as (rewound, forwarded).
async fn handle_reorg_calls(
    chain: Vec<(B256, BeaconBlock)>,
    blobscan_blocks: Vec<(u32, u8)>,
    old_head: BlockHeader,
    new_head: BlockHeader,
) -> Vec<(Vec<B256>, Vec<B256>)> {
    let mut beacon = MockCommonBeaconClient::new();
    let mut blobscan = MockCommonBlobscanClient::new();
    let calls = Arc::new(Mutex::new(vec![]));
    let sink = calls.clone();

    expect_blocks(&mut beacon, chain);
    blobscan.expect_get_block().returning(move |slot| {
        let block = blobscan_blocks
            .iter()
            .find(|(s, _)| *s == slot)
            .map(|(slot, hash)| BlobscanBlock {
                hash: B256::repeat_byte(*hash),
                number: *slot,
                slot: *slot,
            });

        ready(Ok(block))
    });
    blobscan
        .expect_handle_reorg()
        .returning(move |rewound, forwarded| {
            sink.lock().unwrap().push((rewound, forwarded));

            ready(Ok(()))
        });
    // The blocks in these tests have no blobs, so nothing is indexed.
    blobscan.expect_index().never();

    let context = TestContext {
        beacon: Arc::new(beacon),
        blobscan: Arc::new(blobscan),
        provider: Arc::new(ProviderBuilder::new().connect_mocked_client(Asserter::new())),
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
    };
    let mut processor = SlotsProcessor::new(Box::new(context), Some(old_head));

    processor
        .process_block_header(new_head, true)
        .await
        .unwrap();

    let calls = calls.lock().unwrap().clone();

    calls
}

/// The old chain is A <- B <- C (old head), and B builds on the payload of A. Blobscan has the
/// payloads of A (slot 10) and B (slot 11).
///
/// The new chain is A <- B <- C1 <- D1 <- E1 (new head). C1 builds on the payload of A, so B is
/// empty on the new chain. D1 and E1 make C1 and D1 full.
///
/// The payload of B must be rewound, and the payloads of C1 and D1 must be forwarded, the
/// oldest first.
#[tokio::test(start_paused = true)]
async fn a_reorg_that_empties_a_block_rewinds_its_payload() {
    let chain = vec![
        bid_block(0xa0, 10, 0x90, 0x0a, 0x09),
        bid_block(0xb0, 11, 0xa0, 0x0b, 0x0a),
        bid_block(0xc0, 12, 0xb0, 0x0c, 0x0b),
        bid_block(0xc1, 12, 0xb0, 0x1c, 0x0a),
        bid_block(0xd1, 13, 0xc1, 0x1d, 0x1c),
        bid_block(0xe1, 14, 0xd1, 0x1e, 0x1d),
    ];

    let calls = handle_reorg_calls(
        chain,
        vec![(10, 0x0a), (11, 0x0b)],
        header(0xc0, 0xb0, 12),
        header(0xe1, 0xd1, 14),
    )
    .await;

    assert_eq!(
        calls,
        vec![(
            vec![B256::repeat_byte(0x0b)],
            vec![B256::repeat_byte(0x1c), B256::repeat_byte(0x1d)]
        )]
    );
}

/// The old head C and the new head C2 are children of B, and both build on the payload of B.
/// Blobscan already has that payload, so there is nothing to rewind or forward.
#[tokio::test(start_paused = true)]
async fn a_reorg_to_a_sibling_that_keeps_the_parent_full_does_nothing() {
    let chain = vec![
        bid_block(0xa0, 10, 0x90, 0x0a, 0x09),
        bid_block(0xb0, 11, 0xa0, 0x0b, 0x0a),
        bid_block(0xc0, 12, 0xb0, 0x0c, 0x0b),
        bid_block(0xc2, 12, 0xb0, 0x2c, 0x0b),
    ];

    let calls = handle_reorg_calls(
        chain,
        vec![(10, 0x0a), (11, 0x0b)],
        header(0xc0, 0xb0, 12),
        header(0xc2, 0xb0, 12),
    )
    .await;

    assert!(calls.is_empty(), "{calls:?}");
}

/// A route memory of its own, so tests do not share the process wide one.
fn route_memory(raw_first: bool) -> &'static AtomicBool {
    Box::leak(Box::new(AtomicBool::new(raw_first)))
}

/// Mocks for one pre-Gloas block with two blobs, and a harness that uses `memory`.
fn route_harness(
    memory: &'static AtomicBool,
    mock_routes: impl FnOnce(&mut MockCommonBeaconClient),
) -> (Harness, Vec<HeapBlob>, Vec<KzgCommitment>) {
    let blobs = vec![blob(21), blob(22)];
    let commitments: Vec<_> = blobs.iter().map(commitment_of).collect();
    let mut beacon = MockCommonBeaconClient::new();

    expect_blocks(
        &mut beacon,
        vec![(CHILD_ROOT, pre_gloas_block(commitments.clone()))],
    );
    mock_routes(&mut beacon);

    let mut h = harness(beacon, vec![execution_block_json(&commitments)]);

    h.processor = h.processor.with_route_memory(memory);

    (h, blobs, commitments)
}

fn assert_indexed_with_real_proofs(h: &Harness, blobs: &[HeapBlob], commitments: &[KzgCommitment]) {
    let indexed = h.indexed.lock().unwrap();

    assert_eq!(indexed.len(), 1, "the block must be indexed exactly once");
    assert_eq!(indexed[0].len(), 2);

    for (i, indexed_blob) in indexed[0].iter().enumerate() {
        assert_eq!(indexed_blob.proof, proof_for(&blobs[i], &commitments[i]));
    }
}

/// The node answers 404 on `/blobs` (a gnosis node): the sidecars route gives the blobs, and the
/// indexer asks that route first from then on.
#[tokio::test(start_paused = true)]
async fn blobs_route_404_falls_back_to_sidecars_and_remembers_it() {
    let memory = route_memory(true);
    let raw = vec![blob(21), blob(22)];
    let (h, blobs, commitments) = route_harness(memory, |beacon| {
        beacon
            .expect_get_blobs()
            .times(1)
            .returning(|_| ready(Ok(None)));
        beacon
            .expect_get_blob_sidecar_blobs()
            .times(1)
            .returning(move |_| ready(Ok(Some(raw.clone()))));
    });

    h.processor.index_block(CHILD_ROOT).await.unwrap();

    assert_indexed_with_real_proofs(&h, &blobs, &commitments);
    assert!(!memory.load(std::sync::atomic::Ordering::Relaxed));
}

/// With the memory on sidecars, the `/blobs` route is not asked at all while sidecars work.
#[tokio::test(start_paused = true)]
async fn remembered_sidecars_route_is_asked_first() {
    let memory = route_memory(false);
    let raw = vec![blob(21), blob(22)];
    let (h, blobs, commitments) = route_harness(memory, |beacon| {
        beacon.expect_get_blobs().never();
        beacon
            .expect_get_blob_sidecar_blobs()
            .times(1)
            .returning(move |_| ready(Ok(Some(raw.clone()))));
    });

    h.processor.index_block(CHILD_ROOT).await.unwrap();

    assert_indexed_with_real_proofs(&h, &blobs, &commitments);
    assert!(!memory.load(std::sync::atomic::Ordering::Relaxed));
}

/// A node that serves `/blobs` never gets a sidecars request (a sepolia node answers 404 there).
#[tokio::test(start_paused = true)]
async fn working_blobs_route_does_not_touch_sidecars() {
    let memory = route_memory(true);
    let raw = vec![blob(21), blob(22)];
    let (h, blobs, commitments) = route_harness(memory, |beacon| {
        beacon
            .expect_get_blobs()
            .times(1)
            .returning(move |_| ready(Ok(Some(raw.clone()))));
        beacon.expect_get_blob_sidecar_blobs().never();
    });

    h.processor.index_block(CHILD_ROOT).await.unwrap();

    assert_indexed_with_real_proofs(&h, &blobs, &commitments);
    assert!(memory.load(std::sync::atomic::Ordering::Relaxed));
}

/// The remembered route stops working (the node changed): the other one takes over.
#[tokio::test(start_paused = true)]
async fn remembered_route_that_stops_working_switches_back() {
    let memory = route_memory(false);
    let raw = vec![blob(21), blob(22)];
    let (h, blobs, commitments) = route_harness(memory, |beacon| {
        beacon
            .expect_get_blob_sidecar_blobs()
            .times(1)
            .returning(|_| ready(Ok(None)));
        beacon
            .expect_get_blobs()
            .times(1)
            .returning(move |_| ready(Ok(Some(raw.clone()))));
    });

    h.processor.index_block(CHILD_ROOT).await.unwrap();

    assert_indexed_with_real_proofs(&h, &blobs, &commitments);
    assert!(memory.load(std::sync::atomic::Ordering::Relaxed));
}

/// Neither route has the blobs: the fetch retries both, then fails and indexes nothing.
#[tokio::test(start_paused = true)]
async fn both_routes_without_blobs_fail_after_retrying_both() {
    let memory = route_memory(true);
    let (h, _, _) = route_harness(memory, |beacon| {
        beacon
            .expect_get_blobs()
            .times(RETRY_MAX_ATTEMPTS as usize)
            .returning(|_| ready(Ok(None)));
        beacon
            .expect_get_blob_sidecar_blobs()
            .times(RETRY_MAX_ATTEMPTS as usize)
            .returning(|_| ready(Ok(None)));
    });

    let err = h
        .processor
        .index_block(CHILD_ROOT)
        .await
        .expect_err("no blobs anywhere");

    assert!(format!("{err:?}").contains("Blobs not found"), "{err:?}");
    assert!(h.indexed.lock().unwrap().is_empty());
}
