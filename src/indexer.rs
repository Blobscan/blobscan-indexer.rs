use std::{thread, time::Duration};

use anyhow::Result as AnyhowResult;
use tracing::error;

use crate::{
    args::Args,
    clients::beacon::types::BlockId,
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

    pub async fn run(&self, from_slot: Option<u32>) -> AnyhowResult<()> {
        let beacon_client = self.context.beacon_client();
        let blobscan_client = self.context.blobscan_client();

        let mut current_slot = match from_slot {
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

        loop {
            let beacon_head_result = match beacon_client.get_block(BlockId::Head).await {
                Ok(res) => res,
                Err(error) => {
                    error!(
                        target = "indexer",
                        ?error,
                        "Failed to fetch beacon head block"
                    );

                    return Err(error.into());
                }
            };

            if let Some(beacon_head_block) = beacon_head_result {
                let head_slot: u32 = beacon_head_block.slot.parse()?;

                self.synchronizer.run(current_slot, head_slot).await?;

                current_slot = head_slot;
            }

            thread::sleep(Duration::from_secs(10));
        }
    }
}
