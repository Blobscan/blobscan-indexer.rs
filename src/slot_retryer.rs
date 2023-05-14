use tracing::info;

use crate::{
    blobscan_client::types::FailedSlotsChunkEntity,
    context::Context,
    slot_processor_manager::{SlotProcessorManager, SlotProcessorManagerError},
};

pub struct SlotRetryer {
    context: Context,
}

impl SlotRetryer {
    pub fn new(context: Context) -> Self {
        Self { context }
    }

    pub async fn run(&self) -> Result<(), anyhow::Error> {
        let thread_context = self.context.clone();

        let blobscan_client = thread_context.blobscan_client();
        let slots_processor_manager = SlotProcessorManager::try_new(thread_context.clone())?;

        let failed_slots_chunks = blobscan_client.get_failed_slots_chunks().await?;
        let mut chunk_ids: Vec<u32> = vec![];
        let mut failed_sub_chunks: Vec<FailedSlotsChunkEntity> = vec![];

        if !failed_slots_chunks.is_empty() {
            for failed_chunk in failed_slots_chunks.iter() {
                match slots_processor_manager
                    .process_slots(failed_chunk.initial_slot, failed_chunk.final_slot)
                    .await
                {
                    Ok(_) => {
                        info!(
                            "Failed slots from {} to {} were successfully re-processed",
                            failed_chunk.initial_slot, failed_chunk.final_slot
                        );
                    }
                    Err(err) => match err {
                        SlotProcessorManagerError::FailedSlotsProcessing { chunks } => chunks
                            .into_iter()
                            .for_each(|chunk| failed_sub_chunks.push(chunk)),
                        SlotProcessorManagerError::Other(err) => {
                            anyhow::bail!(err);
                        }
                    },
                };

                /*
                    When attempting to re-index failed chunks, there may be instances where certain sub-chunks
                    fail again. Therefore, to prevent storing overlapped chunks, it's beneficial to remove all of
                    them, irrespective of whether they failed during the retry process as their failed sub-chunks
                    will be stored either way.
                */
                chunk_ids.push(failed_chunk.id.unwrap())
            }
        }

        if !failed_sub_chunks.is_empty() {
            blobscan_client
                .add_failed_slots_chunks(failed_sub_chunks)
                .await?;
        } else {
            info!("No failed slots chunks to retry");
        }

        if !chunk_ids.is_empty() {
            blobscan_client
                .remove_failed_slots_chunks(chunk_ids)
                .await?;
        }

        Ok(())
    }
}
