use alloy::eips::eip4844::HeapBlob;
use anyhow::{anyhow, Context, Result};
use c_kzg::{ethereum_kzg_settings, Blob as KzgBlob, Bytes48 as KzgBytes48};

use crate::clients::beacon::types::{Blob as BeaconBlob, KzgCommitment, Proof};

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

/// Computes and verifies the KZG proofs of `blobs`, and returns each blob with its commitment
/// and its proof, in the same order.
///
/// All the blobs are processed one after the other in one blocking task. The slots processors
/// run in parallel, so this keeps the number of CPU-bound tasks at the same time equal to the
/// slots concurrency. The function stops at the first error.
pub async fn compute_blob_proofs(
    blobs: Vec<HeapBlob>,
    commitments: Vec<KzgCommitment>,
) -> Result<Vec<BeaconBlob>> {
    if blobs.len() != commitments.len() {
        return Err(anyhow!(
            "{} blobs but {} KZG commitments",
            blobs.len(),
            commitments.len()
        ));
    }

    tokio::task::spawn_blocking(move || {
        blobs
            .into_iter()
            .zip(commitments)
            .enumerate()
            .map(|(i, (blob, kzg_commitment))| {
                let kzg_proof = compute_blob_proof(&blob, &kzg_commitment)
                    .with_context(|| format!("Blob {i}"))?;

                Ok(BeaconBlob {
                    kzg_commitment,
                    kzg_proof,
                    blob,
                })
            })
            .collect()
    })
    .await?
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
        let commitments: Vec<_> = blobs.iter().map(commitment_of).collect();

        let sidecars = compute_blob_proofs(blobs.clone(), commitments.clone())
            .await
            .unwrap();

        assert_eq!(sidecars.len(), blobs.len());

        for ((sidecar, blob), commitment) in sidecars.iter().zip(&blobs).zip(&commitments) {
            assert_eq!(sidecar.blob, *blob);
            assert_eq!(sidecar.kzg_commitment, *commitment);
            assert_eq!(
                sidecar.kzg_proof,
                compute_blob_proof(blob, commitment).unwrap()
            );
        }
    }

    #[tokio::test]
    async fn a_failing_blob_fails_the_batch_and_names_the_blob() {
        let good = test_blob(1);
        let other = test_blob(2);

        let err = compute_blob_proofs(
            vec![good.clone(), good.clone()],
            vec![commitment_of(&good), commitment_of(&other)],
        )
        .await
        .unwrap_err();

        assert!(format!("{err:#}").contains("Blob 1"), "{err:#}");
    }

    #[tokio::test]
    async fn a_different_number_of_blobs_and_commitments_is_an_error() {
        let blob = test_blob(1);

        let result = compute_blob_proofs(vec![blob.clone()], vec![]).await;

        assert!(result.is_err());
    }
}
