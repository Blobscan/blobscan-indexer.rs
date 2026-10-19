use std::time::Duration;

use alloy::primitives::B256;
use anyhow::anyhow;
use futures::{FutureExt, StreamExt};
use reqwest_eventsource::Event;
use tokio::{sync::oneshot, task::JoinHandle, time::timeout};
use tracing::{debug, error, info, info_span, warn, Instrument};

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

/// Maximum time to wait for an SSE event before considering the connection stale.
/// Beacon chain slots are ~12s apart, so 120s covers several missed slots.
const SSE_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

const HEAD_BLOCK_PROCESSING_DELAY: Duration = Duration::from_millis(500);

/// Consecutive failed (re)subscriptions tolerated before giving up and letting the
/// process restart. Any received message resets the count.
const SSE_MAX_CONSECUTIVE_FAILURES: u32 = 10;
const SSE_RECONNECT_BASE_DELAY: Duration = Duration::from_secs(2);
const SSE_RECONNECT_MAX_DELAY: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub enum SSEIndexingError {
    #[error("an error ocurred while receiving events from the SSE stream: {0}")]
    ConnectionFailure(#[from] reqwest_eventsource::Error),
    #[error("SSE stream unavailable after {attempts} consecutive failures: {detail}")]
    Unavailable { attempts: u32, detail: String },
    #[error("failed to subscribe to SSE stream: {0}")]
    FailedSubscription(#[source] ClientError),
    #[error("failed to fetch indexer state: {0}")]
    IndexerStateRetrievalError(#[source] ClientError),
    #[error("unexpected event \"{0}\" received")]
    UnknownEvent(String),
    #[error(transparent)]
    EventDeserializationFailure(#[from] serde_json::Error),
    #[error("failed to handle event \"{event}\": {error}")]
    EventHandlingError { event: String, error: anyhow::Error },
}

/// Delay before the `failures`-th consecutive reconnect attempt: 2s, 4s, 8s… capped at 60s.
fn sse_reconnect_delay(failures: u32, base: Duration, max: Duration) -> Duration {
    base.saturating_mul(2u32.saturating_pow(failures.saturating_sub(1)))
        .min(max)
}

/// A connection-level failure the beacon node or a proxy in front of it can cause
/// (restart, 429, 502, an error page with a JSON body): worth resubscribing.
/// Malformed streams are not: they point at a bug, not at the network.
fn is_recoverable_sse_error(error: &reqwest_eventsource::Error) -> bool {
    use reqwest_eventsource::Error;

    matches!(
        error,
        Error::StreamEnded
            | Error::Transport(_)
            | Error::InvalidContentType(..)
            | Error::InvalidStatusCode(..)
    )
}

/// The error message alone ("Invalid header value: \"application/json\"") hides what the
/// server actually answered, so read the status and the start of the body.
async fn describe_sse_error(error: reqwest_eventsource::Error) -> String {
    use reqwest_eventsource::Error;

    let (summary, response) = match error {
        Error::InvalidContentType(_, response) | Error::InvalidStatusCode(_, response) => {
            (format!("HTTP {}", response.status()), Some(response))
        }
        other => (other.to_string(), None),
    };

    match response {
        Some(response) => {
            let body = response.text().await.unwrap_or_default();
            let body: String = body.chars().take(300).collect();

            format!("{summary}, body: {body}")
        }
        None => summary,
    }
}

pub struct RunParams {
    pub last_synced_slot: Option<u32>,
    pub last_synced_block: Option<BlockHeader>,
}

pub struct SSEIndexingTask {
    context: Box<dyn CommonContext>,
    error_report_tx: TaskErrorChannelSender,
    /// (base, max) of the exponential delay between resubscriptions.
    reconnect_delays: (Duration, Duration),
}

impl SSEIndexingTask {
    pub fn new(context: Box<dyn CommonContext>, error_report_tx: TaskErrorChannelSender) -> Self {
        SSEIndexingTask {
            context,
            error_report_tx,
            reconnect_delays: (SSE_RECONNECT_BASE_DELAY, SSE_RECONNECT_MAX_DELAY),
        }
    }

    #[cfg(test)]
    fn with_reconnect_delays(mut self, base: Duration, max: Duration) -> Self {
        self.reconnect_delays = (base, max);
        self
    }

    pub fn run(&self, params: RunParams) -> IndexingTaskJoinHandle {
        let context = self.context.clone();
        let error_report_tx = self.error_report_tx.clone();
        let reconnect_delays = self.reconnect_delays;
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
            let mut consecutive_failures: u32 = 0;

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
                    let mut received_message = false;
                    let head_event_span = info_span!("head");
                    let finalized_event_span =
                        info_span!("finalized_checkpoint");

                    loop {
                        let event = match timeout(SSE_IDLE_TIMEOUT, event_source.next()).await {
                            Ok(Some(event)) => event,
                            // A closed stream is handled like `StreamEnded` below.
                            Ok(None) => Err(reqwest_eventsource::Error::StreamEnded),
                            Err(_) => {
                                warn!("No SSE event received in {}s. Reconnecting…", SSE_IDLE_TIMEOUT.as_secs());
                                event_source.close();

                                if let Some(handle) = catchup_task_handle.take() {
                                    handle.abort();
                                }

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
                            }
                        };

                        match event {
                            Ok(Event::Open) => {
                                debug!("Subscrption connection opened")
                            }
                            Ok(Event::Message(event)) => {
                                consecutive_failures = 0;
                                received_message = true;
                                let event_name = event.event.as_str();

                                match event_name {
                                    "head" => {
                                        let head_block_data =
                                            serde_json::from_str::<HeadEventData>(&event.data)?;
                                        let head_slot = head_block_data.slot;
                                        let head_root = head_block_data.block;

                                        if let Some(result) = catchup_sync_rx
                                            .as_mut()
                                            .and_then(|rx| rx.now_or_never())
                                        {
                                            match result {
                                                Ok(_) => {
                                                    sse_synchronizer
                                                        .set_checkpoint(Some(CheckpointType::Upper));
                                                }
                                                Err(_) => {
                                                    warn!("Catchup task channel closed unexpectedly. Restoring checkpoint.");
                                                    sse_synchronizer
                                                        .set_checkpoint(Some(CheckpointType::Upper));
                                                }
                                            }
                                            catchup_sync_rx = None;
                                        }

                                        if is_first_event {
                                            if let Some(last_sse_synced_slot) = last_sse_synced_slot {
                                                if last_sse_synced_slot < head_slot.saturating_sub(1) {
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
                                                        from_slot: last_sse_synced_slot + 1,
                                                        to_slot: head_slot,
                                                        prev_block: last_sse_synced_block.clone(),
                                                        checkpoint: Some(CheckpointType::Upper),
                                                    }));


                                                    catchup_sync_rx = Some(channel_rx);

                                                    sse_synchronizer.set_checkpoint(None);
                                                    sse_synchronizer.set_last_synced_block(None);
                                                }
                                            }
                                        }

                                        // The head SSE event may arrive before the block header is
                                        // queryable via the REST API — either due to a race within
                                        // the beacon node itself, or because requests are hitting a
                                        // different node in a load-balanced setup. We wait briefly
                                        // before the first attempt and retry on `None` to handle both.
                                        tokio::time::sleep(HEAD_BLOCK_PROCESSING_DELAY).await;

                                        sse_synchronizer
                                            .sync_block(head_root.into())
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

                                if !is_recoverable_sse_error(&error) {
                                    return Err(error.into());
                                }

                                // A stream that ends after delivering events is routine. One that
                                // ends before delivering any is a failure, or this would resubscribe
                                // in a tight loop.
                                if matches!(error, reqwest_eventsource::Error::StreamEnded) && received_message {
                                    info!("SSE stream ended. Resubscribing to stream…");
                                } else {
                                    consecutive_failures += 1;

                                    let detail = describe_sse_error(error).await;

                                    if consecutive_failures > SSE_MAX_CONSECUTIVE_FAILURES {
                                        return Err(SSEIndexingError::Unavailable {
                                            attempts: consecutive_failures,
                                            detail,
                                        });
                                    }

                                    let delay = sse_reconnect_delay(consecutive_failures, reconnect_delays.0, reconnect_delays.1);

                                    warn!(
                                        attempt = consecutive_failures,
                                        "SSE connection failed ({detail}). Resubscribing in {}s…",
                                        delay.as_secs()
                                    );

                                    tokio::time::sleep(delay).await;
                                }

                                {
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
                                }
                            }
                        }
                    }

                    Ok(())
                }.instrument(sse_indexing_span.clone())
                .await;

                if let Err(error) = result {
                    if let Err(send_err) = error_report_tx
                        .send(ErrorResport {
                            task_name: "sse-indexing".into(),
                            error: error.into(),
                        })
                        .await
                    {
                        error!("SSE indexing task failed: Failed to send error report: {send_err}");
                    }

                    break;
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_delay_doubles_and_is_capped() {
        let secs =
            |n| sse_reconnect_delay(n, SSE_RECONNECT_BASE_DELAY, SSE_RECONNECT_MAX_DELAY).as_secs();

        assert_eq!(secs(1), 2);
        assert_eq!(secs(2), 4);
        assert_eq!(secs(3), 8);
        assert_eq!(secs(5), 32);
        assert_eq!(secs(6), 60);
        assert_eq!(secs(u32::MAX), 60);
    }

    #[test]
    fn only_connection_level_errors_are_recoverable() {
        use reqwest_eventsource::Error;

        assert!(is_recoverable_sse_error(&Error::StreamEnded));
        assert!(!is_recoverable_sse_error(&Error::InvalidLastEventId(
            "x".into()
        )));
    }

    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use alloy::{
        network::Ethereum,
        providers::{Provider, ProviderBuilder},
        transports::mock::Asserter,
    };
    use reqwest_eventsource::EventSource;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::mpsc,
    };

    use crate::{
        clients::{beacon::MockCommonBeaconClient, blobscan::MockCommonBlobscanClient},
        context::SyncingSettings,
        indexer::types::TaskErrorChannelReceiver,
        network::{Network, NetworkName},
    };

    #[derive(Clone)]
    struct TestContext {
        beacon: Arc<MockCommonBeaconClient>,
        blobscan: Arc<MockCommonBlobscanClient>,
        provider: Arc<dyn Provider<Ethereum>>,
        network: Network,
        settings: Arc<SyncingSettings>,
    }

    impl CommonContext for TestContext {
        fn beacon_client(&self) -> &dyn crate::clients::beacon::CommonBeaconClient {
            self.beacon.as_ref()
        }

        fn blobscan_client(&self) -> &dyn crate::clients::blobscan::CommonBlobscanClient {
            self.blobscan.as_ref()
        }

        fn network(&self) -> &Network {
            &self.network
        }

        fn provider(&self) -> &dyn Provider<Ethereum> {
            self.provider.as_ref()
        }

        fn syncing_settings(&self) -> &SyncingSettings {
            &self.settings
        }
    }

    /// One scripted answer of the fake beacon node: status, content type, body and whether the
    /// connection then stays open (a healthy event stream does).
    type Reply = (u16, &'static str, &'static str, bool);

    const BAD_CONTENT_TYPE: Reply = (200, "application/json", "{\"message\":\"oops\"}", false);
    const UNAVAILABLE: Reply = (503, "application/json", "{\"message\":\"down\"}", false);
    const HEALTHY: Reply = (200, "text/event-stream", ": hello\n\n", true);

    /// Serves `replies` to successive connections (the last one repeats).
    async fn fake_beacon_node(replies: Vec<Reply>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/events", listener.local_addr().unwrap());
        let served = AtomicUsize::new(0);

        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let n = served.fetch_add(1, Ordering::SeqCst);
                let (status, content_type, body, keep_open) = replies[n.min(replies.len() - 1)];
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await;

                let head = if keep_open {
                    format!("HTTP/1.1 {status} X\r\ncontent-type: {content_type}\r\n\r\n{body}")
                } else {
                    format!(
                        "HTTP/1.1 {status} X\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };
                let _ = socket.write_all(head.as_bytes()).await;

                if keep_open {
                    tokio::spawn(async move {
                        let _socket = socket;
                        std::future::pending::<()>().await;
                    });
                }
            }
        });

        url
    }

    /// Runs the SSE task against `replies`. Returns the receiver of subscription
    /// notifications and the error reports, plus the task handle.
    async fn run_task(
        replies: Vec<Reply>,
    ) -> (
        mpsc::UnboundedReceiver<()>,
        TaskErrorChannelReceiver,
        IndexingTaskJoinHandle,
    ) {
        let url = fake_beacon_node(replies).await;
        let (subscribed_tx, subscribed_rx) = mpsc::unbounded_channel();
        let mut beacon = MockCommonBeaconClient::new();

        beacon.expect_subscribe_to_events().returning(move |_| {
            subscribed_tx.send(()).unwrap();

            Ok(EventSource::new(reqwest::Client::new().get(url.clone())).unwrap())
        });

        let mut blobscan = MockCommonBlobscanClient::new();

        blobscan
            .expect_get_sync_state()
            .returning(|| Box::pin(std::future::ready(Ok(None))));

        let context = TestContext {
            beacon: Arc::new(beacon),
            blobscan: Arc::new(blobscan),
            provider: Arc::new(ProviderBuilder::new().connect_mocked_client(Asserter::new())),
            network: Network {
                name: NetworkName::Devnet,
                chain_id: 1,
                dencun_fork_slot: 0,
                epoch: 0,
            },
            settings: Arc::new(SyncingSettings {
                concurrency: 1,
                checkpoint_size: 1,
                disable_checkpoints: true,
            }),
        };
        let (error_tx, error_rx) = mpsc::channel(8);
        let handle = SSEIndexingTask::new(Box::new(context), error_tx)
            .with_reconnect_delays(Duration::from_millis(1), Duration::from_millis(4))
            .run(RunParams {
                last_synced_slot: None,
                last_synced_block: None,
            });

        (subscribed_rx, error_rx, handle)
    }

    #[tokio::test]
    async fn resubscribes_after_connection_failures_instead_of_exiting() {
        let (mut subscribed, mut errors, handle) =
            run_task(vec![BAD_CONTENT_TYPE, UNAVAILABLE, HEALTHY]).await;

        for attempt in 1..=3 {
            subscribed
                .recv()
                .await
                .unwrap_or_else(|| panic!("no subscription attempt {attempt}"));
        }

        assert!(errors.try_recv().is_err(), "the task reported an error");
        assert!(!handle.is_finished(), "the task exited");

        handle.abort();
    }

    #[tokio::test]
    async fn gives_up_after_too_many_consecutive_failures() {
        let (mut subscribed, mut errors, _handle) = run_task(vec![UNAVAILABLE]).await;

        let report = errors.recv().await.expect("an error report");
        let message = report.error.to_string();

        assert!(message.contains("consecutive failures"), "{message}");
        assert!(message.contains("503"), "{message}");

        let mut attempts = 0;

        while subscribed.try_recv().is_ok() {
            attempts += 1;
        }

        assert_eq!(attempts, SSE_MAX_CONSECUTIVE_FAILURES as usize + 1);
    }
}
