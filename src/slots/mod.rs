use std::{error::Error, str::FromStr, time::Instant};

use blob_indexer::{calculate_versioned_hash, get_eip_4844_tx, get_tx_versioned_hashes};
use ethers::prelude::*;
use futures::future::join_all;

use crate::{
    beacon_chain::BeaconChainAPI,
    db::{
        blob_db_manager::DBManager,
        mongodb::{MongoDBManager, MongoDBManagerOptions},
        types::Blob,
    },
};

type StdErr = Box<dyn Error>;

pub struct Config {
    pub db_manager: MongoDBManager,
    pub beacon_api: BeaconChainAPI,
    pub provider: Provider<Http>,
}

async fn process_slot(slot: u32, config: &mut Config) -> Result<(), StdErr> {
    let provider = &config.provider;
    let db_manager = &mut config.db_manager;
    let beacon_api = &config.beacon_api;

    let start = Instant::now();
    println!("Reading slot {slot}");
    let beacon_block = match beacon_api.get_block(Some(slot)).await {
        Ok(block) => block,
        Err(err) => {
            println!("Skipping slot as there is no beacon block");

            return Ok(());
        }
    };

    let execution_payload = match beacon_block.body.execution_payload {
        Some(payload) => payload,
        None => {
            println!("Skipping slot as there is no execution payload");

            return Ok(());
        }
    };

    let blob_kzg_commitments = match beacon_block.body.blob_kzg_commitments {
        Some(commitments) => commitments,
        None => {
            println!("Skipping slot as there is no blob commitment");

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
        println!("Skipping slot as there is no blob tx in execution block");

        return Ok(());
    }

    let blobs = match beacon_api.get_blobs_sidecar(slot).await {
        Ok(blobs_sidecar) => blobs_sidecar.blobs,
        Err(err) => {
            println!("Skipping slot as an error occurred when fetching its sidecar");

            return Ok(());
        }
    };

    if blobs.len() == 0 {
        println!("Skipping slot as there is no sidecar blobs");

        return Ok(());
    }

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

    println!(
        "Blobs from slot {} indexed (elapsed time: {:?})",
        slot,
        duration.as_secs()
    );

    Ok(())
}

pub async fn process_slots(
    start_slot: u32,
    end_slot: u32,
    config: &mut Config,
) -> Result<(), StdErr> {
    let mut current_slot = start_slot;

    while current_slot < end_slot {
        let result = process_slot(current_slot, config).await;

        // TODO: implement exponential backoff for proper error handling. If X intents have been made, then notify and stop process
        if let Err(e) = result {
            config
                .db_manager
                .update_last_slot(
                    current_slot,
                    Some(MongoDBManagerOptions { use_session: false }),
                )
                .await?;
            panic!("Error while processing slot {}: {}", current_slot, e);
        };

        current_slot = current_slot + 1;
    }

    config
        .db_manager
        .update_last_slot(
            current_slot - 1,
            Some(MongoDBManagerOptions { use_session: false }),
        )
        .await?;

    Ok(())
}
