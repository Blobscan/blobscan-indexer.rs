//! `index_block` against mocked clients: the sidecar KZG proofs that beacon nodes return (the
//! infinity point since Fulu) are replaced with real ones, and real ones are kept.

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
            types::{Blob as BeaconBlob, Block as BeaconBlock, ExecutionPayload, KzgCommitment},
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

/// The real sepolia block 11942470, trimmed to two blob
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

fn beacon_block(commitments: Vec<KzgCommitment>) -> BeaconBlock {
    BeaconBlock {
        blob_kzg_commitments: Some(commitments),
        execution_payload: Some(ExecutionPayload {
            block_hash: B256::repeat_byte(0x07),
            block_number: 11942470,
        }),
        parent_root: B256::repeat_byte(0x01),
        slot: SLOT,
    }
}

struct Harness {
    processor: SlotsProcessor,
    indexed: Arc<Mutex<Vec<Vec<IndexedBlob>>>>,
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
    }
}

fn proof_for(blob: &HeapBlob, commitment: &KzgCommitment) -> B256x48 {
    compute_blob_proof(blob, commitment).unwrap()
}

/// A block whose sidecars carry `sidecar_proofs`, one per blob.
async fn index_block_with_sidecar_proofs(
    blobs: &[HeapBlob],
    sidecar_proofs: Vec<B256x48>,
) -> Vec<IndexedBlob> {
    let commitments: Vec<_> = blobs.iter().map(commitment_of).collect();
    let sidecars: Vec<_> = blobs
        .iter()
        .zip(&commitments)
        .zip(sidecar_proofs)
        .map(|((blob, commitment), proof)| (blob.clone(), *commitment, proof))
        .collect();
    let block_commitments = commitments.clone();
    let mut beacon = MockCommonBeaconClient::new();

    beacon
        .expect_get_block()
        .returning(move |_| ready(Ok(Some(beacon_block(block_commitments.clone())))));
    beacon.expect_get_blobs().returning(move |_| {
        ready(Ok(Some(
            sidecars
                .iter()
                .map(|(blob, commitment, proof)| BeaconBlob {
                    kzg_commitment: *commitment,
                    kzg_proof: *proof,
                    blob: blob.clone(),
                })
                .collect(),
        )))
    });

    let h = harness(beacon, vec![execution_block_json(&commitments)]);

    h.processor
        .index_block(B256::repeat_byte(0xaa))
        .await
        .unwrap();

    let indexed = h.indexed.lock().unwrap();

    assert_eq!(indexed.len(), 1);
    indexed[0].clone()
}

/// Since Fulu the beacon nodes answer sidecars with the infinity point instead of the proof.
#[tokio::test(start_paused = true)]
async fn placeholder_proofs_are_replaced_with_real_ones() {
    let blobs = vec![blob(10), blob(11)];
    let indexed = index_block_with_sidecar_proofs(&blobs, vec![PLACEHOLDER_PROOF; 2]).await;

    for (i, indexed_blob) in indexed.iter().enumerate() {
        assert_ne!(indexed_blob.proof, PLACEHOLDER_PROOF);
        assert_eq!(
            indexed_blob.proof,
            proof_for(&blobs[i], &commitment_of(&blobs[i]))
        );
    }
}

#[tokio::test(start_paused = true)]
async fn valid_proofs_are_kept() {
    let blobs = vec![blob(12), blob(13)];
    let given: Vec<_> = blobs
        .iter()
        .map(|b| proof_for(b, &commitment_of(b)))
        .collect();
    let indexed = index_block_with_sidecar_proofs(&blobs, given.clone()).await;

    assert_eq!(indexed.iter().map(|b| b.proof).collect::<Vec<_>>(), given);
}

/// Not only the infinity point: any proof that does not verify is replaced.
#[tokio::test(start_paused = true)]
async fn proofs_that_do_not_verify_are_replaced() {
    let blobs = vec![blob(12), blob(13)];
    let indexed =
        index_block_with_sidecar_proofs(&blobs, vec![B256x48::repeat_byte(0x11), B256x48::ZERO])
            .await;

    for (i, indexed_blob) in indexed.iter().enumerate() {
        assert_eq!(
            indexed_blob.proof,
            proof_for(&blobs[i], &commitment_of(&blobs[i]))
        );
    }
}

#[tokio::test(start_paused = true)]
async fn only_the_invalid_sidecar_proofs_are_replaced() {
    let blobs = vec![blob(14), blob(15)];
    let valid = proof_for(&blobs[0], &commitment_of(&blobs[0]));
    let indexed = index_block_with_sidecar_proofs(&blobs, vec![valid, PLACEHOLDER_PROOF]).await;

    assert_eq!(indexed[0].proof, valid);
    assert_eq!(
        indexed[1].proof,
        proof_for(&blobs[1], &commitment_of(&blobs[1]))
    );
}
