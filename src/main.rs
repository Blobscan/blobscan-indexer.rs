use std::{thread, time::Duration};

use anyhow::Result;
use backoff::future::retry_notify;
use tracing::{error, info, warn, Instrument};

use args::Args;
use clap::Parser;
use context::{Config as ContextConfig, Context};
use env::Environment;
use slots_processor::{Config as SlotsProcessorConfig, SlotsProcessor};
use utils::exp_backoff::get_exp_backoff_config;
use utils::telemetry::{get_subscriber, init_subscriber};

mod args;
mod clients;
mod context;
mod env;
mod slots_processor;
mod utils;

const MAX_SLOTS_PER_SAVE: u32 = 1000;

async fn run() -> Result<()> {
    dotenv::dotenv().ok();
    let env = Environment::from_env()?;
    let args = Args::parse();

    let mut _guard;

    if let Some(sentry_dsn) = env.sentry_dsn.clone() {
        _guard = sentry::init((
            sentry_dsn,
            sentry::ClientOptions {
                release: sentry::release_name!(),
                ..Default::default()
            },
        ));
    }

    let subscriber = get_subscriber("blobscan_indexer".into(), "info".into(), std::io::stdout);
    init_subscriber(subscriber);

    let slots_processor_config = args
        .num_threads
        .map(|threads_length| SlotsProcessorConfig { threads_length });
    let context = match Context::try_new(ContextConfig::from(env)) {
        Ok(c) => c,
        Err(error) => {
            error!(
                target = "blobscan_indexer",
                ?error,
                "Failed to create context"
            );

            return Err(error);
        }
    };

    let beacon_client = context.beacon_client();
    let blobscan_client = context.blobscan_client();

    let mut current_slot = match args.from_slot {
        Some(from_slot) => from_slot,
        None => match blobscan_client.get_slot().await {
            Err(error) => {
                error!(
                    target = "blobscan_indexer",
                    ?error,
                    "Failed to fetch latest slot"
                );

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
                    target = "blobscan_indexer",
                    "Failed to fetch beacon head block. Retrying in {duration} seconds…"
                );
            },
        )
        .await
        {
            Err(error) => {
                error!(
                    target = "blobscan_indexer",
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
                    let slots_chunk = std::cmp::min(unprocessed_slots, MAX_SLOTS_PER_SAVE);
                    let chunk_initial_slot = current_slot;
                    let chunk_final_slot = current_slot + slots_chunk;

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
                                .await
                                .map_err(|err| err.into())
                        },
                        |_, duration: Duration| {
                            let duration = duration.as_secs();
                            warn!(
                                target = "blobscan_indexer",
                                latest_slot = chunk_final_slot - 1,
                                "Failed to update latest slot. Retrying in {duration} seconds…"
                            );
                        },
                    )
                    .await
                    {
                        Err(error) => {
                            error!(
                                target = "blobscan_indexer",
                                ?error,
                                "Failed to update latest slot"
                            );

                            return Err(error.into());
                        }
                        Ok(_) => (),
                    };

                    info!(
                        target = "blobscan_indexer",
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

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}
