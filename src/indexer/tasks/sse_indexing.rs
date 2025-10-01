use alloy::primitives::B256;
use anyhow::anyhow;
use futures::{FutureExt, StreamExt};
use reqwest_eventsource::Event;
use tokio::{sync::oneshot, task::JoinHandle};
use tracing::{debug, info, info_span, Instrument};

use crate::{
    clients::{
        beacon::types::{BlockHeader, FinalizedCheckpointEventData, HeadEventData, Topic},
        blobscan::types::BlockchainSyncState,
        common::ClientError,
    },
    context::CommonContext,
    indexer::{
        tasks::indexing::{IndexingTask, RunParams as IndexingRunParams},
        types::{
            ErrorResport, IndexingTaskJoinHandle, TaskErrorChannelSender, TaskResult,
            TaskResultChannelReceiver,
        },
    },
    synchronizer::{CheckpointType, CommonSynchronizer, SynchronizerBuilder},
    utils::alloy::B256Ext,
};

#[derive(Debug, thiserror::Error)]
pub enum SSEIndexingError {
    #[error("an error ocurred while receiving events from the SSE stream")]
    ConnectionFailure(#[from] reqwest_eventsource::Error),
    #[error("failed to subscribe to SSE stream")]
    FailedSubscription(#[source] ClientError),
    #[error("failed to fetch indexer state")]
    IndexerStateRetrievalError(#[source] ClientError),
    #[error("unexpected event \"{0}\" received")]
    UnknownEvent(String),
    #[error(transparent)]
    EventDeserializationFailure(#[from] serde_json::Error),
    #[error("failed to handle event \"{event}\": {error}")]
    EventHandlingError { event: String, error: anyhow::Error },
}

pub struct RunParams {
    pub last_synced_slot: Option<u32>,
    pub last_synced_block: Option<BlockHeader>,
}

pub struct SSEIndexingTask {
    context: Box<dyn CommonContext>,
    error_report_tx: TaskErrorChannelSender,
}

impl SSEIndexingTask {
    pub fn new(context: Box<dyn CommonContext>, error_report_tx: TaskErrorChannelSender) -> Self {
        SSEIndexingTask {
            context,
            error_report_tx,
        }
    }

    pub fn run(&self, params: RunParams) -> IndexingTaskJoinHandle {
        let context = self.context.clone();
        let error_report_tx = self.error_report_tx.clone();
        let last_synced_block = params.last_synced_block;
        let last_synced_slot = params.last_synced_slot;

        tokio::spawn(async move {
            let topics = vec![Topic::Head, Topic::FinalizedCheckpoint];
            let events = topics
                .iter()
                .map(|topic| topic.into())
                .collect::<Vec<String>>()
                .join(", ");
            let sse_indexing_span = info_span!("sse-indexing");
            let mut last_sse_synced_block = last_synced_block;
            let mut last_sse_synced_slot = last_synced_slot;

            loop {
                let result: Result<(), SSEIndexingError> = async {
                    let mut sse_synchronizer_builder = SynchronizerBuilder::default();

                    if let Some(last_synced_block) = last_sse_synced_block.clone() {
                        sse_synchronizer_builder.with_last_synced_block(last_synced_block);
                    }

                    let mut sse_synchronizer = sse_synchronizer_builder.build(context.clone());

                    let mut event_source = context
                        .beacon_client()
                        .subscribe_to_events(&topics)
                        .map_err(SSEIndexingError::FailedSubscription)?;

                    info!("Subscribed to stream events: {}", events);

                    let mut catchup_sync_rx: Option<TaskResultChannelReceiver> = None;
                    let mut catchup_task_handle: Option<JoinHandle<()>> = None;
                    let mut is_first_event = true;
                    let head_event_span = info_span!("head");
                    let finalized_event_span =
                        info_span!("finalized_checkpoint");

                    while let Some(event) = event_source.next().await {
                        match event {
                            Ok(Event::Open) => {
                                debug!("Subscrption connection opened")
                            }
                            Ok(Event::Message(event)) => {
                                let event_name = event.event.as_str();

                                match event_name {
                                    "head" => {
                                        let head_block_data =
                                            serde_json::from_str::<HeadEventData>(&event.data)?;
                                        let head_slot = head_block_data.slot;

                                            if let Some(Ok(_)) = catchup_sync_rx
                                                .as_mut()
                                                .and_then(|rx| rx.now_or_never())
                                            {
                                                sse_synchronizer
                                                    .set_checkpoint(Some(CheckpointType::Upper));
                                                catchup_sync_rx = None;
                                            }


                                        if is_first_event {
                                            if let Some(last_sse_synced_slot) = last_sse_synced_slot {
                                                if last_sse_synced_slot < head_slot - 1 {
                                                    let (channel_tx, channel_rx) =
                                                        oneshot::channel::<TaskResult>();

                                                    let catchup_task = IndexingTask::new(
                                                        "catchup",
                                                        context.clone(),
                                                        Some(info_span!(parent: None, "catchup"))
                                                    );


                                                    catchup_task_handle = Some(catchup_task.run(IndexingRunParams {
                                                        error_report_tx: error_report_tx.clone(),
                                                        result_report_tx: Some(channel_tx),
                                                        from_block_id: (last_sse_synced_slot + 1)
                                                            .into(),
                                                        to_block_id: head_slot.into(),
                                                        prev_block: last_sse_synced_block.clone(),
                                                        checkpoint: Some(CheckpointType::Upper),
                                                    }));


                                                    catchup_sync_rx = Some(channel_rx);

                                                    sse_synchronizer.set_checkpoint(None);
                                                    sse_synchronizer.set_last_synced_block(None);
                                                }
                                            }
                                        }

                                        sse_synchronizer
                                            .sync_block(head_slot.into())
                                            .instrument(head_event_span.clone())
                                            .await
                                            .map_err(|err| {
                                                SSEIndexingError::EventHandlingError {
                                                    event: event.event.clone(),
                                                    error: err.into(),
                                                }
                                            })?;

                                        is_first_event = false;
                                    }
                                    "finalized_checkpoint" => {
                                        async {
                                            let finalized_checkpoint_data = serde_json::from_str::<
                                                FinalizedCheckpointEventData,
                                            >(
                                                &event.data
                                            )?;

                                             let block_hash = finalized_checkpoint_data.block;
                                        let full_block_hash = block_hash.to_full_hex();
                                        let last_finalized_block_number = match
                                            context
                                            .beacon_client()
                                            .get_block(block_hash.into())
                                            .await
                                            .map_err(|err|
                                                SSEIndexingError::EventHandlingError { event: event.event.clone(), error: anyhow!(
                                                    "Failed to retrieve finalized block {full_block_hash}: {err}"
                                                ) }
                                            )? {
                                            Some(block) => match block.execution_payload {
                                                Some(execution_payload) => execution_payload.block_number,
                                                None => {
                                                    return Err(
                                                        SSEIndexingError::EventHandlingError { event: event.event.clone(), error: anyhow!(
                                                    "Finalized block {full_block_hash} not found"
                                                ) },
                                                    )
                                                }
                                            },
                                            None => {
                                                return Err(
                                                    SSEIndexingError::EventHandlingError { event: event.event.clone(), error: anyhow!(
                                                    "Finalized block {full_block_hash} not found"
                                                ) },
                                                )
                                            }
                                        };

                                        context
                                            .blobscan_client()
                                            .update_sync_state(BlockchainSyncState {
                                                last_finalized_block: Some(last_finalized_block_number),
                                                last_lower_synced_slot: None,
                                                last_upper_synced_slot: None,
                                                last_upper_synced_block_root: None,
                                                last_upper_synced_block_slot: None,
                                            })
                                            .await
                                            .map_err(|err| SSEIndexingError::EventHandlingError {
                                                event: event.event,
                                                error: err.into(),
                                            })?;

                                        info!(
                                            finalized_execution_block = last_finalized_block_number,
                                            "Updated last finalized block number"
                                        );

                                            Ok::<_, SSEIndexingError>(())
                                        }
                                        .instrument(finalized_event_span.clone())
                                        .await?;
                                    }
                                    unexpected_event => {
                                        return Err(SSEIndexingError::UnknownEvent(
                                            unexpected_event.into(),
                                        ));
                                    }
                                }
                            }
                            Err(error) => {
                                event_source.close();

                                if let Some(catchup_task_handle) = catchup_task_handle {
                                    catchup_task_handle.abort();
                                }

                                if let reqwest_eventsource::Error::StreamEnded = error {
                                    info!("SSE stream ended. Resubscribing to stream…");

                                    let sync_state = context.blobscan_client().get_sync_state().await.map_err(SSEIndexingError::IndexerStateRetrievalError)?;

                                    last_sse_synced_slot = sync_state.as_ref().and_then(|state| state.last_upper_synced_slot);
                                    last_sse_synced_block = sync_state.as_ref().and_then(|state| {
                                        match (
                                            state.last_upper_synced_block_root,
                                            state.last_upper_synced_block_slot,
                                        ) {
                                            (Some(root), Some(slot)) => Some(BlockHeader {
                                                parent_root: B256::ZERO,
                                                root,
                                                slot,
                                            }),
                                            _ => None,
                                        }
                                    });

                                    break;
                                } else {
                                    return Err(error.into());
                                }
                            }
                        }
                    }

                    Ok(())
                }.instrument(sse_indexing_span.clone())
                .await;

                if let Err(error) = result {
                    error_report_tx
                        .send(ErrorResport {
                            task_name: "sse-indexing".into(),
                            error: error.into(),
                        })
                        .await
                        .unwrap();

                    break;
                }
            }
        })
    }
}
