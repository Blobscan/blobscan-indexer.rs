use anyhow::Result as AnyhowResult;
use futures::StreamExt;
use reqwest_eventsource::Event;
use tracing::{debug, error};

use crate::{
    args::Args,
    clients::{
        beacon::types::{BlockId, HeadBlockEventData, Topic},
        blobscan::types::BlockchainSyncState,
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

    pub async fn run(&mut self, start_block_id: Option<BlockId>) -> AnyhowResult<()> {
        let beacon_client = self.context.beacon_client();
        let blobscan_client = self.context.blobscan_client();
        let mut event_source = beacon_client.subscribe_to_events(vec![Topic::Head])?;

        let sync_state = match blobscan_client.get_synced_state().await {
            Ok(state) => state,
            Err(error) => {
                error!(target = "indexer", ?error, "Failed to fetch sync state");

                return Err(error.into());
            }
        };

        let current_lower_block_id = match &sync_state {
            Some(state) => match state.last_lower_synced_slot {
                Some(slot) => BlockId::Slot(slot - 1),
                None => BlockId::Head,
            },
            None => BlockId::Head,
        };
        let current_upper_block_id = match &sync_state {
            Some(state) => match state.last_upper_synced_slot {
                Some(slot) => BlockId::Slot(slot + 1),
                None => BlockId::Head,
            },
            None => BlockId::Head,
        };

        self.synchronizer
            .run(&current_lower_block_id, &BlockId::Slot(0))
            .await?;

        self.synchronizer
            .run(&current_upper_block_id, &BlockId::Head)
            .await?;

        let mut slots_processor = SlotsProcessor::new(self.context.clone());

        while let Some(event) = event_source.next().await {
            match event {
                Ok(Event::Open) => debug!(target = "indexer", "Listening for head block events…"),
                Ok(Event::Message(event)) => {
                    let head_block_data = serde_json::from_str::<HeadBlockEventData>(&event.data)?;

                    slots_processor
                        .process_slot(head_block_data.slot, Some(true))
                        .await?;
                    blobscan_client
                        .update_sync_state(BlockchainSyncState {
                            last_lower_synced_slot: None,
                            last_upper_synced_slot: Some(head_block_data.slot),
                        })
                        .await?;
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
