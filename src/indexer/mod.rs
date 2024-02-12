use std::thread;

use anyhow::anyhow;
use futures::StreamExt;
use reqwest_eventsource::Event;
use tokio::{sync::mpsc, task::JoinHandle};
use tracing::{debug, error, info};

use crate::{
    args::Args,
    clients::{
        beacon::types::{BlockId, HeadBlockEventData, Topic},
        blobscan::types::BlockchainSyncState,
    },
    context::{Config as ContextConfig, Context},
    env::Environment,
    synchronizer::{Synchronizer, SynchronizerBuilder},
};

use self::{
    error::{IndexerError, IndexingTaskError},
    types::{IndexerResult, IndexerTaskResult},
};

pub mod error;
pub mod types;

pub struct Indexer {
    context: Context,
    num_threads: u32,
    slots_checkpoint: Option<u32>,
}

impl Indexer {
    pub fn try_new(env: &Environment, args: &Args) -> IndexerResult<Self> {
        let context = match Context::try_new(ContextConfig::from(env)) {
            Ok(c) => c,
            Err(error) => {
                error!(target = "indexer", ?error, "Failed to create context");

                return Err(error.into());
            }
        };
        let num_threads = match args.num_threads {
            Some(num_threads) => num_threads,
            None => thread::available_parallelism()
                .map_err(|err| anyhow!("Failed to get number of available threads: {:?}", err))?
                .get() as u32,
        };

        Ok(Self {
            context,
            num_threads,
            slots_checkpoint: args.slots_per_save,
        })
    }

    pub async fn run(&mut self, custom_start_block_id: Option<BlockId>) -> IndexerResult<()> {
        let sync_state = match self.context.blobscan_client().get_sync_state().await {
            Ok(state) => state,
            Err(error) => {
                error!(target = "indexer", ?error, "Failed to fetch sync state");

                return Err(error.into());
            }
        };

        let current_lower_block_id = match custom_start_block_id.clone() {
            Some(block_id) => block_id,
            None => match &sync_state {
                Some(state) => match state.last_lower_synced_slot {
                    Some(slot) => BlockId::Slot(slot - 1),
                    None => BlockId::Head,
                },
                None => BlockId::Head,
            },
        };
        let current_upper_block_id = match custom_start_block_id {
            Some(block_id) => block_id,
            None => match &sync_state {
                Some(state) => match state.last_upper_synced_slot {
                    Some(slot) => BlockId::Slot(slot + 1),
                    None => BlockId::Head,
                },
                None => BlockId::Head,
            },
        };

        info!(
            target = "indexer",
            ?current_lower_block_id,
            ?current_upper_block_id,
            "Starting indexer…",
        );

        let (tx, mut rx) = mpsc::channel(32);
        let tx1 = tx.clone();

        self._start_historical_sync_task(tx1, current_lower_block_id);
        self._start_realtime_sync_task(tx, current_upper_block_id);

        while let Some(message) = rx.recv().await {
            if let Err(error) = message {
                error!(target = "indexer", ?error, "Indexer error occurred");

                return Err(error.into());
            }
        }

        Ok(())
    }

    fn _start_historical_sync_task(
        &self,
        tx: mpsc::Sender<IndexerTaskResult>,
        start_block_id: BlockId,
    ) -> JoinHandle<IndexerTaskResult> {
        let mut synchronizer = self._create_synchronizer();

        let handler = tokio::spawn(async move {
            let result = synchronizer.run(&start_block_id, &BlockId::Slot(0)).await;

            if let Err(error) = result {
                // TODO: Find a better way to handle this error
                tx.send(Err(IndexingTaskError::FailedIndexingTask {
                    task_name: "historical_sync".to_string(),
                    error: error.into(),
                }))
                .await
                .unwrap();
            };

            Ok(())
        });

        handler
    }

    fn _start_realtime_sync_task(
        &self,
        tx: mpsc::Sender<IndexerTaskResult>,
        start_block_id: BlockId,
    ) -> JoinHandle<IndexerTaskResult> {
        let task_context = self.context.clone();
        let mut synchronizer = self._create_synchronizer();

        let handler = tokio::spawn(async move {
            let result: Result<(), IndexerError> = async {
                let blobscan_client = task_context.blobscan_client();
                let mut event_source = task_context
                    .beacon_client()
                    .subscribe_to_events(vec![Topic::Head])?;
                let mut is_initial_sync_to_head = true;

                while let Some(event) = event_source.next().await {
                    match event {
                        Ok(Event::Open) => {
                            debug!(target = "indexer", "Listening for head block events…")
                        }
                        Ok(Event::Message(event)) => {
                            let head_block_data =
                                serde_json::from_str::<HeadBlockEventData>(&event.data)?;

                            let head_block_id = &BlockId::Slot(head_block_data.slot);
                            let initial_block_id = if is_initial_sync_to_head {
                                is_initial_sync_to_head = false;
                                &start_block_id
                            } else {
                                head_block_id
                            };

                            synchronizer.run(initial_block_id, head_block_id).await?;

                            blobscan_client
                                .update_sync_state(BlockchainSyncState {
                                    last_lower_synced_slot: None,
                                    last_upper_synced_slot: Some(head_block_data.slot),
                                })
                                .await?;
                        }
                        Err(error) => {
                            event_source.close();

                            return Err(error.into());
                        }
                    }
                }

                Ok(())
            }
            .await;

            if let Err(error) = result {
                // TODO: Find a better way to handle this error
                tx.send(Err(IndexingTaskError::FailedIndexingTask {
                    task_name: "realtime_head_block_sync".to_string(),
                    error: error.into(),
                }))
                .await
                .unwrap();
            };

            Ok(())
        });

        handler
    }

    fn _create_synchronizer(&self) -> Synchronizer {
        let mut synchronizer_builder = SynchronizerBuilder::new();

        synchronizer_builder.with_num_threads(self.num_threads);

        if let Some(slots_checkpoint) = self.slots_checkpoint {
            synchronizer_builder.with_slots_checkpoint(slots_checkpoint);
        }

        synchronizer_builder.build(self.context.clone())
    }
}
