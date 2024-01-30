use anyhow::Result as AnyhowResult;
use futures::StreamExt;
use reqwest_eventsource::Event;
use tracing::{debug, error};

use crate::{
    args::Args,
    clients::beacon::types::{Block as BeaconBlock, BlockId, Topic},
    context::{Config as ContextConfig, Context},
    env::Environment,
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

    async fn _index_to_target_block(
        &self,
        target_block_slot: BlockId,
        start_slot: Option<u32>,
    ) -> AnyhowResult<BeaconBlock> {
        let beacon_client = self.context.beacon_client();
        let blobscan_client = self.context.blobscan_client();

        let mut current_slot = match start_slot {
            Some(initial_slot) => initial_slot,
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

        loop {
            let beacon_target_block_result = match beacon_client.get_block(&target_block_slot).await
            {
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

            if let Some(beacon_target_block) = beacon_target_block_result {
                let target_slot: u32 = beacon_target_block.message.slot.parse()?;

                if target_slot == current_slot {
                    return Ok(beacon_target_block);
                }

                self.synchronizer.run(current_slot, target_slot).await?;

                current_slot = target_slot;
            }
        }
    }

    pub async fn run(&mut self, start_slot: Option<u32>) -> AnyhowResult<()> {
        let beacon_client = self.context.beacon_client();

        let target_indexed_beacon_block = self
            ._index_to_target_block(BlockId::Finalized, start_slot)
            .await?;

        // We disable parallel processing for better handling of possible reorgs
        self.synchronizer.enable_parallel_processing(false);

        let last_indexed_beacon_block = self
            ._index_to_target_block(
                BlockId::Head,
                Some(target_indexed_beacon_block.message.slot.parse()?),
            )
            .await?;

        let mut event_source = beacon_client.subscribe_to_events(vec![Topic::Head])?;

        while let Some(event) = event_source.next().await {
            match event {
                Ok(Event::Open) => debug!(target = "indexer", "Listening for head block events…"),
                Ok(Event::Message(message)) => {
                    let head_block_data =
                        serde_json::from_str::<ChainReorgResponse>(&message.data)?;
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
