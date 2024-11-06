use alloy::primitives::B256;
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

const BLOB_COMMITMENT_VERSION_KZG: u8 = 0x01;

pub fn sha256(value: &str) -> Result<B256> {
    let value_without_prefix = if let Some(value_without_prefix) = value.strip_prefix("0x") {
        value_without_prefix
    } else {
        value
    };
    let value_without_prefix = hex::decode(value_without_prefix)?;

    let mut hasher = Sha256::new();

    hasher.update(value_without_prefix);

    let result = hasher.finalize();

    Ok(B256::from_slice(&result))
}

pub fn calculate_versioned_hash(commitment: &str) -> Result<B256> {
    let hashed_commitment =
        sha256(commitment).context(format!("Failed to encode commitment {commitment}"))?;

    // Replace first byte with the blob commitment version byte
    let hashed_commitment = &mut hashed_commitment[1..].to_vec();
    hashed_commitment.insert(0, BLOB_COMMITMENT_VERSION_KZG);

    Ok(B256::from_slice(hashed_commitment))
}

pub fn get_full_hash(hash: &B256) -> String {
    format!("0x{:x}", hash)
}
