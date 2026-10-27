use alloy::eips::eip4844::HeapBlob;
use anyhow::{anyhow, Context, Result};
use c_kzg::{ethereum_kzg_settings, Blob as KzgBlob, Bytes48 as KzgBytes48};

use crate::clients::beacon::types::{Blob as BeaconBlob, KzgCommitment, Proof};

/// The compressed point at infinity: `0xc0` followed by 47 zero bytes.
///
/// Since Fulu, beacon nodes rebuild blob sidecars from PeerDAS data and cannot recover the
/// single-blob KZG proof, so `kzg_proof` comes back as this value instead of a proof. It is
/// stored as-is by Blobscan unless the indexer replaces it.
pub const PLACEHOLDER_PROOF: Proof = {
    let mut bytes = [0u8; 48];
    bytes[0] = 0xc0;

    Proof::new(bytes)
};

pub fn is_placeholder_proof(proof: &Proof) -> bool {
    *proof == PLACEHOLDER_PROOF
}

/// Computes the KZG blob proof of `blob` for `commitment`, and checks that it verifies.
///
/// The check doubles as a match test: a blob that does not belong to `commitment` (a reorged
/// block at the same slot, say) yields a proof that does not verify, and this returns an error
/// instead of attaching the blob to the wrong commitment.
///
/// Blobscan stores and serves the proof, and it is a pure function of the blob and its
/// commitment, so it can be recomputed when a beacon node does not return a real one.
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

/// Whether `proof` is a valid KZG proof of `blob` for `commitment`. A proof that cannot even be
/// decoded or checked (wrong point encoding, out-of-range blob) is simply not valid.
pub fn verify_blob_proof(blob: &HeapBlob, commitment: &KzgCommitment, proof: &Proof) -> bool {
    let (Ok(kzg_blob), Ok(kzg_commitment), Ok(kzg_proof)) = (
        KzgBlob::from_bytes(blob.inner().as_ref()),
        KzgBytes48::from_bytes(commitment.as_ref()),
        KzgBytes48::from_bytes(proof.as_ref()),
    ) else {
        return false;
    };

    ethereum_kzg_settings(0)
        .verify_blob_kzg_proof(&kzg_blob, &kzg_commitment, &kzg_proof)
        .unwrap_or(false)
}

/// Makes sure every sidecar carries a valid KZG proof, replacing the ones that do not. Returns
/// how many were replaced.
///
/// A proof is kept only if it verifies against its blob and commitment, which costs about 1.5 ms
/// per blob, against about 40 ms to compute one. Everything else is recomputed: the infinity
/// point that beacon nodes return since Fulu, any other dummy a node or proxy may use, or plain
/// garbage. The check is deliberately "does it verify" and not "does it equal the infinity
/// point", so it does not depend on knowing every placeholder there is.
///
/// All the blobs are processed one after the other in one blocking task. The slots processors
/// run in parallel, so this keeps the number of CPU-bound tasks at the same time equal to the
/// slots concurrency. The function also stops at the first error.
pub async fn replace_invalid_proofs(
    mut blobs: Vec<BeaconBlob>,
) -> Result<(Vec<BeaconBlob>, usize)> {
    tokio::task::spawn_blocking(move || {
        let mut replaced = 0;

        for (i, sidecar) in blobs.iter_mut().enumerate() {
            // The infinity point is by far the common case: skip the verification for it.
            if !is_placeholder_proof(&sidecar.kzg_proof)
                && verify_blob_proof(&sidecar.blob, &sidecar.kzg_commitment, &sidecar.kzg_proof)
            {
                continue;
            }

            let proof = compute_blob_proof(&sidecar.blob, &sidecar.kzg_commitment)
                .with_context(|| format!("Blob {i}"))?;

            // The infinity point is the correct proof of a constant polynomial (for example, an
            // all-zero blob). Count only the proofs that change.
            if proof != sidecar.kzg_proof {
                sidecar.kzg_proof = proof;
                replaced += 1;
            }
        }

        Ok((blobs, replaced))
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

    fn sidecar(seed: u8, proof: Proof) -> BeaconBlob {
        let blob = test_blob(seed);

        BeaconBlob {
            kzg_commitment: commitment_of(&blob),
            kzg_proof: proof,
            blob,
        }
    }

    #[test]
    fn the_placeholder_is_the_compressed_point_at_infinity() {
        let hex = format!("0xc0{}", "00".repeat(47));

        assert_eq!(PLACEHOLDER_PROOF, hex.parse::<Proof>().unwrap());
        assert!(is_placeholder_proof(&PLACEHOLDER_PROOF));
        assert!(!is_placeholder_proof(&Proof::ZERO));
    }

    #[tokio::test]
    async fn only_the_invalid_proofs_are_replaced() {
        let real = compute_blob_proof(&test_blob(3), &commitment_of(&test_blob(3))).unwrap();
        let blobs = vec![
            sidecar(1, PLACEHOLDER_PROOF),
            sidecar(3, real),
            sidecar(2, PLACEHOLDER_PROOF),
        ];

        let (blobs, replaced) = replace_invalid_proofs(blobs).await.unwrap();

        assert_eq!(replaced, 2);
        assert_eq!(blobs[1].kzg_proof, real, "a real proof must not be touched");

        for blob in &blobs {
            assert!(!is_placeholder_proof(&blob.kzg_proof));
            assert_eq!(
                blob.kzg_proof,
                compute_blob_proof(&blob.blob, &blob.kzg_commitment).unwrap()
            );
        }
    }

    #[tokio::test]
    async fn nothing_to_do_when_every_proof_is_valid() {
        let real = compute_blob_proof(&test_blob(4), &commitment_of(&test_blob(4))).unwrap();
        let blobs = vec![sidecar(4, real)];

        let (blobs, replaced) = replace_invalid_proofs(blobs).await.unwrap();

        assert_eq!(replaced, 0);
        assert_eq!(blobs[0].kzg_proof, real);
    }

    #[tokio::test]
    async fn the_infinity_point_of_a_constant_blob_is_not_counted_as_replaced() {
        // The KZG proof of a constant polynomial is the infinity point.
        let blob = HeapBlob::new(&vec![0u8; 131_072]).unwrap();
        let blobs = vec![BeaconBlob {
            kzg_commitment: commitment_of(&blob),
            kzg_proof: PLACEHOLDER_PROOF,
            blob,
        }];

        let (blobs, replaced) = replace_invalid_proofs(blobs).await.unwrap();

        assert_eq!(replaced, 0);
        assert_eq!(blobs[0].kzg_proof, PLACEHOLDER_PROOF);
    }

    #[tokio::test]
    async fn fails_when_a_blob_does_not_match_its_commitment() {
        let (a, b) = (test_blob(1), test_blob(2));
        let blobs = vec![BeaconBlob {
            kzg_commitment: commitment_of(&b),
            kzg_proof: PLACEHOLDER_PROOF,
            blob: a,
        }];

        let err = replace_invalid_proofs(blobs).await.unwrap_err();

        assert!(format!("{err:#}").contains("does not match"), "{err:#}");
    }

    #[tokio::test]
    async fn a_dummy_that_is_not_the_infinity_point_is_replaced_too() {
        let zeros = Proof::ZERO;
        let garbage = Proof::repeat_byte(0x11);
        // A valid proof of ANOTHER blob: well-formed, but not the proof of this one.
        let other = compute_blob_proof(&test_blob(9), &commitment_of(&test_blob(9))).unwrap();
        let blobs = vec![sidecar(1, zeros), sidecar(2, garbage), sidecar(3, other)];

        let (blobs, replaced) = replace_invalid_proofs(blobs).await.unwrap();

        assert_eq!(replaced, 3);

        for blob in &blobs {
            assert!(verify_blob_proof(
                &blob.blob,
                &blob.kzg_commitment,
                &blob.kzg_proof
            ));
        }
    }

    #[test]
    fn verification_accepts_only_the_right_proof() {
        let (blob, other) = (test_blob(1), test_blob(2));
        let (commitment, other_commitment) = (commitment_of(&blob), commitment_of(&other));
        let proof = compute_blob_proof(&blob, &commitment).unwrap();

        assert!(verify_blob_proof(&blob, &commitment, &proof));
        assert!(!verify_blob_proof(&blob, &other_commitment, &proof));
        assert!(!verify_blob_proof(&other, &commitment, &proof));
        assert!(!verify_blob_proof(&blob, &commitment, &PLACEHOLDER_PROOF));
        assert!(!verify_blob_proof(&blob, &commitment, &Proof::ZERO));
    }
}
