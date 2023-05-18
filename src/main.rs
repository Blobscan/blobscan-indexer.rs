use anyhow::Result;
use backoff::future::retry_notify;
use context::{Config as ContextConfig, Context};
use env::Environment;
use slots_processor::{Config as SlotsProcessorConfig, SlotsProcessor};
use tracing::{error, info, warn, Instrument};
use utils::exp_backoff::get_exp_backoff_config;

use crate::utils::telemetry::{get_subscriber, init_subscriber};

use std::{thread, time::Duration};

mod beacon_client;
mod blobscan_client;
mod context;
mod env;
mod slots_processor;
mod utils;

const MAX_SLOTS_PER_SAVE: u32 = 1000;

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
                let unprocessed_slots = latest_slot - current_slot;
                let current_max_slots_size = std::cmp::min(unprocessed_slots, MAX_SLOTS_PER_SAVE);
                let num_chunks = unprocessed_slots / current_max_slots_size;

                let remaining_slots = unprocessed_slots % current_max_slots_size;
                let num_chunks = if remaining_slots > 0 {
                    num_chunks + 1
                } else {
                    num_chunks
                };

                info!(
                    "Processing slots from {} to {}, partitioned into {} chunks…",
                    current_slot, latest_slot, num_chunks
                );

                for i in 0..num_chunks {
                    let slots_in_current_chunk = if i == num_chunks - 1 {
                        current_max_slots_size + remaining_slots
                    } else {
                        current_max_slots_size
                    };

                    let chunk_initial_slot = current_slot + i * current_max_slots_size;
                    let chunk_final_slot = chunk_initial_slot + slots_in_current_chunk;

                    let slot_manager_span = tracing::info_span!(
                        "slots_processor",
                        initial_slot = chunk_initial_slot,
                        final_slot = chunk_final_slot
                    );

                    slots_processor
                        .process_slots(chunk_initial_slot, chunk_final_slot)
                        .instrument(slot_manager_span)
                        .await?;

                    match retry_notify(
                        get_exp_backoff_config(),
                        || async move {
                            blobscan_client
                                .update_slot(chunk_final_slot - 1)
                                .await.map_err(|err| err.into())
                        },
                        |e, duration: Duration| {
                            let duration = duration.as_secs();
                            warn!("Failed to update latest slot to {}. Retrying in {duration} seconds… (Reason: {e})", chunk_final_slot - 1);
                        },
                    ).await {
                        Ok(_) => (),
                        Err(err) => {
                            error!("Failed to update latest slot to {}", chunk_final_slot - 1);
                            return Err(err.into());
                        }
                    };

                    info!(
                        "Chunk {} of {} ({} slots) processed successfully!. Latest slot updated to {}.",
                        i+1,
                        num_chunks,
                        chunk_final_slot - chunk_initial_slot,
                        chunk_final_slot - 1
                    );
                }

                current_slot = latest_slot;

                info!(
                    "All slots processed successfully! Total slots processed: {}",
                    unprocessed_slots
                );
            }
        }

        thread::sleep(Duration::from_secs(10));
    }
}
