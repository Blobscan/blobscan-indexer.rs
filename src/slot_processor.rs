use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context as AnyhowContext, Result};
use backoff::{
    future::retry_notify, Error as BackoffError, ExponentialBackoff, ExponentialBackoffBuilder,
};
use ethers::prelude::*;

use tracing::{error, info, warn};

use crate::{
    db::{blob_db_manager::DBManager, mongodb::MongoDBManagerOptions},
    types::{Blob, BlockData, TransactionData},
    utils::{context::Context, web3::calculate_versioned_hash},
};

pub struct SlotProcessorOptions {
    pub backoff_config: ExponentialBackoff,
}

pub struct SlotProcessor<'a> {
    options: SlotProcessorOptions,
    context: &'a Context,
    db_options: MongoDBManagerOptions,
}

impl<'a> SlotProcessor<'a> {
    pub async fn try_init(
        context: &'a Context,
        options: Option<SlotProcessorOptions>,
    ) -> Result<SlotProcessor> {
        let options = options.unwrap_or(SlotProcessorOptions {
            backoff_config: ExponentialBackoffBuilder::default()
                .with_initial_interval(Duration::from_secs(2))
                .with_max_elapsed_time(Some(Duration::from_secs(60)))
                .build(),
        });

        Ok(Self {
            options,
            context,
            db_options: MongoDBManagerOptions {
                session: context.db_manager.client.start_session(None).await?,
            },
        })
    }

    pub async fn process_slots(&mut self, start_slot: u32, end_slot: u32) -> Result<()> {
        let mut current_slot = start_slot;

        while current_slot < end_slot {
            let result = self.process_slot_with_retry(current_slot).await;

            if let Err(e) = result {
                self.save_slot(current_slot - 1).await?;

                error!("[Slot {current_slot}] Couldn't process slot: {e}");

                return Err(e);
            };

            current_slot += 1;
        }

        self.save_slot(current_slot).await?;

        Ok(())
    }

    async fn process_slot_with_retry(&mut self, slot: u32) -> Result<()> {
        let backoff_config = self.options.backoff_config.clone();

        /*
          This is necessary because the `retry` function requires
          the closure to be `FnMut` and the `SlotProcessor` instance is not `Clone`able. The `Arc<Mutex<>>` allows us to
          share the `SlotProcessor` instance across multiple tasks and safely mutate it within the context of the retry loop.
        */
        let shared_slot_processor = Arc::new(Mutex::new(self));

        retry_notify(
            backoff_config,
            || {
                let slot_processor = Arc::clone(&shared_slot_processor);

                /*
                 Using unwrap() here. If Mutex is poisoned due to a panic, it returns an error. 
                 In this case, we allow the indexer to crash as the state might be invalid. 
                */
                async move {
                    let mut slot_processor = slot_processor.lock().unwrap();

                    match slot_processor.process_slot(slot).await {
                        Ok(_) => Ok(()),
                        Err(process_slot_err) => {
                            match slot_processor.db_options.session.abort_transaction().await {
                                Ok(_) => Err(process_slot_err),
                                Err(err) => Err(BackoffError::Permanent(err.into())),
                            }
                        }
                    }
                }
            },
            |e, duration: Duration| {
                let duration = duration.as_secs();
                warn!("[Slot {slot}] Slot processing failed. Retrying in {duration} seconds… (Reason: {e})");
            },
        )
        .await
    }

    pub async fn process_slot(&mut self, slot: u32) -> Result<(), backoff::Error<anyhow::Error>> {
        let Context {
            beacon_api,
            db_manager,
            provider,
        } = self.context;
        let db_options = &mut self.db_options;

        let start = Instant::now();

        let beacon_block = match beacon_api
            .get_block(Some(slot))
            .await
            .map_err(|err| BackoffError::transient(anyhow::Error::new(err)))?
        {
            Some(block) => block,
            None => {
                info!("[Slot {slot}] Skipping as there is no beacon block");

                return Ok(());
            }
        };

        let execution_payload = match beacon_block.body.execution_payload {
            Some(payload) => payload,
            None => {
                info!("[Slot {slot}] Skipping as beacon block doesn't contain execution payload");

                return Ok(());
            }
        };

        let blob_kzg_commitments = match beacon_block.body.blob_kzg_commitments {
            Some(commitments) => commitments,
            None => {
                info!(
                    "[Slot {slot}] Skipping as beacon block doesn't contain blob kzg commitments"
                );

                return Ok(());
            }
        };
        let execution_block_hash = execution_payload.block_hash;

        let execution_block = provider
            .get_block_with_txs(execution_block_hash)
            .await
            .with_context(|| format!("Failed to fetch execution block {execution_block_hash}"))?
            .with_context(|| format!("Execution block {execution_block_hash} not found"))
            .map_err(BackoffError::Permanent)?;

        let block_data =
            BlockData::try_from((&execution_block, slot)).map_err(BackoffError::Permanent)?;

        if block_data.tx_to_versioned_hashes.is_empty() {
            info!("[Slot {slot}] Skipping as execution block doesn't contain blob txs");

            return Ok(());
        }

        let blobs = match beacon_api
            .get_blobs_sidecar(slot)
            .await
            .map_err(|err| BackoffError::transient(anyhow::Error::new(err)))?
        {
            Some(blobs_sidecar) => {
                if blobs_sidecar.blobs.is_empty() {
                    info!("[Slot {slot}] Skipping as blobs sidecar is empty");

                    return Ok(());
                } else {
                    blobs_sidecar.blobs
                }
            }
            None => {
                info!("[Slot {slot}] Skipping as there is no blobs sidecar");

                return Ok(());
            }
        };

        db_manager.start_transaction(Some(db_options)).await?;

        db_manager
            .insert_block(&block_data, Some(db_options))
            .await?;

        for tx in block_data.block.transactions.iter() {
            let blob_versioned_hashes = block_data
                .tx_to_versioned_hashes
                .get(&tx.hash)
                .with_context(|| format!("Couldn't find versioned hashes for tx {}", tx.hash))
                .map_err(BackoffError::Permanent)?;

            db_manager
                .insert_tx(
                    &TransactionData {
                        tx,
                        blob_versioned_hashes,
                    },
                    Some(db_options),
                )
                .await?;
        }

        for (i, blob) in blobs.iter().enumerate() {
            let commitment = blob_kzg_commitments[i].clone();
            let versioned_hash = calculate_versioned_hash(&commitment)?;
            let tx_hash = block_data.tx_to_versioned_hashes.iter().find_map(
                |(tx_hash, versioned_hashes)| match versioned_hashes.contains(&versioned_hash) {
                    true => Some(tx_hash),
                    false => None,
                },
            ).with_context(|| format!("No blob transaction found for commitment {commitment} and versioned hash {versioned_hash}"))?;

            db_manager
                .insert_blob(
                    &Blob {
                        commitment,
                        data: blob,
                        versioned_hash,
                        tx_hash: *tx_hash,
                    },
                    Some(db_options),
                )
                .await?;
        }

        db_manager.commit_transaction(Some(db_options)).await?;

        let duration = start.elapsed();

        info!(
            "[Slot {slot}] Blobs indexed correctly (elapsed time: {:?}s)",
            duration.as_secs()
        );

        Ok(())
    }

    async fn save_slot(&mut self, slot: u32) -> Result<()> {
        self.context
            .db_manager
            .update_last_slot(slot, Some(&mut self.db_options))
            .await?;

        Ok(())
    }
}
