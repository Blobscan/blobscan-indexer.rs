use std::collections::HashMap;

use crate::{clients::beacon::types::Blob as BeaconBlob, utils::web3::calculate_versioned_hash};
use alloy::{consensus::Transaction, primitives::B256, rpc::types::Block};

pub fn create_tx_hash_versioned_hashes_mapping(
    block: &Block,
) -> Result<HashMap<B256, Vec<B256>>, anyhow::Error> {
    let mut tx_to_versioned_hashes = HashMap::new();

    if let Some(transactions) = block.transactions.as_transactions() {
        transactions.iter().for_each(|tx| {
            if let Some(versioned_hashes) = tx.inner.blob_versioned_hashes() {
                tx_to_versioned_hashes.insert(tx.info().hash.unwrap(), versioned_hashes.to_vec());
            }
        });
    }

    Ok(tx_to_versioned_hashes)
}

pub fn create_versioned_hash_blob_mapping(
    blobs: &Vec<BeaconBlob>,
) -> Result<HashMap<B256, &BeaconBlob>, anyhow::Error> {
    let mut version_hash_to_blob = HashMap::new();

    for blob in blobs {
        let versioned_hash = calculate_versioned_hash(&blob.kzg_commitment)?;

        version_hash_to_blob.entry(versioned_hash).or_insert(blob);
    }

    Ok(version_hash_to_blob)
}
