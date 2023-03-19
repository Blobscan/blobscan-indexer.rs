use std::{error, panic, str::FromStr, time::Instant};

use ethers::prelude::*;
use futures::future::join_all;
use log::{error, info};

use crate::{
    context::Context,
    db::{blob_db_manager::DBManager, mongodb::MongoDBManagerOptions, types::Blob},
    utils::web3::{calculate_versioned_hash, get_eip_4844_tx, get_tx_versioned_hashes},
};

type StdErr = Box<dyn error::Error>;

pub async fn process_slots(start_slot: u32, end_slot: u32, context: &mut Context) {
    let mut current_slot = start_slot;

    while current_slot < end_slot {
        let result = process_slot(current_slot, context).await;

        // TODO: implement exponential backoff for proper error handling. If X intents have been made, then notify and stop process
        if let Err(e) = result {
            save_slot(current_slot - 1, context).await;

            error!(
                target: context.logger.as_str(),
                "[Slot {}] Couldn't process slot: {}", current_slot, e
            );

            panic!();
        };

        current_slot = current_slot + 1;
    }

    save_slot(current_slot, context).await
}

async fn process_slot(slot: u32, context: &mut Context) -> Result<(), StdErr> {
    let Context {
        beacon_api,
        db_manager,
        provider,
        logger,
    } = context;

    let start = Instant::now();
    let beacon_block = match beacon_api.get_block(Some(slot)).await? {
        Some(block) => block,
        None => {
            info!(
                target: logger,
                "[Slot {}] Skipping as there is no beacon block", slot
            );

            return Ok(());
        }
    };

    let execution_payload = match beacon_block.body.execution_payload {
        Some(payload) => payload,
        None => {
            info!(
                target: logger,
                "[Slot {}] Skipping as beacon block doesn't contain execution payload", slot
            );

            return Ok(());
        }
    };

    let blob_kzg_commitments = match beacon_block.body.blob_kzg_commitments {
        Some(commitments) => commitments,
        None => {
            info!(
                target: logger,
                "[Slot {}] Skipping as beacon block doesn't contain blob kzg commitments", slot
            );

            return Ok(());
        }
    };
    let execution_block_hash = execution_payload.block_hash;
    let execution_block_hash = H256::from_str(execution_block_hash.as_str())?;

    let execution_block = match provider.get_block(execution_block_hash).await? {
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
            target: logger,
            "[Slot {}] Skipping as execution block doesn't contain blob txs", slot
        );

        return Ok(());
    }

    let blobs = match beacon_api.get_blobs_sidecar(slot).await? {
        Some(blobs_sidecar) => {
            if blobs_sidecar.blobs.len() == 0 {
                info!(
                    target: logger,
                    "[Slot {}] Skipping as blobs sidecar is empty", slot
                );

                return Ok(());
            } else {
                blobs_sidecar.blobs
            }
        }
        None => {
            info!(
                target: logger,
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

    db_manager.commit_transaction(None).await?;

    let duration = start.elapsed();

    info!(
        target: logger,
        "[Slot {}] Blobs indexed correctly (elapsed time: {:?}s)",
        slot,
        duration.as_secs()
    );

    Ok(())
}

async fn save_slot(slot: u32, context: &mut Context) {
    let Context {
        db_manager, logger, ..
    } = context;

    let result = db_manager
        .update_last_slot(slot, Some(MongoDBManagerOptions { use_session: false }))
        .await;

    if let Err(e) = result {
        error!(target: logger, "Couldn't update last slot: {}", e);
        panic!();
    }
}
