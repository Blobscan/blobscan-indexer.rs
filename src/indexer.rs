use anyhow::Result as AnyhowResult;
use tracing::error;

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

    async fn _index(&self, from_slot: u32, to_slot: BlockId) -> AnyhowResult<BeaconBlock> {
        let beacon_client = self.context.beacon_client();

        let mut current_slot = from_slot;

        loop {
            let beacon_target_block_result = match beacon_client.get_block(&to_slot).await {
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
        let blobscan_client = self.context.blobscan_client();

        let start_slot = match start_slot {
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

        let last_indexed_beacon_block = self._index(start_slot, BlockId::Finalized).await?;

        // We disable parallel processing for better handling of possible reorgs
        self.synchronizer.enable_parallel_processing(false);

        let last_indexed_beacon_block = self
            ._index(
                last_indexed_beacon_block.message.slot.parse()?,
                BlockId::Head,
            )
            .await?;

        let event_source = beacon_client.subscribe_to_events(vec![Topic::Head])?;

        Ok(())
    }
}
