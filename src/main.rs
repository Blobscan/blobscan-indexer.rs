use crate::mongodb::connect_to_database;
use blob_indexer::{calculate_versioned_hash, get_eip_4844_tx};
use ethers::prelude::*;
use futures::future::join_all;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::{env, error, str::FromStr, thread, time::Duration};

mod mongodb;

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
    blobs: Vec<String>,
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
    let db = connect_to_database().await?;

    // let mut current_slot = 0;
    let mut current_slot = 1222;

    loop {
        let latest_beacon_block =
            reqwest::get(format!("{}/eth/v2/beacon/blocks/head", beacon_node_rpc))
                .await?
                .json::<BeaconAPIResponse>()
                .await?;
        let head_slot: u32 = latest_beacon_block.data.message.slot.parse()?;

        while current_slot < head_slot {
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
            // println!("{:?}", beacon_block);
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
                    .into_iter()
                    .map(|tx_hash| get_eip_4844_tx(&provider, tx_hash)),
            )
            .await;

            let execution_block_txs = execution_block_txs
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
                "{}/eth/v1/blobs/sidecar/{}",
                beacon_node_rpc, current_slot
            ))
            .await?;

            if beacon_sidecar_response.status() == StatusCode::OK {
                println!("Skipping slot as an error occurred when fetching its sidecar");
                continue;
            }

            println!("PROCESSING SIDECAR");
            let beacon_sidecar_response = beacon_sidecar_response
                .json::<BeaconSidecarResponse>()
                .await?;
            let sidecar_blobs = beacon_sidecar_response.data.blobs;
            let mut index: usize = 0;

            let res = sidecar_blobs.into_iter().map(|blob| {
                let commitment = &blob_kzg_commitments[index];
                let versioned_hash = calculate_versioned_hash(commitment);

                index = index + 1;
            });
            println!("Execution block {execution_block_hash} read");
        }

        thread::sleep(Duration::from_secs(1));
    }
}
