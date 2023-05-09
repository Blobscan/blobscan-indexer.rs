use std::{sync::Arc, time::Duration};

use backoff::{
    future::retry_notify, Error as BackoffError, ExponentialBackoff, ExponentialBackoffBuilder,
};
use futures::future::join_all;
use tokio::task::JoinHandle;
use tracing::{warn, Instrument};

use self::slot_processor::{
    errors::SlotProcessorError, Config as SlotProcessorConfig, SlotProcessor,
};
use crate::{blobscan_client::types::BlobscanClientError, context::Context};

mod slot_processor;

pub struct SlotProcessorManager {
    shared_context: Arc<Context>,
    config: Config,
    max_threads_length: u32,
}

pub struct Config {
    pub backoff_config: ExponentialBackoff,
}

impl SlotProcessorManager {
    pub fn try_new(context: Context, config: Option<Config>) -> Result<Self, anyhow::Error> {
        let max_threads_length = std::thread::available_parallelism()?.get() as u32;
        let shared_context = Arc::new(context);

        let config = config.unwrap_or_else(|| Config {
            backoff_config: ExponentialBackoffBuilder::default()
                .with_initial_interval(Duration::from_secs(2))
                .with_max_elapsed_time(Some(Duration::from_secs(60)))
                .build(),
        });

        Ok(Self {
            shared_context,
            config,
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
            let slots_chunk = if i == 0 {
                slots_per_thread + slots_chunk % self.max_threads_length
            } else {
                slots_per_thread
            };

            let thread_context = Arc::clone(&self.shared_context);
            let backoff_config = self.config.backoff_config.clone();

            let thread = tokio::spawn(async move {
                let slot_span = tracing::trace_span!("slot_processor", slot = end_slot);
                let slot_processor = SlotProcessor::new(
                    &thread_context,
                    Some(SlotProcessorConfig {
                        backoff_config: backoff_config.clone(),
                    }),
                );

                let processor_result = &slot_processor
                    .process_slots(current_slot, current_slot + slots_chunk)
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

                        blobscan_client.update_slot(last_slot).await.map_err(|err| BackoffError::transient(err))
                    },
                    |e, duration: Duration| {
                        let duration = duration.as_secs();
                        warn!("Couldn't update latest slot. Retrying in {duration} seconds… (Reason: {e})");
                    },
                ).await
            });

            threads.push(thread);

            current_slot = current_slot + slots_chunk;
        }

        // TODO: Handle joins
        join_all(threads).await;

        Ok(())
    }
}
