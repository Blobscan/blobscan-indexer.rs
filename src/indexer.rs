use std::{thread, time::Duration};

use anyhow::Result;
use backoff::future::retry_notify;
use clap::Parser;
use tracing::{debug, error, warn, Instrument};

use crate::{
    args::Args,
    context::{Config as ContextConfig, Context},
    env::Environment,
    slots_processor::{Config as SlotsProcessorConfig, SlotsProcessor},
    utils::exp_backoff::get_exp_backoff_config,
};

pub async fn run() -> Result<()> {
    let env = Environment::from_env()?;
    let args = Args::parse();

    let max_slot_per_save = args.slots_per_save.unwrap_or(1000);
    let slots_processor_config = args
        .num_threads
        .map(|threads_length| SlotsProcessorConfig { threads_length });
    let context = match Context::try_new(ContextConfig::from(env)) {
        Ok(c) => c,
        Err(error) => {
            error!(target = "indexer", ?error, "Failed to create context");

            return Err(error);
        }
    };

    let beacon_client = context.beacon_client();
    let blobscan_client = context.blobscan_client();

    let mut current_slot = match args.from_slot {
        Some(from_slot) => from_slot,
        None => match blobscan_client.get_slot().await {
            Err(error) => {
                error!(target = "indexer", ?error, "Failed to fetch latest slot");

                return Err(error.into());
            }
            Ok(res) => match res {
                Some(latest_slot) => latest_slot + 1,
                None => 0,
            },
        },
    };

    let slots_processor = SlotsProcessor::try_new(context.clone(), slots_processor_config)?;

    loop {
        let beacon_head_result = match retry_notify(
            get_exp_backoff_config(),
            || async move {
                beacon_client
                    .get_block(None)
                    .await
                    .map_err(|err| err.into())
            },
            |_, duration: Duration| {
                let duration = duration.as_secs();
                warn!(
                    target = "indexer",
                    "Failed to fetch beacon head block. Retrying in {duration} seconds…"
                );
            },
        )
        .await
        {
            Err(error) => {
                error!(
                    target = "indexer",
                    ?error,
                    "Failed to fetch beacon head block"
                );

                return Err(error.into());
            }
            Ok(res) => res,
        };

        if let Some(latest_beacon_block) = beacon_head_result {
            let latest_slot: u32 = latest_beacon_block.slot.parse()?;

            if current_slot < latest_slot {
                let mut unprocessed_slots = latest_slot - current_slot;

                while unprocessed_slots > 0 {
                    let slots_chunk = std::cmp::min(unprocessed_slots, max_slot_per_save);
                    let chunk_initial_slot = current_slot;
                    let chunk_final_slot = current_slot + slots_chunk;

                    let slot_manager_span = tracing::debug_span!(
                        "slots_processor",
                        initial_slot = chunk_initial_slot,
                        final_slot = chunk_final_slot
                    );

                    slots_processor
                        .process_slots(chunk_initial_slot, chunk_final_slot)
                        .instrument(slot_manager_span)
                        .await?;

                    if let Err(error) = retry_notify(
                        get_exp_backoff_config(),
                        || async move {
                            blobscan_client
                                .update_slot(chunk_final_slot - 1)
                                .await
                                .map_err(|err| err.into())
                        },
                        |_, duration: Duration| {
                            let duration = duration.as_secs();
                            warn!(
                                target = "indexer",
                                latest_slot = chunk_final_slot - 1,
                                "Failed to update latest slot. Retrying in {duration} seconds…"
                            );
                        },
                    )
                    .await
                    {
                        error!(target = "indexer", ?error, "Failed to update latest slot");

                        return Err(error.into());
                    }

                    debug!(
                        target = "indexer",
                        latest_slot = chunk_final_slot - 1,
                        "Latest slot updated"
                    );

                    current_slot += slots_chunk;
                    unprocessed_slots -= slots_chunk;
                }
            }
        }

        thread::sleep(Duration::from_secs(10));
    }
}
