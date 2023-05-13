use anyhow::Result;
use tracing::Instrument;

use crate::{
    context::create_context,
    slot_processor::SlotProcessor,
    utils::telemetry::{get_subscriber, init_subscriber},
};
use std::{thread, time::Duration};

mod beacon_client;
mod blobscan_client;
mod context;
mod env;
mod slot_processor;
mod utils;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();

    let sentry_dsn = dotenv::var("SENTRY_DSN").unwrap();
    let _guard = sentry::init((
        sentry_dsn,
        sentry::ClientOptions {
            release: sentry::release_name!(),
            ..Default::default()
        },
    ));

    let subscriber = get_subscriber("blobscan_indexer".into(), "info".into(), std::io::stdout);
    init_subscriber(subscriber);

    let context = create_context()?;
    let mut slot_processor = SlotProcessor::try_init(&context, None).await?;
    let mut current_slot = match context.blobscan_client.get_slot().await? {
        Some(last_slot) => last_slot + 1,
        None => 0,
    };

    loop {
        if let Some(latest_beacon_block) = context.beacon_client.get_block(None).await? {
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
        thread::sleep(Duration::from_secs(10));
    }
}
