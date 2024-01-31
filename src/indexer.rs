use anyhow::Result as AnyhowResult;
use futures::StreamExt;
use reqwest_eventsource::Event;
use tracing::{debug, error};

use crate::{
    args::Args,
    clients::beacon::types::{
        BlockHeader as BeaconBlockHeader, BlockId, HeadBlockEventData, Topic,
    },
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

        let last_indexed_block_header = self
            ._index_to_target_slot(current_slot, BlockId::Finalized)
            .await?;

        // We disable parallel processing for better handling of possible reorgs
        self.synchronizer.enable_parallel_processing(false);

        let last_indexed_block_header = self
            ._index_to_target_slot(last_indexed_block_header.message.slot, BlockId::Head)
            .await?;
        let mut last_indexed_block_root = last_indexed_block_header.root;
        let slots_processor = SlotsProcessor::new(self.context.clone());

        while let Some(event) = event_source.next().await {
            match event {
                Ok(Event::Open) => debug!(target = "indexer", "Listening for head block events…"),
                Ok(Event::Message(event)) => {
                    let head_block_data = serde_json::from_str::<HeadBlockEventData>(&event.data)?;

                    slots_processor
                        .process_slot(head_block_data.slot, Some(last_indexed_block_root))
                        .await?;
                    self.context
                        .blobscan_client()
                        .update_slot(head_block_data.slot)
                        .await?;

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

    async fn _index_to_target_slot(
        &self,
        initial_slot: u32,
        target_slot: BlockId,
    ) -> AnyhowResult<BeaconBlockHeader> {
        let beacon_client = self.context.beacon_client();
        let mut current_slot = initial_slot;

        loop {
            let target_block_header_result =
                match beacon_client.get_block_header(&target_slot).await {
                    Ok(res) => res,
                    Err(error) => {
                        error!(
                            target = "indexer",
                            ?error,
                            "Failed to fetch beacon target block"
                        );

                        return Err(error.into());
                    }
                };

            if let Some(target_block_header) = target_block_header_result {
                let target_slot = target_block_header.message.slot;

                if current_slot == target_slot {
                    return Ok(target_block_header);
                }

                self.synchronizer.run(current_slot, target_slot).await?;

                current_slot = target_slot;
            }
        }
    }
}
