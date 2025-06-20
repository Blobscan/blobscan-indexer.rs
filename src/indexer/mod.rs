use std::thread;

use alloy::primitives::B256;
use anyhow::anyhow;
use event_handlers::{finalized_checkpoint::FinalizedCheckpointHandler, head::HeadEventHandler};
use futures::StreamExt;
use reqwest_eventsource::Event;
use tokio::{sync::mpsc, task::JoinHandle};
use tracing::{debug, error, info, Instrument};

use crate::{
    args::Args,
    clients::beacon::types::{BlockHeader, BlockId, Topic},
    context::{CommonContext, Config as ContextConfig, Context},
    env::Environment,
    indexer::error::HistoricalIndexingError,
    synchronizer::{CheckpointType, CommonSynchronizer, SynchronizerBuilder},
};

use self::{
    error::{IndexerError, LiveIndexingError},
    types::{IndexerResult, IndexerTaskMessage},
};

pub mod error;
pub mod event_handlers;
pub mod types;

pub struct Indexer {
    context: Box<dyn CommonContext>,
    dencun_fork_slot: u32,
    disable_sync_historical: bool,

    checkpoint_slots: Option<u32>,
    disabled_checkpoint: Option<CheckpointType>,
    num_threads: u32,
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

        let checkpoint_slots = args.slots_per_save;
        let disabled_checkpoint = if args.disable_sync_checkpoint_save {
            Some(CheckpointType::Disabled)
        } else {
            None
        };
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
        let disable_sync_historical = args.disable_sync_historical;

        let dencun_fork_slot = env
            .dencun_fork_slot
            .unwrap_or(env.network_name.dencun_fork_slot());

        Ok(Self {
            context: Box::new(context),
            dencun_fork_slot,
            disable_sync_historical,
            checkpoint_slots,
            disabled_checkpoint,
            num_threads,
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

        let last_synced_block = sync_state.as_ref().and_then(|state| {
            match (
                state.last_upper_synced_block_root,
                state.last_upper_synced_slot,
            ) {
                (Some(root), Some(slot)) => Some(BlockHeader {
                    parent_root: B256::ZERO,
                    root,
                    slot,
                }),
                _ => None,
            }
        });

        info!(
            last_lowest_synced_slot = ?current_lower_block_id,
            last_upper_synced_block_slot = ?last_synced_block.as_ref().map(|block| block.slot),
            last_upper_synced_block_root = ?last_synced_block.as_ref().map(|block| block.root),
            "Starting indexer…",
        );

        let (tx, mut rx) = mpsc::channel(32);
        let tx1 = tx.clone();
        let mut total_tasks = 0;

        if end_block_id.is_none() {
            self.start_live_indexing_task(tx, last_synced_block, start_block_id);
            total_tasks += 1;
        }

        let historical_sync_completed =
            matches!(current_lower_block_id, BlockId::Slot(slot) if slot < self.dencun_fork_slot);

        if !self.disable_sync_historical && !historical_sync_completed {
            let historical_sync_final_block_id =
                end_block_id.unwrap_or(BlockId::Slot(self.dencun_fork_slot - 1));

            self.start_historical_indexing_task(
                tx1,
                current_lower_block_id,
                historical_sync_final_block_id,
            );

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

    fn start_historical_indexing_task(
        &self,
        tx: mpsc::Sender<IndexerTaskMessage>,
        start_block_id: BlockId,
        end_block_id: BlockId,
    ) -> JoinHandle<IndexerResult<()>> {
        let mut synchronizer = self.create_synchronizer(CheckpointType::Lower, None);

        tokio::spawn(async move {
            let historical_syc_thread_span = tracing::info_span!("indexer:historical");

            let result: Result<(), IndexerError> = async move {
                let result = synchronizer.sync_blocks(start_block_id, end_block_id).await;

                if let Err(error) = result {
                    tx.send(IndexerTaskMessage::Error(
                        HistoricalIndexingError::SynchronizerError(error).into(),
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

    fn start_live_indexing_task(
        &self,
        tx: mpsc::Sender<IndexerTaskMessage>,
        last_indexed_block: Option<BlockHeader>,
        start_block_id: Option<BlockId>,
    ) -> JoinHandle<IndexerResult<()>> {
        let task_context = self.context.clone();

        let synchronizer = self.create_synchronizer(CheckpointType::Upper, last_indexed_block);
        let realtime_sync_task_span = tracing::info_span!("indexer:live");

        tokio::spawn(async move {
            let result: Result<(), LiveIndexingError> = async {
                let topics = vec![Topic::Head, Topic::FinalizedCheckpoint];
                let events = topics
                    .iter()
                    .map(|topic| topic.into())
                    .collect::<Vec<String>>()
                    .join(", ");

                let mut head_event_handler = HeadEventHandler::new(synchronizer, start_block_id);
                let finalized_checkpoint_event_handler =
                    FinalizedCheckpointHandler::new(task_context.clone());

                loop {
                    let mut event_source = task_context
                        .beacon_client()
                        .subscribe_to_events(&topics)
                        .map_err(LiveIndexingError::BeaconEventsSubscriptionError)?;

                    info!("Subscribed to beacon SSE stream: {}", events);

                    while let Some(event) = event_source.next().await {
                        match event {
                            Ok(Event::Open) => {
                                debug!("Subscription connection opened");
                            }
                            Ok(Event::Message(event)) => {
                                let event_name = event.event.as_str();

                                match event_name {
                                    "head" => {
                                        head_event_handler
                                            .handle(event.data)
                                            .instrument(tracing::info_span!("head_block"))
                                            .await?;
                                    }
                                    "finalized_checkpoint" => {
                                        finalized_checkpoint_event_handler
                                            .handle(event.data)
                                            .instrument(tracing::info_span!("finalized_checkpoint"))
                                            .await?;
                                    }
                                    unexpected_event_id => {
                                        return Err(LiveIndexingError::UnexpectedBeaconEvent(
                                            unexpected_event_id.to_string(),
                                        ));
                                    }
                                }
                            }
                            Err(error) => {
                                event_source.close();

                                if let reqwest_eventsource::Error::StreamEnded = error {
                                    info!("Beacon node SSE stream ended. Resubscribing to stream…");

                                    break;
                                } else {
                                    return Err(error.into());
                                }
                            }
                        }
                    }
                }
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

    fn create_synchronizer(
        &self,
        checkpoint_type: CheckpointType,
        last_synced_block: Option<BlockHeader>,
    ) -> Box<dyn CommonSynchronizer> {
        let mut synchronizer_builder = SynchronizerBuilder::new();

        if let Some(checkpoint_slots) = self.checkpoint_slots {
            synchronizer_builder.with_slots_checkpoint(checkpoint_slots);
        }

        if let Some(last_synced_block) = last_synced_block {
            synchronizer_builder.with_last_synced_block(last_synced_block);
        }

        let checkpoint_type = self.disabled_checkpoint.unwrap_or(checkpoint_type);

        synchronizer_builder.with_checkpoint_type(checkpoint_type);

        synchronizer_builder.with_num_threads(self.num_threads);

        Box::new(synchronizer_builder.build(self.context.clone()))
    }
}
