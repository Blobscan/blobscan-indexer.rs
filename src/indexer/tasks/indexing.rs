use tracing::{debug, Instrument, Span};

use crate::{
    clients::beacon::types::{BlockHeader, BlockId},
    context::CommonContext,
    indexer::types::{
        ErrorResport, IndexingTaskJoinHandle, TaskErrorChannelSender, TaskResultChannelSender,
    },
    synchronizer::{CheckpointType, CommonSynchronizer, SynchronizerBuilder},
};

pub struct IndexingTask {
    context: Box<dyn CommonContext>,
    name: String,
    span: Option<Span>,
}

pub struct RunParams {
    pub error_report_tx: TaskErrorChannelSender,
    pub result_report_tx: Option<TaskResultChannelSender>,
    pub from_block_id: BlockId,
    pub to_block_id: BlockId,
    pub prev_block: Option<BlockHeader>,
    pub checkpoint: Option<CheckpointType>,
}

impl IndexingTask {
    pub fn new(name: &str, context: Box<dyn CommonContext>, span: Option<Span>) -> Self {
        IndexingTask {
            context,
            name: name.into(),
            span,
        }
    }

    pub fn run(&self, params: RunParams) -> IndexingTaskJoinHandle {
        let context = self.context.clone();
        let name = self.name.clone();
        let span = self.span.clone();

        tokio::spawn(async move {
            let RunParams {
                error_report_tx,
                result_report_tx,
                from_block_id,
                prev_block,
                checkpoint,
                to_block_id,
            } = params;
            let mut synchronizer_builder = SynchronizerBuilder::new();

            if let Some(prev_block) = prev_block {
                synchronizer_builder.with_last_synced_block(prev_block);
            }

            synchronizer_builder.with_checkpoint(checkpoint);

            let mut synchronizer = synchronizer_builder.build(context);

            let indexing_task_span = span.unwrap_or(tracing::info_span!("indexing-task"));

            async {
                let result = if from_block_id == to_block_id {
                    synchronizer.sync_block(from_block_id).await
                } else {
                    synchronizer.sync_blocks(from_block_id, to_block_id).await
                };

                match result {
                    Ok(sync_result) => {
                        debug!("Task {name} completed!");

                        if let Some(report_tx) = result_report_tx {
                            report_tx.send(sync_result).unwrap();
                        }
                    }
                    Err(sync_error) => {
                        error_report_tx
                            .send(ErrorResport {
                                task_name: name,
                                error: sync_error.into(),
                            })
                            .await
                            .unwrap();
                    }
                }
            }
            .instrument(indexing_task_span)
            .await;
        })
    }
}
