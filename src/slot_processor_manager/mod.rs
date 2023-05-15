use anyhow::{anyhow, Context as AnyhowContext};
use futures::future::join_all;
use tokio::task::{JoinError, JoinHandle};
use tracing::{error, Instrument};

use self::slot_processor::{errors::SlotProcessorError, SlotProcessor};
use crate::{blobscan_client::types::FailedSlotsChunkEntity, context::Context};

mod slot_processor;

pub struct SlotProcessorManager {
    context: Context,
    max_threads_length: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum SlotProcessorManagerError {
    #[error("Slot processor manager failed to process the following slots chunks: {chunks:?}")]
    FailedSlotsProcessing { chunks: Vec<FailedSlotsChunkEntity> },

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl SlotProcessorManager {
    pub fn try_new(context: Context) -> Result<Self, SlotProcessorManagerError> {
        let max_threads_length = std::thread::available_parallelism()
            .with_context(|| "Failed to get maximum thread length")?
            .get() as u32;

        Ok(Self {
            context,
            max_threads_length,
        })
    }

    pub async fn process_slots(
        &self,
        start_slot: u32,
        end_slot: u32,
    ) -> Result<(), SlotProcessorManagerError> {
        if start_slot == end_slot {
            return Ok(());
        }

        let slots_chunk = end_slot - start_slot;
        let slots_per_thread = slots_chunk / self.max_threads_length;
        let threads_length = if slots_per_thread > 0 {
            self.max_threads_length
        } else {
            slots_chunk
        };
        let mut threads: Vec<JoinHandle<Result<u32, SlotProcessorError>>> = vec![];
        let mut current_slot = start_slot;

        for i in 0..threads_length {
            let thread_slots_chunk = if i == 0 {
                slots_per_thread + slots_chunk % self.max_threads_length
            } else {
                slots_per_thread
            };

            let thread_context = self.context.clone();
            let thread_initial_slot = current_slot;
            let thread_final_slot = current_slot + thread_slots_chunk;

            let thread = tokio::spawn(async move {
                let thread_slots_span = tracing::trace_span!(
                    "thread_slots_processor",
                    initial_slot = thread_initial_slot,
                    final_slot = thread_final_slot
                );
                let slot_processor = SlotProcessor::new(thread_context);

                slot_processor
                    .process_slots(thread_initial_slot, thread_final_slot)
                    .instrument(thread_slots_span)
                    .await
            });

            threads.push(thread);

            current_slot += thread_slots_chunk;
        }

        let thread_outputs = join_all(threads).await;

        let failed_slots_chunks = self.get_failed_slots_chunks(&thread_outputs).await?;

        if !failed_slots_chunks.is_empty() {
            return Err(SlotProcessorManagerError::FailedSlotsProcessing {
                chunks: failed_slots_chunks,
            });
        }

        Ok(())
    }

    async fn get_failed_slots_chunks(
        &self,
        thread_outputs: &[Result<Result<u32, SlotProcessorError>, JoinError>],
    ) -> Result<Vec<FailedSlotsChunkEntity>, SlotProcessorManagerError> {
        let failed_threads = thread_outputs
            .iter()
            .filter(|thread_join| match thread_join {
                Ok(thread_result) => thread_result.is_err(),
                Err(_) => true,
            });
        let mut failed_slots_chunks: Vec<FailedSlotsChunkEntity> = vec![];

        for thread in failed_threads.into_iter() {
            match thread {
                Ok(thread_result) => match thread_result.as_ref().unwrap_err() {
                    SlotProcessorError::ProcessingError {
                        slot,
                        target_slot,
                        reason: _,
                    } => {
                        error!("Failed to process slots from {} to {}", slot, target_slot);
                        failed_slots_chunks.push(FailedSlotsChunkEntity::from((
                            slot.to_owned(),
                            target_slot.to_owned(),
                        )))
                    }
                },
                Err(join_error) => {
                    return Err(anyhow!(format!(
                        "Slot processing thread failed unexpectedly: {:?}",
                        join_error
                    ))
                    .into())
                }
            }
        }

        return Ok(failed_slots_chunks);
    }
}
