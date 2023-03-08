use std::str::from_utf8;

use ethers::{prelude::*, providers, utils::keccak256};

const BLOB_COMMITMENT_VERSION_KZG: &str = "0x01";

pub async fn get_eip_4844_tx(
    provider: &Provider<Http>,
    hash: H256,
) -> Result<Transaction, providers::ProviderError> {
    provider
        .request::<Vec<H256>, Transaction>("eth_getTransactionByHash", vec![hash])
        .await
}

pub fn calculate_versioned_hash(commitment: &String) -> String {
    let commitment_hash = &keccak256(commitment.as_bytes())[4..];
    let commitment_hash = from_utf8(commitment_hash).unwrap();

    format!("{}{}", BLOB_COMMITMENT_VERSION_KZG, commitment_hash)
}
