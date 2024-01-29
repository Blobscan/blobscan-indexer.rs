use anyhow::Result;
use futures::{future::join_all, StreamExt};
use reqwest_eventsource::Event;
use tokio::task::JoinHandle;
use tracing::{debug, error, Instrument};

use crate::{
    clients::beacon::types::{ChainReorgResponse, Topic},
    context::Context,
    slots_processor::slot_processor::SlotProcessor,
};

pub struct ChainReorgHandler {
    context: Context,
}

impl ChainReorgHandler {
    pub fn new(context: Context) -> ChainReorgHandler {
        Self { context }
    }

    pub fn run(&self) -> Result<JoinHandle<Result<(), anyhow::Error>>> {
        let thread_context = self.context.clone();

        let handle = tokio::spawn(async move {
            let blobscan_client = thread_context.blobscan_client();
            let beacon_client = thread_context.beacon_client();
            let mut event_source = match beacon_client.subscribe_to_events(vec![Topic::ChainReorg])
            {
                Ok(es) => es,
                Err(error) => {
                    error!(
                        target = "chain_reorg_handler",
                        ?error,
                        "Failed to subscribe to chain reorg events"
                    );

                    return Err(error.into());
                }
            };

            while let Some(event) = event_source.next().await {
                match event {
                    Ok(Event::Open) => debug!(
                        target = "chain_reorg_handler",
                        "Listening to chain reorg events…"
                    ),
                    Ok(Event::Message(message)) => {
                        let chain_reorg_data =
                            serde_json::from_str::<ChainReorgResponse>(&message.data)?;

                        let new_head_slot = chain_reorg_data.slot.parse::<u32>()?;
                        let reorg_depth = chain_reorg_data.depth.parse::<u32>()?;

                        blobscan_client
                            .handle_reorg(new_head_slot, reorg_depth)
                            .await?;

                        // reprocess reorganized slots
                        let slot_processor = SlotProcessor::new(thread_context.clone());

                        let mut futures = Vec::new();

                        for slot in new_head_slot..new_head_slot + reorg_depth {
                            let slot_span = tracing::trace_span!("slot_processor");

                            futures.push(slot_processor.process_slot(slot).instrument(slot_span));
                        }

                        let results = join_all(futures).await;

                        for result in results {
                            if let Err(error) = result {
                                error!(
                                    target = "chain_reorg_handler",
                                    ?error,
                                    "Failed to process reorg slot"
                                );

                                return Err(error.into());
                            }
                        }
                    }
                    Err(e) => {
                        error!(
                            target = "chain_reorg_handler",
                            ?e,
                            "Failed to receive chain reorg event"
                        );

                        event_source.close();

                        return Err(e.into());
                    }
                }
            }

            Ok(())
        });

        Ok(handle)
    }
}
