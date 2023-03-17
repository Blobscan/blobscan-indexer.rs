use std::{
    env,
    error::{self},
    thread,
    time::Duration,
};

use ethers::prelude::*;
use slots::{process_slots, types::BeaconAPIResponse, Config as SlotConfig};

use crate::db::{blob_db_manager::DBManager, mongodb::connect};

mod db;
mod slots;

type StdErr = Box<dyn error::Error>;

#[tokio::main]
async fn main() -> Result<(), StdErr> {
    dotenv::dotenv()?;

    let execution_node_rpc = env::var("EXECUTION_NODE_RPC")?;
    let beacon_node_rpc = env::var("BEACON_NODE_RPC")?;

    let provider = Provider::<Http>::try_from(execution_node_rpc)?;
    let db_manager = connect().await?;

    let mut config = SlotConfig {
        provider,
        db_manager,
        beacon_node_rpc,
    };

    let mut current_slot = match config.db_manager.read_metadata(None).await? {
        Some(metadata) => metadata.last_slot + 1,
        None => 0,
    };

    loop {
        let latest_beacon_block = reqwest::get(format!(
            "{}/eth/v2/beacon/blocks/head",
            config.beacon_node_rpc
        ))
        .await?
        .json::<BeaconAPIResponse>()
        .await?;
        let head_slot: u32 = latest_beacon_block.data.message.slot.parse()?;

        if current_slot < head_slot {
            process_slots(current_slot, head_slot, &mut config).await?;

            current_slot = head_slot;
        }

        thread::sleep(Duration::from_secs(1));
    }
}
