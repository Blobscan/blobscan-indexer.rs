use std::{cmp, thread};

use anyhow::{anyhow, Context as AnyhowContext};

use futures::StreamExt;
use reqwest_eventsource::Event;
use tokio::{sync::mpsc, task::JoinHandle};
use tracing::{debug, error, info, warn};

use crate::{
    args::Args,
    clients::{
        beacon::types::{
            BlockId, ChainReorgEventData, FinalizedCheckpointEventData, HeadEventData, Topic,
        },
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
    dencun_fork_slot: u32,
    num_threads: u32,
    slots_checkpoint: Option<u32>,
    disable_sync_checkpoint_save: bool,
    disable_sync_historical: bool,
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

        let slots_checkpoint = args.slots_per_save;
        let num_threads = match args.num_threads {
            Some(num_threads) => num_threads,
            None => thread::available_parallelism()
                .map_err(|err| anyhow!("Failed to get number of available threads: {:?}", err))?
                .get() as u32,
        };
        let disable_sync_checkpoint_save = args.disable_sync_checkpoint_save;
        let disable_sync_historical = args.disable_sync_historical;

        let dencun_fork_slot = env
            .dencun_fork_slot
            .unwrap_or(env.network_name.dencun_fork_slot());

        Ok(Self {
            context,
            num_threads,
            slots_checkpoint,
            dencun_fork_slot,
            disable_sync_checkpoint_save,
            disable_sync_historical,
        })
    }

    pub async fn run(
        &mut self,
        start_block_id: Option<BlockId>,
        end_block_id: Option<BlockId>,
    ) -> IndexerResult<()> {
        let sync_state = match self.context.blobscan_client().get_sync_state().await {
            Ok(state) => state,
            Err(error) => {
                error!(target = "indexer", ?error, "Failed to fetch sync state");

                return Err(error.into());
            }
        };

        let current_lower_block_id = match start_block_id.clone() {
            Some(block_id) => block_id,
            None => match &sync_state {
                Some(state) => match state.last_lower_synced_slot {
                    Some(slot) => BlockId::Slot(self._get_current_lower_slot(slot)),
                    None => match state.last_upper_synced_slot {
                        Some(slot) => BlockId::Slot(self._get_current_lower_slot(slot)),
                        None => BlockId::Head,
                    },
                },
                None => BlockId::Head,
            },
        };
        let current_upper_block_id = match start_block_id {
            Some(block_id) => block_id,
            None => match &sync_state {
                Some(state) => match state.last_upper_synced_slot {
                    Some(slot) => BlockId::Slot(slot + 1),
                    None => match state.last_lower_synced_slot {
                        Some(slot) => BlockId::Slot(slot + 1),
                        None => BlockId::Head,
                    },
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
        let mut total_tasks = 0;

        if end_block_id.is_none() {
            self._start_realtime_sync_task(tx, current_upper_block_id);
            total_tasks += 1;
        }

        if !self.disable_sync_historical {
            let historical_start_block_id = current_lower_block_id;
            let historical_end_block_id = match end_block_id {
                Some(block_id) => block_id,
                None => BlockId::Slot(self.dencun_fork_slot),
            };

            self._start_historical_sync_task(
                tx1,
                historical_start_block_id,
                historical_end_block_id,
            );

            total_tasks += 1;
        }

        let mut completed_tasks = 0;

        while let Some(message) = rx.recv().await {
            match message {
                IndexerTaskResult::Done(task_name) => {
                    info!(target = "indexer", task = task_name, "Task completed.");

                    completed_tasks += 1;

                    if completed_tasks == total_tasks {
                        return Ok(());
                    }
                }
                IndexerTaskResult::Error(error) => {
                    error!(target = "indexer", ?error, "Indexer error occurred");

                    return Err(error.into());
                }
            }
        }

        Ok(())
    }

    fn _start_historical_sync_task(
        &self,
        tx: mpsc::Sender<IndexerTaskResult>,
        start_block_id: BlockId,
        end_block_id: BlockId,
    ) -> JoinHandle<IndexerResult<()>> {
        let task_name = "historical_sync".to_string();
        let mut synchronizer = self._create_synchronizer();

        tokio::spawn(async move {
            let result = synchronizer.run(&start_block_id, &end_block_id).await;

            if let Err(error) = result {
                // TODO: Find a better way to handle this error
                tx.send(IndexerTaskResult::Error(
                    IndexingTaskError::FailedIndexingTask {
                        task_name,
                        error: error.into(),
                    },
                ))
                .await
                .unwrap();
            } else {
                tx.send(IndexerTaskResult::Done(task_name)).await.unwrap();
            }

            Ok(())
        })
    }

    fn _start_realtime_sync_task(
        &self,
        tx: mpsc::Sender<IndexerTaskResult>,
        start_block_id: BlockId,
    ) -> JoinHandle<IndexerResult<()>> {
        let task_name = "realtime_sync".to_string();
        let target = format!("indexer:{task_name}");
        let task_context = self.context.clone();
        let mut synchronizer = self._create_synchronizer();

        tokio::spawn(async move {
            let result: Result<(), IndexerError> = async {
                let beacon_client = task_context.beacon_client();
                let blobscan_client = task_context.blobscan_client();
                let topics = vec![
                    Topic::ChainReorg,
                    Topic::Head,
                    Topic::FinalizedCheckpoint,
                ];
                let mut event_source = task_context
                    .beacon_client()
                    .subscribe_to_events(&topics)?;
                let mut is_initial_sync_to_head = true;

                while let Some(event) = event_source.next().await {
                    match event {
                        Ok(Event::Open) => {
                            let events = topics
                                .iter()
                                .map(|topic| topic.into())
                                .collect::<Vec<String>>()
                                .join(", ");
                            debug!(target, events, "Listening to beacon events…")
                        }
                        Ok(Event::Message(event)) => {
                            let event_name = event.event.as_str();

                            match event_name {
                                "chain_reorg" => {
                                    let reorg_block_data =
                                        serde_json::from_str::<ChainReorgEventData>(&event.data)?;
                                    let slot = reorg_block_data.slot;
                                    let old_head_block = reorg_block_data.old_head_block;
                                    let target_depth = reorg_block_data.depth;

                                    let mut current_reorged_block = old_head_block;
                                    let mut reorged_slots: Vec<u32> = vec![];

                                    for current_depth in 1..=target_depth {
                                        let reorged_block_head = match beacon_client.get_block_header(&BlockId::Hash(current_reorged_block)).await? {
                                            Some(block) => block,
                                            None => {
                                                warn!(target, event=event_name, slot=slot, "Found {current_depth} out of {target_depth} reorged blocks only");
                                                break
                                            }
                                        };

                                        reorged_slots.push(reorged_block_head.header.message.slot);
                                        current_reorged_block = reorged_block_head.header.message.parent_root;
                                    }

                                    let total_updated_slots = blobscan_client.handle_reorged_slots(&reorged_slots).await?;

                                    info!(target, event=event_name, slot=slot, "Reorganization of depth {target_depth} detected. Found the following reorged slots: {:#?}. Total slots marked as reorged: {total_updated_slots}", reorged_slots);
                                },
                                "head" => {
                                    let head_block_data =
                                        serde_json::from_str::<HeadEventData>(&event.data)?;

                                    let head_block_id = &BlockId::Slot(head_block_data.slot);
                                    let initial_block_id = if is_initial_sync_to_head {
                                        is_initial_sync_to_head = false;
                                        &start_block_id
                                    } else {
                                        head_block_id
                                    };

                                    synchronizer.run(initial_block_id, head_block_id).await?;
                                }
                                "finalized_checkpoint" => {
                                    let finalized_checkpoint_data =
                                        serde_json::from_str::<FinalizedCheckpointEventData>(
                                            &event.data,
                                        )?;
                                    let block_hash = finalized_checkpoint_data.block;
                                    let full_block_hash = format!("0x{:x}", block_hash);
                                    let last_finalized_block_number = beacon_client
                                        .get_block(&BlockId::Hash(block_hash))
                                        .await?
                                        .with_context(|| {
                                            anyhow!("Finalized block with hash {full_block_hash} not found")
                                        })?
                                        .message.body.execution_payload
                                        .with_context(|| {
                                            anyhow!("Finalized block with hash {full_block_hash} has no execution payload")
                                        })?.block_number;

                                    blobscan_client
                                        .update_sync_state(BlockchainSyncState {
                                            last_lower_synced_slot: None,
                                            last_upper_synced_slot: None,
                                            last_finalized_block: Some(
                                                last_finalized_block_number
                                            ),
                                        })
                                        .await?;

                                    info!(target, event=event_name, execution_block=last_finalized_block_number, "New finalized block detected");
                                },
                                unexpected_event_id => {
                                    return Err(IndexerError::UnexpectedEvent { event: unexpected_event_id.to_string() })
                                }
                            }
                        },
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
                tx.send(IndexerTaskResult::Error(
                    IndexingTaskError::FailedIndexingTask { task_name, error },
                ))
                .await
                .unwrap();
            } else {
                tx.send(IndexerTaskResult::Done(task_name)).await.unwrap();
            }

            Ok(())
        })
    }

    fn _create_synchronizer(&self) -> Synchronizer {
        let mut synchronizer_builder = SynchronizerBuilder::new();

        synchronizer_builder.with_disable_checkpoint_save(self.disable_sync_checkpoint_save);

        synchronizer_builder.with_num_threads(self.num_threads);

        if let Some(slots_checkpoint) = self.slots_checkpoint {
            synchronizer_builder.with_slots_checkpoint(slots_checkpoint);
        }

        synchronizer_builder.build(self.context.clone())
    }

    fn _get_current_lower_slot(&self, last_synced_slot: u32) -> u32 {
        cmp::max(last_synced_slot, self.dencun_fork_slot)
    }
}
