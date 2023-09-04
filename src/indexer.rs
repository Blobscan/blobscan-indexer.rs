use std::{
    cmp::{min, Ordering},
    thread,
    time::Duration,
};

use anyhow::{anyhow, Result};
use backoff::future::retry_notify;
use clap::Parser;
use tracing::{debug, error, info, warn, Instrument};

use crate::{
    args::Args,
    context::{Config as ContextConfig, Context},
    env::Environment,
    slots_processor::{Config as SlotsProcessorConfig, SlotsProcessor},
    utils::exp_backoff::get_exp_backoff_config,
};

pub fn print_banner(args: &Args, env: &Environment) {
    let num_threads = args.num_threads.unwrap_or_default();
    let sentry_dsn = env.sentry_dsn.clone();
    println!("____  _       _                         ");
    println!("| __ )| | ___ | |__  ___  ___ __ _ _ __  ");
    println!("|  _ \\| |/ _ \\| '_ \\/ __|/ __/ _` | '_ \\ ");
    println!("| |_) | | (_) | |_) \\__ \\ (_| (_| | | | |");
    println!("|____/|_|\\___/|_.__/|___/\\___\\__,_|_| |_|\n");
    println!("Blobscan indexer (EIP-4844 blob indexer) - blobscan.com");
    println!("=======================================================");
    if num_threads == 0 {
        println!("Number of threads: auto");
    } else {
        println!("Number of threads: {}", num_threads);
    }
    println!("Slot chunk size: {}", args.slots_per_save);
    println!("Blobscan API endpoint: {}", env.blobscan_api_endpoint);
    println!("CL endpoint: {}", env.beacon_node_endpoint);
    println!("EL endpoint: {}", env.execution_node_endpoint);
    println!("Sentry DSN: {}", sentry_dsn.unwrap_or_default());
    println!("\n");
}

pub async fn run(env: Environment) -> Result<()> {
    let args = Args::parse();

    let slots_processor_config = args
        .num_threads
        .map(|threads_length| SlotsProcessorConfig { threads_length });

    print_banner(&args, &env);

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

            match current_slot.cmp(&latest_slot) {
                Ordering::Less => {
                    let mut unprocessed_slots = latest_slot - current_slot;
                    let plural_suffix = if unprocessed_slots > 1 { "s" } else { "" };

                    info!(
                        target = "indexer",
                        current_slot,
                        latest_slot,
                        "Syncing {unprocessed_slots} slot{plural_suffix}…"
                    );

                    while unprocessed_slots > 0 {
                        let slots_chunk = min(unprocessed_slots, args.slots_per_save);
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
                                    "Failed to update latest indexed slot. Retrying in {duration} seconds…"
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
                            "Latest indexed slot updated"
                        );

                        current_slot += slots_chunk;
                        unprocessed_slots -= slots_chunk;
                    }
                }
                Ordering::Greater => {
                    let err = anyhow!(
                        "Current indexer slot ({current_slot}) is greater than head slot ({latest_slot})"
                    );

                    error!(
                        target = "indexer",
                        current_slot,
                        latest_slot,
                        "{}",
                        err.to_string()
                    );

                    return Err(err);
                }
                Ordering::Equal => (),
            }
        }

        thread::sleep(Duration::from_secs(10));
    }
}
