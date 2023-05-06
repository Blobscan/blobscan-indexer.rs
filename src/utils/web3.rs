use std::str::FromStr;

use anyhow::{Context, Result};
use ethers::core::k256::sha2::{Digest, Sha256};
use ethers::{prelude::*, types::H256};

const BLOB_COMMITMENT_VERSION_KZG: u8 = 0x01;

pub fn sha256(value: &str) -> Result<H256> {
    let value_without_prefix = if let Some(value_without_prefix) = value.strip_prefix("0x") {
        value_without_prefix
    } else {
        value
    };
    let value_without_prefix = hex::decode(value_without_prefix)?;

    let mut hasher = Sha256::new();

    hasher.update(value_without_prefix);

    let result = hasher.finalize();

    Ok(H256::from_slice(&result))
}

pub fn calculate_versioned_hash(commitment: &str) -> Result<H256> {
    let hashed_commitment =
        sha256(commitment).context(format!("Failed to encode commitment {commitment}"))?;

    // Replace first byte with the blob commitment version byte
    let hashed_commitment = &mut hashed_commitment.as_bytes()[1..].to_vec();
    hashed_commitment.insert(0, BLOB_COMMITMENT_VERSION_KZG);

    Ok(H256::from_slice(hashed_commitment))
}

pub fn get_tx_versioned_hashes(tx: &Transaction) -> Result<Option<Vec<H256>>> {
    match tx.other.get("blobVersionedHashes") {
        Some(blob_versioned_hashes) => {
            let blob_versioned_hashes = blob_versioned_hashes
                .as_array()
                .context("blobVersionedHashes field is not an array")?;

            if blob_versioned_hashes.is_empty() {
                return Ok(None);
            }

            let blob_versioned_hashes = blob_versioned_hashes
                .iter()
                .enumerate()
                .map(|(i, versioned_hash)| {
                    versioned_hash
                        .as_str()
                        .with_context(|| format!("blobVersionedHashes[{}]: expected a string", i))
                        .and_then(|versioned_hash| {
                            H256::from_str(versioned_hash)
                                .context(format!("blobVersionedHashes[{}]: invalid H256", i))
                        })
                })
                .collect::<Result<Vec<H256>>>()?;

            Ok(Some(blob_versioned_hashes))
        }
        None => Ok(None),
    }
}
