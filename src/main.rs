use anyhow::Result;
use context::{Config as ContextConfig, Context};
use env::Environment;
use slots_processor::{Config as SlotsProcessorConfig, SlotsProcessor};
use tracing::{info, Instrument};

use crate::utils::telemetry::{get_subscriber, init_subscriber};

use std::{thread, time::Duration};

mod beacon_client;
mod blobscan_client;
mod context;
mod env;
mod slots_processor;
mod utils;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    let env = Environment::from_env()?;

    let subscriber = get_subscriber("blobscan_indexer".into(), "info".into(), std::io::stdout);
    init_subscriber(subscriber);

    let slots_processor_config = env
        .num_processing_threads
        .map(|threads_length| SlotsProcessorConfig { threads_length });
    let context = Context::try_new(ContextConfig::from(env))?;
    let beacon_client = context.beacon_client();
    let blobscan_client = context.blobscan_client();

    let mut current_slot = match blobscan_client.get_slot().await? {
        Some(last_slot) => last_slot + 1,
        None => 0,
    };

    let slots_processor = SlotsProcessor::try_new(context.clone(), slots_processor_config)?;

    loop {
        if let Some(latest_beacon_block) = beacon_client.get_block(None).await? {
            let latest_slot: u32 = latest_beacon_block.slot.parse()?;

            if current_slot < latest_slot {
                let slot_manager_span = tracing::debug_span!(
                    "slot_processor_manager",
                    initial_slot = current_slot,
                    final_slot = latest_slot
                );

                slots_processor
                    .process_slots(current_slot, latest_slot)
                    .instrument(slot_manager_span)
                    .await?;

                blobscan_client.update_slot(latest_slot - 1).await?;
                info!("Latest slot updated to {}", latest_slot - 1);

                current_slot = latest_slot;
            }
        }

        thread::sleep(Duration::from_secs(10));
    }
}
