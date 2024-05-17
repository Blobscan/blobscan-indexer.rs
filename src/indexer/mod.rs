use std::thread;

use anyhow::{anyhow, Context as AnyhowContext};

use futures::StreamExt;
use reqwest_eventsource::Event;
use tokio::{sync::mpsc, task::JoinHandle};
use tracing::{debug, error, info, warn, Instrument};

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
    indexer::error::{
        ChainReorgedEventHandlingError, FinalizedBlockEventHandlingError,
        HeadBlockEventHandlingError, HistoricalSyncingError,
    },
    synchronizer::SynchronizerBuilder,
    utils::web3::get_full_hash,
};

use self::{
    error::{IndexerError, RealtimeSyncingError},
    types::{IndexerResult, IndexerTaskMessage},
};

pub mod error;
pub mod types;

pub struct Indexer {
    context: Context,
    synchronizer_builder: SynchronizerBuilder,
    dencun_fork_slot: u32,
    disable_sync_historical: bool,
}

impl Indexer {
    pub fn try_new(env: &Environment, args: &Args) -> IndexerResult<Self> {
        let context = match Context::try_new(ContextConfig::from(env)) {
            Ok(c) => c,
            Err(error) => {
                error!(?error, "Failed to create context");

                return Err(IndexerError::CreationFailure(anyhow!(
                    "Failed to create context: {:?}",
                    error
                )));
            }
        };

        let slots_checkpoint = args.slots_per_save;
        let num_threads = match args.num_threads {
            Some(num_threads) => num_threads,
            None => thread::available_parallelism()
                .map_err(|err| {
                    IndexerError::CreationFailure(anyhow!(
                        "Failed to get number of available threads: {:?}",
                        err
                    ))
                })?
                .get() as u32,
        };
        let disable_sync_checkpoint_save = args.disable_sync_checkpoint_save;
        let disable_sync_historical = args.disable_sync_historical;

        let dencun_fork_slot = env
            .dencun_fork_slot
            .unwrap_or(env.network_name.dencun_fork_slot());

        let mut synchronizer_builder = SynchronizerBuilder::new();

        synchronizer_builder.with_disable_checkpoint_save(disable_sync_checkpoint_save);
        synchronizer_builder.with_num_threads(num_threads);

        if let Some(slots_checkpoint) = slots_checkpoint {
            synchronizer_builder.with_slots_checkpoint(slots_checkpoint);
        }

        Ok(Self {
            context,
            synchronizer_builder,
            dencun_fork_slot,
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
                error!(?error, "Failed to fetch blobscan's sync state");

                return Err(IndexerError::BlobscanSyncStateRetrievalError(error));
            }
        };

        let current_lower_block_id = match start_block_id.clone() {
            Some(block_id) => block_id,
            None => match &sync_state {
                Some(state) => match state.last_lower_synced_slot {
                    Some(slot) => BlockId::Slot(slot - 1),
                    None => match state.last_upper_synced_slot {
                        Some(slot) => BlockId::Slot(slot - 1),
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
            ?current_lower_block_id,
            ?current_upper_block_id,
            "Starting indexer…",
        );

        let (tx, mut rx) = mpsc::channel(32);
        let tx1 = tx.clone();
        let mut total_tasks = 0;

        if end_block_id.is_none() {
            self._start_realtime_syncing_task(tx, current_upper_block_id);
            total_tasks += 1;
        }

        let end_block_id = end_block_id.unwrap_or(BlockId::Slot(self.dencun_fork_slot - 1));
        let historical_sync_completed =
            matches!(current_lower_block_id, BlockId::Slot(slot) if slot < self.dencun_fork_slot);

        if !self.disable_sync_historical && !historical_sync_completed {
            self._start_historical_syncing_task(tx1, current_lower_block_id, end_block_id);

            total_tasks += 1;
        }

        let mut completed_tasks = 0;

        while let Some(message) = rx.recv().await {
            match message {
                IndexerTaskMessage::Done => {
                    completed_tasks += 1;

                    if completed_tasks == total_tasks {
                        return Ok(());
                    }
                }
                IndexerTaskMessage::Error(error) => {
                    error!(?error, "An error occurred while running a syncing task");

                    return Err(error.into());
                }
            }
        }

        Ok(())
    }

    fn _start_historical_syncing_task(
        &self,
        tx: mpsc::Sender<IndexerTaskMessage>,
        start_block_id: BlockId,
        end_block_id: BlockId,
    ) -> JoinHandle<IndexerResult<()>> {
        let mut synchronizer = self.synchronizer_builder.build(self.context.clone());

        tokio::spawn(async move {
            let historical_syc_thread_span = tracing::info_span!("sync:historical");

            let result: Result<(), IndexerError> = async move {
                let result = synchronizer.run(&start_block_id, &end_block_id).await;

                if let Err(error) = result {
                    tx.send(IndexerTaskMessage::Error(
                        HistoricalSyncingError::SynchronizerError(error).into(),
                    ))
                    .await?;
                } else {
                    info!("Historical syncing completed successfully");

                    tx.send(IndexerTaskMessage::Done).await?;
                }

                Ok(())
            }
            .instrument(historical_syc_thread_span)
            .await;

            result?;

            Ok(())
        })
    }

    fn _start_realtime_syncing_task(
        &self,
        tx: mpsc::Sender<IndexerTaskMessage>,
        start_block_id: BlockId,
    ) -> JoinHandle<IndexerResult<()>> {
        let task_context = self.context.clone();
        let mut synchronizer = self.synchronizer_builder.build(self.context.clone());

        tokio::spawn(async move {
            let realtime_sync_task_span = tracing::info_span!("sync:realtime");

            let result: Result<(), RealtimeSyncingError> = async {
                let beacon_client = task_context.beacon_client();
                let blobscan_client = task_context.blobscan_client();
                let topics = vec![
                    Topic::ChainReorg,
                    Topic::Head,
                    Topic::FinalizedCheckpoint,
                ];
                let mut event_source = task_context
                    .beacon_client()
                    .subscribe_to_events(&topics).map_err(RealtimeSyncingError::BeaconEventsSubscriptionError)?;
                let mut is_initial_sync_to_head = true;
                let events = topics
                .iter()
                .map(|topic| topic.into())
                .collect::<Vec<String>>()
                .join(", ");

                info!("Subscribed to beacon events: {events}");

                while let Some(event) = event_source.next().await {
                    match event {
                        Ok(Event::Open) => {
                            debug!("Subscription connection opened")
                        }
                        Ok(Event::Message(event)) => {
                            let event_name = event.event.as_str();

                            match event_name {
                                "chain_reorg" => {
                                    let chain_reorg_span = tracing::info_span!("chain_reorg");

                                      let result: Result<(), ChainReorgedEventHandlingError> =  async {

                                        let reorg_block_data =
                                            serde_json::from_str::<ChainReorgEventData>(&event.data)?;
                                        let slot = reorg_block_data.slot;
                                        let old_head_block = reorg_block_data.old_head_block;
                                        let target_depth = reorg_block_data.depth;

                                        let mut current_reorged_block = old_head_block;
                                        let mut reorged_slots: Vec<u32> = vec![];

                                        for current_depth in 1..=target_depth {
                                            let reorged_block_head = match beacon_client.get_block_header(&BlockId::Hash(current_reorged_block)).await.map_err(|err| ChainReorgedEventHandlingError::BlockRetrievalError(get_full_hash(&current_reorged_block), err))? {
                                                Some(block) => block,
                                                None => {
                                                    warn!(event=event_name, slot=slot, "Found {current_depth} out of {target_depth} reorged blocks only");
                                                    break
                                                }
                                            };

                                            reorged_slots.push(reorged_block_head.header.message.slot);
                                            current_reorged_block = reorged_block_head.header.message.parent_root;
                                        }

                                        let total_updated_slots = blobscan_client.handle_reorged_slots(&reorged_slots).await.map_err(|err| ChainReorgedEventHandlingError::ReorgedHandlingFailure(target_depth, get_full_hash(&old_head_block), err))?;

                                        info!(event=event_name, slot=slot, "Reorganization of depth {target_depth} detected. Found the following reorged slots: {:#?}. Total slots marked as reorged: {total_updated_slots}", reorged_slots);

                                        Ok(())
                                    }.instrument(chain_reorg_span).await;

                                    if let Err(error) = result {
                                        return Err(RealtimeSyncingError::BeaconEventProcessingError(error.into()));
                                    }
                                },
                                "head" => {                                    
                                    let head_span = tracing::info_span!("head_block");

                                    let result: Result<(), HeadBlockEventHandlingError> = async {
                                        let head_block_data =
                                        serde_json::from_str::<HeadEventData>(&event.data)?;


                                    let head_block_id = &BlockId::Slot(head_block_data.slot);
                                    let initial_block_id = if is_initial_sync_to_head {
                                        is_initial_sync_to_head = false;

                                        &start_block_id
                                    } else {
                                        head_block_id
                                    };

                                    synchronizer.run(initial_block_id, &BlockId::Slot(head_block_data.slot + 1)).await?;

                                    Ok(())
                                    }.instrument(head_span).await;

                                    if let Err(error) = result {
                                        return Err(RealtimeSyncingError::BeaconEventProcessingError(error.into()));
                                    }
                                },
                                "finalized_checkpoint" => {
                                    let finalized_checkpoint_span = tracing::info_span!("finalized_checkpoint");

                                    let result: Result<(), FinalizedBlockEventHandlingError> = async move {
                                        let finalized_checkpoint_data =
                                            serde_json::from_str::<FinalizedCheckpointEventData>(
                                                &event.data,
                                            )?;
                                        let block_hash = finalized_checkpoint_data.block;
                                        let last_finalized_block_number = beacon_client
                                            .get_block(&BlockId::Hash(block_hash))
                                            .await.map_err(|err| FinalizedBlockEventHandlingError::BlockRetrievalError(get_full_hash(&block_hash), err))?
                                            .with_context(|| {
                                                anyhow!("Finalized block not found")
                                            })?
                                            .message.body.execution_payload
                                            .with_context(|| {
                                                anyhow!("Finalized block has no execution payload")
                                            })?.block_number;

                                        blobscan_client
                                            .update_sync_state(BlockchainSyncState {
                                                last_lower_synced_slot: None,
                                                last_upper_synced_slot: None,
                                                last_finalized_block: Some(
                                                    last_finalized_block_number
                                                ),
                                            })
                                            .await.map_err(FinalizedBlockEventHandlingError::BlobscanSyncStateUpdateError)?;

                                        info!(finalized_execution_block=last_finalized_block_number, "Finalized checkpoint event received. Updated last finalized block number");

                                        Ok(())
                                    }.instrument(finalized_checkpoint_span).await;

                                    if let Err(error) = result {
                                        return Err(RealtimeSyncingError::BeaconEventProcessingError(error.into()));
                                    }

                                },
                                unexpected_event_id => {
                                    return Err(RealtimeSyncingError::UnexpectedBeaconEvent(unexpected_event_id.to_string()));
                                },
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
            .instrument(realtime_sync_task_span)
            .await;

            if let Err(error) = result {
                tx.send(IndexerTaskMessage::Error(error.into())).await?;
            } else {
                tx.send(IndexerTaskMessage::Done).await?;
            }

            Ok(())
        })
    }
}
