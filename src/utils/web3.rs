use std::str::FromStr;

use ethers::core::k256::sha2::{Digest, Sha256};
use ethers::{prelude::*, providers, types::H256};

const BLOB_COMMITMENT_VERSION_KZG: u8 = 0x01;

pub async fn get_eip_4844_tx(
    provider: &Provider<Http>,
    hash: &H256,
) -> Result<Transaction, providers::ProviderError> {
    provider
        .request::<Vec<&H256>, Transaction>("eth_getTransactionByHash", vec![hash])
        .await
}

pub fn sha256(value: &str) -> H256 {
    let value_without_prefix = if value.starts_with("0x") {
        &value[2..]
    } else {
        value
    };
    let value_without_prefix = hex::decode(value_without_prefix).unwrap();

    let mut hasher = Sha256::new();

    hasher.update(value_without_prefix);

    let result = hasher.finalize();

    H256::from_slice(&result)
}

pub fn calculate_versioned_hash(commitment: &String) -> H256 {
    let hashed_commitment = sha256(&commitment);

    // Replace first byte with the blob commitment version byte
    let hashed_commitment = &mut hashed_commitment.as_bytes()[1..].to_vec();
    hashed_commitment.insert(0, BLOB_COMMITMENT_VERSION_KZG);

    H256::from_slice(hashed_commitment)
}

pub fn get_tx_versioned_hashes(tx: &Transaction) -> Vec<H256> {
    tx.other
        .get("blobVersionedHashes")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|x| H256::from_str(x.as_str().unwrap()).unwrap())
        .collect::<Vec<H256>>()
}
