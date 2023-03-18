use std::{
    env,
    error::{self},
    thread,
    time::Duration,
};

use beacon_chain::BeaconChainAPI;
use ethers::prelude::*;
use slots::{process_slots, Config as SlotConfig};

use crate::db::{blob_db_manager::DBManager, mongodb::connect};

mod beacon_chain;
mod db;
mod slots;
mod utils;

type StdErr = Box<dyn error::Error>;

#[tokio::main]
async fn main() -> Result<(), StdErr> {
    dotenv::dotenv()?;

    let execution_node_rpc = env::var("EXECUTION_NODE_RPC")?;
    let beacon_node_rpc = env::var("BEACON_NODE_RPC")?;

    log4rs::init_file("log4rs.yml", Default::default()).unwrap();

    let beacon_api = BeaconChainAPI::new(beacon_node_rpc);
    let db_manager = connect().await?;
    let provider = Provider::<Http>::try_from(execution_node_rpc)?;

    let mut config = SlotConfig {
        beacon_api,
        db_manager,
        provider,
    };

    let mut current_slot = match config.db_manager.read_metadata(None).await? {
        Some(metadata) => metadata.last_slot + 1,
        None => 0,
    };

    loop {
        match config.beacon_api.get_block(None).await? {
            Some(latest_beacon_block) => {
                let latest_slot: u32 = latest_beacon_block.slot.parse()?;

                if current_slot < latest_slot {
                    process_slots(current_slot, latest_slot, &mut config).await;

                    current_slot = latest_slot;
                }
            }
            _ => (),
        };

        thread::sleep(Duration::from_secs(1));
    }
}
