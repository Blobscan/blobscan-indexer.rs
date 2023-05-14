use anyhow::Result;
use context::{Config as ContextConfig, Context};
use env::Environment;
use slot_processor_manager::{SlotProcessorManager, SlotProcessorManagerError};
use slot_retryer::SlotRetryer;
use tracing::Instrument;

use crate::utils::telemetry::{get_subscriber, init_subscriber};

use std::{thread, time::Duration};

mod beacon_client;
mod blobscan_client;
mod context;
mod env;
mod slot_processor_manager;
mod slot_retryer;
mod utils;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    let env = Environment::from_env()?;

    let subscriber = get_subscriber("blobscan_indexer".into(), "info".into(), std::io::stdout);
    init_subscriber(subscriber);

    let context = Context::try_new(ContextConfig::from(env))?;
    let beacon_client = context.beacon_client();
    let blobscan_client = context.blobscan_client();

    let mut current_slot = match blobscan_client.get_slot().await? {
        Some(last_slot) => last_slot + 1,
        None => 0,
    };

    let slot_retryer = SlotRetryer::new(context.clone());
    let slot_retryer_span = tracing::info_span!("slot_retryer");

    slot_retryer.run().instrument(slot_retryer_span).await?;

    let slot_processor_manager = SlotProcessorManager::try_new(context.clone())?;

    loop {
        if let Some(latest_beacon_block) = beacon_client.get_block(None).await? {
            let latest_slot: u32 = latest_beacon_block.slot.parse()?;

            if current_slot < latest_slot {
                match slot_processor_manager
                    .process_slots(current_slot, latest_slot)
                    .await
                {
                    Ok(_) => (),
                    Err(err) => match err {
                        SlotProcessorManagerError::FailedSlotsProcessing { chunks } => {
                            blobscan_client.add_failed_slots_chunks(chunks).await?;
                        }
                        SlotProcessorManagerError::Other(err) => {
                            anyhow::bail!(err);
                        }
                    },
                }

                blobscan_client.update_slot(latest_slot - 1).await?;

                current_slot = latest_slot;
            }
        }

        thread::sleep(Duration::from_secs(10));
    }
}
