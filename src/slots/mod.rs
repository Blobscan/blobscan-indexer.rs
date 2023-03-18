use std::{error, panic, str::FromStr, time::Instant};

use blob_indexer::{calculate_versioned_hash, get_eip_4844_tx, get_tx_versioned_hashes};
use ethers::prelude::*;
use futures::future::join_all;
use log::{error, info};

use crate::{
    beacon_chain::BeaconChainAPI,
    db::{
        blob_db_manager::DBManager,
        mongodb::{MongoDBManager, MongoDBManagerOptions},
        types::Blob,
    },
    utils::logs::INDEXER_LOGGER,
};

type StdErr = Box<dyn error::Error>;

pub struct Config {
    pub db_manager: MongoDBManager,
    pub beacon_api: BeaconChainAPI,
    pub provider: Provider<Http>,
}

pub async fn process_slots(start_slot: u32, end_slot: u32, config: &mut Config) {
    let mut current_slot = start_slot;

    while current_slot < end_slot {
        let result = process_slot(current_slot, config).await;

        // TODO: implement exponential backoff for proper error handling. If X intents have been made, then notify and stop process
        if let Err(e) = result {
            save_slot(current_slot, config).await;

            error!(
                target: INDEXER_LOGGER,
                "[Slot {}] Couldn't process slot: {}", current_slot, e
            );

            panic!();
        };

        current_slot = current_slot + 1;
    }

    save_slot(current_slot, config).await
}

async fn process_slot(slot: u32, config: &mut Config) -> Result<(), StdErr> {
    let provider = &config.provider;
    let db_manager = &mut config.db_manager;
    let beacon_api = &config.beacon_api;

    let start = Instant::now();
    let beacon_block = match beacon_api.get_block(Some(slot)).await? {
        Some(block) => block,
        None => {
            info!(
                target: INDEXER_LOGGER,
                "[Slot {}] Skipping as there is no beacon block", slot
            );

            return Ok(());
        }
    };

    let execution_payload = match beacon_block.body.execution_payload {
        Some(payload) => payload,
        None => {
            info!(
                target: INDEXER_LOGGER,
                "[Slot {}] Skipping as beacon block doesn't contain execution payload", slot
            );

            return Ok(());
        }
    };

    let blob_kzg_commitments = match beacon_block.body.blob_kzg_commitments {
        Some(commitments) => commitments,
        None => {
            info!(
                target: INDEXER_LOGGER,
                "[Slot {}] Skipping as beacon block doesn't contain blob kzg commitments", slot
            );

            return Ok(());
        }
    };
    let execution_block_hash = execution_payload.block_hash;
    let execution_block_hash = H256::from_str(execution_block_hash.as_str())?;

    let execution_block = match config.provider.get_block(execution_block_hash).await? {
        Some(block) => block,
        None => {
            let error_msg = format!("Execution block {} not found", execution_block_hash);

            return Err(Box::new(ProviderError::CustomError(error_msg)));
        }
    };

    let execution_block_txs = join_all(
        execution_block
            .transactions
            .iter()
            .map(|tx_hash| get_eip_4844_tx(&provider, tx_hash)),
    )
    .await
    .into_iter()
    .map(|tx| tx.unwrap())
    .collect::<Vec<Transaction>>();

    let blob_txs = execution_block_txs
        .into_iter()
        .filter(|tx| tx.other.contains_key("blobVersionedHashes"))
        .collect::<Vec<Transaction>>();

    if blob_txs.len() == 0 {
        info!(
            target: INDEXER_LOGGER,
            "[Slot {}] Skipping as execution block doesn't contain blob txs", slot
        );

        return Ok(());
    }

    let blobs = match beacon_api.get_blobs_sidecar(slot).await? {
        Some(blobs_sidecar) => {
            if blobs_sidecar.blobs.len() == 0 {
                info!(
                    target: INDEXER_LOGGER,
                    "[Slot {}] Skipping as blobs sidecar is empty", slot
                );

                return Ok(());
            } else {
                blobs_sidecar.blobs
            }
        }
        None => {
            info!(
                target: INDEXER_LOGGER,
                "[Slot {}] Skipping as there is no blobs sidecar", slot
            );

            return Ok(());
        }
    };

    db_manager.start_transaction().await?;

    db_manager
        .insert_block(&execution_block, &blob_txs, slot, None)
        .await?;

    for (i, tx) in blob_txs.iter().enumerate() {
        db_manager.insert_tx(tx, i as u32, None).await?;
    }

    for (i, blob) in blobs.iter().enumerate() {
        let commitment = &blob_kzg_commitments[i];

        let versioned_hash = calculate_versioned_hash(commitment);

        let blob_tx = blob_txs
            .iter()
            .find(|tx| {
                let versioned_hashes = get_tx_versioned_hashes(tx);
                versioned_hashes.contains(&versioned_hash)
            })
            .unwrap()
            .clone();

        // TODO: use flyweight pattern to avoid cloning structs
        let blob = &Blob {
            commitment: commitment.clone(),
            data: blob.clone(),
            index: i as u32,
            versioned_hash,
        };

        db_manager.insert_blob(blob, blob_tx.hash, None).await?;
    }

    config.db_manager.commit_transaction(None).await?;

    let duration = start.elapsed();

    info!(
        target: INDEXER_LOGGER,
        "[Slot {}] Blobs indexed correctly (elapsed time: {:?}s)",
        slot,
        duration.as_secs()
    );

    Ok(())
}

async fn save_slot(slot: u32, config: &mut Config) {
    let result = config
        .db_manager
        .update_last_slot(slot, Some(MongoDBManagerOptions { use_session: false }))
        .await;

    if let Err(e) = result {
        error!(target: INDEXER_LOGGER, "Couldn't update last slot: {}", e);
        panic!();
    }
}
