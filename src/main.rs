use crate::db::{
    blob_db_manager::{Blob, DBManager},
    mongodb::connect,
};
use blob_indexer::{calculate_versioned_hash, get_eip_4844_tx, get_tx_versioned_hashes};
use ethers::prelude::*;
use futures::future::join_all;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::{
    env, error,
    str::FromStr,
    thread,
    time::{Duration, Instant},
};

mod db;

type StdErr = Box<dyn error::Error>;

#[derive(Serialize, Deserialize, Debug)]
struct ExecutionPayload {
    block_hash: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct MessageBody {
    execution_payload: Option<ExecutionPayload>,
    blob_kzg_commitments: Option<Vec<String>>,
}
#[derive(Serialize, Deserialize, Debug)]
struct BlockMessage {
    slot: String,
    body: MessageBody,
}

#[derive(Serialize, Deserialize, Debug)]
struct ResponseData {
    message: BlockMessage,
}

#[derive(Serialize, Deserialize, Debug)]
struct BeaconAPIResponse {
    data: ResponseData,
}

#[derive(Serialize, Deserialize, Debug)]
struct SidecarData {
    blobs: Vec<Bytes>,
}

#[derive(Serialize, Deserialize, Debug)]
struct BeaconSidecarResponse {
    data: SidecarData,
}

#[tokio::main]
async fn main() -> Result<(), StdErr> {
    dotenv::dotenv()?;

    let execution_node_rpc = env::var("EXECUTION_NODE_RPC")?;
    let beacon_node_rpc = env::var("BEACON_NODE_RPC")?;

    let provider = Provider::<Http>::try_from(execution_node_rpc)?;
    let mut db_manager = connect().await?;

    let mut current_slot = 0;

    loop {
        let latest_beacon_block =
            reqwest::get(format!("{}/eth/v2/beacon/blocks/head", beacon_node_rpc))
                .await?
                .json::<BeaconAPIResponse>()
                .await?;
        let head_slot: u32 = latest_beacon_block.data.message.slot.parse()?;

        while current_slot < head_slot {
            let start = Instant::now();
            current_slot = current_slot + 1;

            println!("Reading slot {current_slot} (head slot {head_slot})");
            let beacon_block_response = reqwest::get(format!(
                "{}/eth/v2/beacon/blocks/{}",
                beacon_node_rpc, current_slot
            ))
            .await?;

            // TODO: handle rest of the response cases. What to do?
            if beacon_block_response.status() != StatusCode::OK {
                println!("Skipping slot as there is no beacon block");
                current_slot = current_slot + 1;

                continue;
            }

            let beacon_block_response = beacon_block_response.json::<BeaconAPIResponse>().await?;
            let beacon_block = beacon_block_response.data;
            let execution_payload = beacon_block.message.body.execution_payload;
            let blob_kzg_commitments = beacon_block.message.body.blob_kzg_commitments;

            if execution_payload.is_none() {
                println!("Skipping slot as there is no execution payload");
                continue;
            }

            if blob_kzg_commitments.is_none() {
                println!("Skipping slot as there is no blob commitment");
                continue;
            }

            let blob_kzg_commitments = blob_kzg_commitments.unwrap();
            let execution_block_hash = execution_payload.unwrap().block_hash;
            let execution_block_hash = H256::from_str(execution_block_hash.as_str())?;

            let execution_block = provider.get_block(execution_block_hash).await?.unwrap();

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
                continue;
            }

            let beacon_sidecar_response = reqwest::get(format!(
                "{}/eth/v1/beacon/blobs_sidecars/{}",
                beacon_node_rpc, current_slot
            ))
            .await?;

            if beacon_sidecar_response.status() != StatusCode::OK {
                println!("Skipping slot as an error occurred when fetching its sidecar");
                continue;
            }
            let beacon_sidecar_response = beacon_sidecar_response
                .json::<BeaconSidecarResponse>()
                .await?;
            let sidecar_blobs = beacon_sidecar_response.data.blobs;

            if sidecar_blobs.len() == 0 {
                println!("Skipping slot as there is no sidecar blobs");
                continue;
            }

            db_manager.start_transaction().await?;

            db_manager
                .insert_block(&execution_block, &blob_txs, current_slot, None)
                .await?;

            for (i, tx) in blob_txs.iter().enumerate() {
                db_manager.insert_tx(tx, i as u32, None).await?;
            }

            for (i, blob) in sidecar_blobs.iter().enumerate() {
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

                // TODO: use flyweight pattern to avoid cloning
                let blob = &Blob {
                    commitment: commitment.clone(),
                    data: blob.clone(),
                    index: i as u32,
                    versioned_hash,
                };

                db_manager.insert_blob(blob, blob_tx.hash, None).await?;
            }

            db_manager.commit_transaction().await?;

            let duration = start.elapsed();

            println!(
                "Blobs from slot {} indexed (elapsed time: {:?})",
                current_slot,
                duration.as_secs()
            );
        }

        thread::sleep(Duration::from_secs(1));
    }
}
