use std::{thread, time::Duration};

use slots::process_slots;
use types::StdError;

use crate::{db::blob_db_manager::DBManager, utils::context::create_context};

mod beacon_chain;
mod db;
mod slots;
mod types;
mod utils;

#[tokio::main]
async fn main() -> Result<(), StdError> {
    dotenv::dotenv().expect("Failed to read .env file");

    log4rs::init_file("log4rs.yml", Default::default()).unwrap();

    let mut context = create_context().await?;

    let mut current_slot = match context.db_manager.read_metadata(None).await? {
        Some(metadata) => metadata.last_slot + 1,
        None => 0,
    };

    loop {
        match context.beacon_api.get_block(None).await? {
            Some(latest_beacon_block) => {
                let latest_slot: u32 = latest_beacon_block.slot.parse()?;

                if current_slot < latest_slot {
                    process_slots(current_slot, latest_slot, &mut context).await;

                    current_slot = latest_slot;
                }
            }
            _ => (),
        };

        thread::sleep(Duration::from_secs(1));
    }
}
