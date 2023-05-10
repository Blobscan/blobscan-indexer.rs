use std::{sync::Arc, time::Duration};

use backoff::{future::retry_notify, Error as BackoffError};
use futures::future::join_all;
use tokio::task::JoinHandle;
use tracing::{warn, Instrument};

use self::slot_processor::{errors::SlotProcessorError, SlotProcessor};
use crate::{
    blobscan_client::types::BlobscanClientError, context::Context,
    utils::exp_backoff::get_exp_backoff_config,
};

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
        let mut threads: Vec<JoinHandle<Result<(), BlobscanClientError>>> = vec![];
        let mut current_slot = start_slot;

        for i in 0..threads_length {
            let thread_slots_chunk = if i == 0 {
                slots_per_thread + slots_chunk % self.max_threads_length
            } else {
                slots_per_thread
            };

            let backoff_config = get_exp_backoff_config();
            let thread_context = Arc::clone(&self.shared_context);
            let thread_initial_slot = current_slot;
            let thread_final_slot = current_slot + thread_slots_chunk;

            let thread = tokio::spawn(async move {
                let slot_span = tracing::trace_span!("slot_processor", slot = end_slot);
                let slot_processor = SlotProcessor::new(&thread_context);

                let processor_result = &slot_processor
                    .process_slots(thread_initial_slot, thread_final_slot)
                    .instrument(slot_span)
                    .await;

                let blobscan_client = &thread_context.blobscan_client;

                retry_notify(
                    backoff_config,
                    || async move {
                        let last_slot = match processor_result {
                            Ok(_) => current_slot,
                            Err(err) => match err {
                                SlotProcessorError::ProcessingError { slot, reason: _ } => {
                                    // TODO - Store error
                                    slot.to_owned()
                                },
                            },
                        };

                        blobscan_client.update_slot(last_slot).await.map_err( BackoffError::transient)
                    },
                    |e, duration: Duration| {
                        let duration = duration.as_secs();
                        warn!("Couldn't update latest slot. Retrying in {duration} seconds… (Reason: {e})");
                    },
                ).await
            });

            threads.push(thread);

            current_slot += thread_slots_chunk;
        }

        // TODO: Handle joins
        join_all(threads).await;

        Ok(())
    }
}
