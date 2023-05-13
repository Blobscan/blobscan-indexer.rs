use core::panic;
use std::sync::Arc;

use futures::future::join_all;
use tokio::task::{JoinError, JoinHandle};
use tracing::Instrument;

use self::slot_processor::{errors::SlotProcessorError, SlotProcessor};
use crate::{blobscan_client::types::FailedSlotsChunkEntity, context::Context};

mod slot_processor;

pub struct SlotProcessorManager {
    shared_context: Arc<Context>,
    max_threads_length: u32,
}

impl SlotProcessorManager {
    pub fn try_new(context: Context) -> Result<Self, anyhow::Error> {
        let max_threads_length = std::thread::available_parallelism()?.get() as u32;
        let shared_context = Arc::new(context);

        Ok(Self {
            shared_context,
            max_threads_length,
        })
    }

    pub async fn process_slots(&self, start_slot: u32, end_slot: u32) -> Result<(), anyhow::Error> {
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

            let thread_context = Arc::clone(&self.shared_context);
            let thread_initial_slot = current_slot;
            let thread_final_slot = current_slot + thread_slots_chunk;

            let thread = tokio::spawn(async move {
                let slot_span = tracing::trace_span!("slot_processor", slot = end_slot);
                let slot_processor = SlotProcessor::new(&thread_context);

                slot_processor
                    .process_slots(thread_initial_slot, thread_final_slot)
                    .instrument(slot_span)
                    .await
            });

            threads.push(thread);

            current_slot += thread_slots_chunk;
        }

        let thread_outputs = join_all(threads).await;

        self.process_thread_outputs(&thread_outputs).await?;

        Ok(())
    }

    async fn process_thread_outputs(
        &self,
        thread_outputs: &[Result<Result<u32, SlotProcessorError>, JoinError>],
    ) -> Result<(), anyhow::Error> {
        let failed_slots_chunks = thread_outputs
            .iter()
            .filter(|thread_join| match thread_join {
                Ok(thread_result) => thread_result.is_err(),
                Err(_) => true,
            })
            .map(|thread_join| match thread_join {
                Ok(thread_result) => match thread_result.as_ref().unwrap_err() {
                    SlotProcessorError::ProcessingError {
                        slot,
                        target_slot,
                        reason: _,
                    } => FailedSlotsChunkEntity::from((slot.to_owned(), target_slot.to_owned())),
                },
                Err(join_error) => panic!("Thread panicked: {:?}", join_error),
            })
            .collect::<Vec<FailedSlotsChunkEntity>>();

        if !failed_slots_chunks.is_empty() {
            self.shared_context
                .blobscan_client
                .add_failed_slots_chunks(failed_slots_chunks)
                .await?;
        }

        Ok(())
    }
}
