use anyhow::Result as AnyhowResult;
use futures::StreamExt;
use reqwest_eventsource::Event;
use tracing::{debug, error};

use crate::{
    args::Args,
    clients::beacon::types::{BlockId, HeadBlockEventData, Topic},
    context::{Config as ContextConfig, Context},
    env::Environment,
    slots_processor::SlotsProcessor,
    synchronizer::{Synchronizer, SynchronizerBuilder},
};

pub struct Indexer {
    context: Context,
    synchronizer: Synchronizer,
}

impl Indexer {
    pub fn try_new(env: &Environment, args: &Args) -> AnyhowResult<Self> {
        let context = match Context::try_new(ContextConfig::from(env)) {
            Ok(c) => c,
            Err(error) => {
                error!(target = "indexer", ?error, "Failed to create context");

                return Err(error);
            }
        };
        let mut synchronizer_builder = SynchronizerBuilder::new()?;

        if let Some(num_threads) = args.num_threads {
            synchronizer_builder.with_num_threads(num_threads);
        }

        if let Some(slots_checkpoint) = args.slots_per_save {
            synchronizer_builder.with_slots_checkpoint(slots_checkpoint);
        }

        let synchronizer = synchronizer_builder.build(context.clone());

        Ok(Self {
            context,
            synchronizer,
        })
    }

    pub async fn run(&mut self, start_slot: Option<u32>) -> AnyhowResult<()> {
        let beacon_client = self.context.beacon_client();
        let blobscan_client = self.context.blobscan_client();
        let mut event_source = beacon_client.subscribe_to_events(vec![Topic::Head])?;

        let current_slot = match start_slot {
            Some(start_slot) => start_slot,
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

        let finalized_block_header = self
            .synchronizer
            .run(&BlockId::Slot(current_slot), &BlockId::Finalized)
            .await?;

        // We disable parallel processing for better handling of possible reorgs
        self.synchronizer.enable_parallel_processing(false);

        let head_block_header = self
            .synchronizer
            .run(
                &BlockId::Slot(finalized_block_header.header.message.slot),
                &BlockId::Head,
            )
            .await?;

        let mut last_indexed_block_root = head_block_header.root;
        let slots_processor = SlotsProcessor::new(self.context.clone());

        while let Some(event) = event_source.next().await {
            match event {
                Ok(Event::Open) => debug!(target = "indexer", "Listening for head block events…"),
                Ok(Event::Message(event)) => {
                    let head_block_data = serde_json::from_str::<HeadBlockEventData>(&event.data)?;

                    slots_processor
                        .process_slot(head_block_data.slot, Some(last_indexed_block_root))
                        .await?;
                    blobscan_client.update_slot(head_block_data.slot).await?;

                    last_indexed_block_root = head_block_data.block;
                }
                Err(error) => {
                    error!(
                        target = "indexer",
                        ?error,
                        "Failed to receive head block event"
                    );

                    event_source.close();

                    return Err(error.into());
                }
            }
        }

        Ok(())
    }
}
