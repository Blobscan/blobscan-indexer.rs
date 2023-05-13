use anyhow::Result;
use context::Context;
use slot_processor_manager::SlotProcessorManager;

use crate::utils::telemetry::{get_subscriber, init_subscriber};

use std::{thread, time::Duration};

mod beacon_client;
mod blobscan_client;
mod context;
mod env;
mod slot_processor_manager;
mod utils;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();

    let subscriber = get_subscriber("blobscan_indexer".into(), "info".into(), std::io::stdout);
    init_subscriber(subscriber);

    let context = Context::try_new()?;
    let beacon_client = context.beacon_client();
    let blobscan_client = context.blobscan_client();
    let mut current_slot = match blobscan_client.get_slot().await? {
        Some(last_slot) => last_slot + 1,
        None => 0,
    };
    let slot_processor_manager = SlotProcessorManager::try_new(context.clone())?;

    loop {
        if let Some(latest_beacon_block) = beacon_client.get_block(None).await? {
            let latest_slot: u32 = latest_beacon_block.slot.parse()?;

            if current_slot < latest_slot {
                slot_processor_manager
                    .process_slots(current_slot, latest_slot)
                    .await?;

                blobscan_client.update_slot(latest_slot - 1).await?;

                current_slot = latest_slot;
            }
        }

        thread::sleep(Duration::from_secs(10));
    }
}
