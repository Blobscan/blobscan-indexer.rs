use std::str::FromStr;

use ethers::core::k256::sha2::{Digest, Sha256};
use ethers::{prelude::*, types::H256};

use crate::types::StdError;

const BLOB_COMMITMENT_VERSION_KZG: u8 = 0x01;

pub fn sha256(value: &str) -> Result<H256, StdError> {
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

pub fn calculate_versioned_hash(commitment: &str) -> Result<H256, StdError> {
    let hashed_commitment = sha256(commitment)?;

    // Replace first byte with the blob commitment version byte
    let hashed_commitment = &mut hashed_commitment.as_bytes()[1..].to_vec();
    hashed_commitment.insert(0, BLOB_COMMITMENT_VERSION_KZG);

    Ok(H256::from_slice(hashed_commitment))
}

pub fn get_tx_versioned_hashes(tx: &Transaction) -> Result<Option<Vec<H256>>, StdError> {
    match tx.other.get("blobVersionedHashes") {
        Some(blob_versioned_hashes) => {
            let blob_versioned_hashes = match blob_versioned_hashes.as_array() {
                Some(blob_versioned_hashes) => blob_versioned_hashes,
                None => {
                    return Err("blobVersionedHashes field is not an array".into());
                }
            };

            if blob_versioned_hashes.is_empty() {
                return Ok(None);
            }

            let blob_versioned_hashes = blob_versioned_hashes
                .iter()
                .map(|versioned_hash| match versioned_hash.as_str() {
                    Some(versioned_hash) => Ok(H256::from_str(versioned_hash)?),
                    None => Err("blobVersionedHashes field is not an array of strings".into()),
                })
                .collect::<Result<Vec<H256>, StdError>>()?;

            Ok(Some(blob_versioned_hashes))
        }
        None => Ok(None),
    }
}
