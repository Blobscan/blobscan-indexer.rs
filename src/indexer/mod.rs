use alloy::primitives::B256;
use tokio::sync::mpsc::{self};
use tracing::{error, info, info_span};

use crate::{
    clients::beacon::types::{BlockHeader, BlockId, BlockIdResolution},
    context::{CommonContext, Context},
    indexer::{
        tasks::{
            indexing::{IndexingTask, RunParams as IndexingTaskRunParams},
            sse_indexing::{RunParams as SSEIndexingTaskRunParams, SSEIndexingTask},
        },
        types::{
            ErrorResport, IndexingTaskJoinHandle, TaskErrorChannelReceiver, TaskErrorChannelSender,
        },
    },
    synchronizer::{CheckpointType, CommonSynchronizer, SynchronizerBuilder},
};

use self::error::IndexerError;

pub mod error;
pub mod tasks;
pub mod types;

pub struct Indexer {
    context: Box<dyn CommonContext>,
    disable_backfill: bool,

    error_report_tx: TaskErrorChannelSender,
    error_report_rx: TaskErrorChannelReceiver,
}

pub type IndexerResult<T> = Result<T, IndexerError>;

impl Indexer {
    pub fn new(context: Context, disable_backfill: bool) -> Self {
        let (error_report_tx, error_report_rx) = mpsc::channel::<ErrorResport>(32);

        Self {
            context: Box::new(context),
            disable_backfill,
            error_report_rx,
            error_report_tx,
        }
    }

    pub async fn index_from(&mut self, from_block_id: BlockId) -> IndexerResult<()> {
        let slot = from_block_id
            .resolve_to_slot(self.context.beacon_client())
            .await?;

        self.start_sse_listening_task(SSEIndexingTaskRunParams {
            last_synced_block: None,
            last_synced_slot: Some(slot),
        })
        .await
        .unwrap();

        Ok(())
    }

    pub async fn index_block_range(
        &mut self,
        from_block_id: BlockId,
        to_block_id: BlockId,
    ) -> IndexerResult<()> {
        let mut builder = SynchronizerBuilder::new();

        builder.with_checkpoint(None);

        let mut synchronizer = builder.build(self.context.clone());

        synchronizer.sync_blocks(from_block_id, to_block_id).await?;

        Ok(())
    }
    pub async fn index(&mut self) -> IndexerResult<()> {
        let sync_state = match self.context.blobscan_client().get_sync_state().await {
            Ok(state) => state,
            Err(error) => {
                error!(?error, "Failed to fetch blobscan's sync state");

                return Err(IndexerError::IndexerStateRetrievalError(error));
            }
        };
        let lowest_synced_slot = sync_state
            .as_ref()
            .and_then(|state| state.last_lower_synced_slot);
        let last_synced_block = sync_state.as_ref().and_then(|state| {
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
        let last_synced_slot = sync_state
            .as_ref()
            .and_then(|state| state.last_upper_synced_slot);

        info!(
            lowest_synced_slot = ?lowest_synced_slot,
            last_synced_block_slot = ?last_synced_block.as_ref().map(|block| block.slot),
            last_synced_block_root = ?last_synced_block.as_ref().map(|block| block.root),
            "Starting indexer…",
        );

        let dencun_fork_slot = self.context.network().dencun_fork_slot;
        let backfill_completed = lowest_synced_slot.is_some_and(|slot| slot <= dencun_fork_slot);

        if !self.disable_backfill && !backfill_completed {
            let task = IndexingTask::new(
                "backfill",
                self.context.clone(),
                Some(info_span!("backfill")),
            );

            let current_lowest_block_id = match lowest_synced_slot {
                Some(lowest_synced_slot) => lowest_synced_slot.saturating_sub(1).into(),
                None => match last_synced_slot {
                    Some(last_synced_slot) => last_synced_slot.saturating_sub(1).into(),
                    None => BlockId::Head,
                },
            };

            task.run(IndexingTaskRunParams {
                error_report_tx: self.error_report_tx.clone(),
                result_report_tx: None,
                from_block_id: current_lowest_block_id,
                to_block_id: dencun_fork_slot.into(),
                prev_block: None,
                checkpoint: Some(CheckpointType::Lower),
            });
        }

        self.start_sse_listening_task(SSEIndexingTaskRunParams {
            last_synced_block,
            last_synced_slot,
        });

        if let Some(error_report) = self.error_report_rx.recv().await {
            return Err(IndexerError::IndexingTaskError {
                task_name: error_report.task_name,
                error: error_report.error,
            });
        }

        Ok(())
    }

    fn start_sse_listening_task(&self, params: SSEIndexingTaskRunParams) -> IndexingTaskJoinHandle {
        let task = SSEIndexingTask::new(self.context.clone(), self.error_report_tx.clone());

        task.run(params)
    }
}
