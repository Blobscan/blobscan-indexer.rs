use alloy::eips::eip4844::HeapBlob;
use anyhow::{anyhow, Context, Result};
use c_kzg::{ethereum_kzg_settings, Blob as KzgBlob, Bytes48 as KzgBytes48};
use tokio::task::JoinSet;

use crate::clients::beacon::types::{KzgCommitment, Proof};

/// The compressed point at infinity: `0xc0` followed by 47 zero bytes.
///
/// Since Fulu, beacon nodes rebuild blob sidecars from PeerDAS data. They cannot recover the KZG
/// proof of a single blob, so the sidecars give this value and not a proof. The tests use it to
/// make sure that the indexer never stores it.
#[cfg(test)]
pub const PLACEHOLDER_PROOF: Proof = {
    let mut bytes = [0u8; 48];
    bytes[0] = 0xc0;

    Proof::new(bytes)
};

/// Computes the KZG blob proof of `blob` for `commitment`, and checks that it verifies.
///
/// The check also makes sure that the blob is the blob of `commitment`. If it is not (for example,
/// a blob of a reorged block at the same slot), the proof does not verify and this returns an
/// error.
///
/// `/eth/v1/beacon/blobs` gives the raw blobs only. Blobscan stores and serves the proof, and the
/// proof is a function of the blob and its commitment only, so the indexer computes it.
pub fn compute_blob_proof(blob: &HeapBlob, commitment: &KzgCommitment) -> Result<Proof> {
    let kzg_blob = KzgBlob::from_bytes(blob.inner().as_ref())
        .map_err(|err| anyhow!("Invalid blob for KZG proof computation: {err}"))?;
    let kzg_commitment = KzgBytes48::from_bytes(commitment.as_ref())
        .map_err(|err| anyhow!("Invalid KZG commitment: {err}"))?;

    let proof = ethereum_kzg_settings(0)
        .compute_blob_kzg_proof(&kzg_blob, &kzg_commitment)
        .map_err(|err| anyhow!("Failed to compute the KZG blob proof: {err}"))?;

    let proof_bytes = proof.to_bytes();
    let is_valid = ethereum_kzg_settings(0)
        .verify_blob_kzg_proof(&kzg_blob, &kzg_commitment, &proof_bytes)
        .map_err(|err| anyhow!("Failed to verify the KZG blob proof: {err}"))?;

    if !is_valid {
        return Err(anyhow!("The blob does not match its KZG commitment"));
    }

    Ok(Proof::from_slice(proof_bytes.as_ref()))
}

/// Computes (and verifies) the proofs of many blobs in parallel, in the same order as `items`.
///
/// One proof takes about 45 ms of CPU in a release build and a block can carry 21 blobs, so doing
/// them one after the other would hold the indexer for about a second per block.
pub async fn compute_blob_proofs(items: Vec<(HeapBlob, KzgCommitment)>) -> Result<Vec<Proof>> {
    let mut tasks = JoinSet::new();

    for (i, (blob, commitment)) in items.into_iter().enumerate() {
        tasks.spawn_blocking(move || {
            compute_blob_proof(&blob, &commitment)
                .map(|proof| (i, proof))
                .with_context(|| format!("Blob {i}"))
        });
    }

    let mut proofs = Vec::with_capacity(tasks.len());

    while let Some(result) = tasks.join_next().await {
        proofs.push(result??);
    }

    proofs.sort_by_key(|(i, _)| *i);

    Ok(proofs.into_iter().map(|(_, proof)| proof).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A blob of valid field elements: every 32-byte element starts with a zero byte, which keeps
    /// it below the BLS modulus.
    fn test_blob(seed: u8) -> HeapBlob {
        let mut bytes = vec![0u8; 131_072];

        for (i, chunk) in bytes.chunks_mut(32).enumerate() {
            chunk[1] = seed;
            chunk[2] = (i % 251) as u8;
            chunk[31] = (i / 251) as u8;
        }

        HeapBlob::new(&bytes).unwrap()
    }

    #[test]
    fn computed_proof_verifies_against_the_commitment() {
        let blob = test_blob(7);
        let settings = ethereum_kzg_settings(0);
        let kzg_blob = KzgBlob::from_bytes(blob.inner().as_ref()).unwrap();
        let commitment_bytes = settings
            .blob_to_kzg_commitment(&kzg_blob)
            .unwrap()
            .to_bytes();
        let commitment = KzgCommitment::from_slice(commitment_bytes.as_ref());

        let proof = compute_blob_proof(&blob, &commitment).unwrap();
        let proof_bytes = KzgBytes48::from_bytes(proof.as_ref()).unwrap();

        assert!(settings
            .verify_blob_kzg_proof(&kzg_blob, &commitment_bytes, &proof_bytes)
            .unwrap());
    }

    #[test]
    fn proof_is_deterministic_and_depends_on_the_blob() {
        let settings = ethereum_kzg_settings(0);
        let commitment_of = |blob: &HeapBlob| {
            let kzg_blob = KzgBlob::from_bytes(blob.inner().as_ref()).unwrap();

            KzgCommitment::from_slice(
                settings
                    .blob_to_kzg_commitment(&kzg_blob)
                    .unwrap()
                    .to_bytes()
                    .as_ref(),
            )
        };
        let (a, b) = (test_blob(1), test_blob(2));

        let proof_a = compute_blob_proof(&a, &commitment_of(&a)).unwrap();

        assert_eq!(proof_a, compute_blob_proof(&a, &commitment_of(&a)).unwrap());
        assert_ne!(proof_a, compute_blob_proof(&b, &commitment_of(&b)).unwrap());
    }

    #[test]
    fn rejects_a_blob_that_does_not_belong_to_the_commitment() {
        let settings = ethereum_kzg_settings(0);
        let (a, b) = (test_blob(1), test_blob(2));
        let commitment_of_b = KzgCommitment::from_slice(
            settings
                .blob_to_kzg_commitment(&KzgBlob::from_bytes(b.inner().as_ref()).unwrap())
                .unwrap()
                .to_bytes()
                .as_ref(),
        );

        let err = compute_blob_proof(&a, &commitment_of_b).unwrap_err();

        assert!(err.to_string().contains("does not match"), "{err}");
    }

    #[test]
    fn rejects_a_blob_with_an_out_of_range_field_element() {
        // 0xff..ff is above the BLS12-381 scalar modulus, so it is not a valid field element.
        let blob = HeapBlob::new(&vec![0xffu8; 131_072]).unwrap();

        assert!(compute_blob_proof(&blob, &KzgCommitment::ZERO).is_err());
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

    #[test]
    fn the_placeholder_is_the_compressed_point_at_infinity() {
        let hex = format!("0xc0{}", "00".repeat(47));

        assert_eq!(PLACEHOLDER_PROOF, hex.parse::<Proof>().unwrap());
    }

    #[tokio::test]
    async fn proofs_come_back_in_input_order() {
        let blobs: Vec<_> = (1..=5).map(test_blob).collect();
        let items: Vec<_> = blobs
            .iter()
            .map(|b| (b.clone(), commitment_of(b)))
            .collect();

        let proofs = compute_blob_proofs(items.clone()).await.unwrap();

        for ((blob, commitment), proof) in items.iter().zip(&proofs) {
            assert_eq!(*proof, compute_blob_proof(blob, commitment).unwrap());
        }
    }

    #[tokio::test]
    async fn a_failing_blob_fails_the_batch_and_names_the_blob() {
        let good = test_blob(1);
        let other = test_blob(2);
        let items = vec![
            (good.clone(), commitment_of(&good)),
            (good.clone(), commitment_of(&other)),
        ];

        let err = compute_blob_proofs(items).await.unwrap_err();

        assert!(format!("{err:#}").contains("Blob 1"), "{err:#}");
    }
}
