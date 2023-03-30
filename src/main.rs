use anyhow::Result;
use tracing::Instrument;

use crate::{
    db::blob_db_manager::DBManager,
    slot_processor::SlotProcessor,
    utils::{
        context::create_context,
        telemetry::{get_subscriber, init_subscriber},
    },
};
use std::{thread, time::Duration};

mod beacon_chain;
mod db;
mod slot_processor;
mod types;
mod utils;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().expect("Failed to read .env file");

    let subscriber = get_subscriber("blobscan_indexer".into(), "info".into(), std::io::stdout);
    init_subscriber(subscriber);

    let context = create_context().await?;
    let mut slot_processor = SlotProcessor::try_init(&context, None).await?;

    let mut current_slot = match context.db_manager.read_metadata(None).await? {
        Some(metadata) => metadata.last_slot + 1,
        None => 0,
    };

    loop {
        if let Some(latest_beacon_block) = context.beacon_api.get_block(None).await? {
            let latest_slot: u32 = latest_beacon_block.slot.parse()?;

            let slot_span = tracing::trace_span!("slot_processor", slot = latest_slot);

            if current_slot < latest_slot {
                slot_processor
                    .process_slots(current_slot, latest_slot)
                    .instrument(slot_span)
                    .await?;

                current_slot = latest_slot;
            }
        }
        thread::sleep(Duration::from_secs(1));
    }
}
