use anyhow::Result;
use futures::StreamExt;
use reqwest_eventsource::Event;
use tokio::task::JoinHandle;
use tracing::{debug, error};

use crate::{
    clients::beacon::types::{ChainReorgResponse, Topic},
    context::Context,
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
