use std::time::Duration;

use anyhow::{Context as AnyhowContext, Result};
use backoff::{future::retry_notify, Error as BackoffError};

use ethers::prelude::*;
use tracing::{info, warn, Instrument};

use crate::{
    blobscan_client::types::{BlobEntity, BlockEntity, TransactionEntity},
    context::Context,
    utils::exp_backoff::get_exp_backoff_config,
};

use self::errors::{SingleSlotProcessingError, SlotProcessorError};
use self::helpers::{create_tx_hash_versioned_hashes_mapping, create_versioned_hash_blob_mapping};

pub mod errors;
mod helpers;

pub struct SlotProcessor {
    context: Context,
}

impl SlotProcessor {
    pub fn new(context: Context) -> SlotProcessor {
        Self { context }
    }

    pub async fn process_slots(
        &self,
        start_slot: u32,
        end_slot: u32,
    ) -> Result<u32, SlotProcessorError> {
        for current_slot in start_slot..end_slot {
            let slot_span = tracing::info_span!("slot_processor", slot = current_slot);
            let result = self
                .process_slot_with_retry(current_slot)
                .instrument(slot_span)
                .await;

            if let Err(e) = result {
                return Err(SlotProcessorError::ProcessingError {
                    slot: current_slot,
                    target_slot: end_slot,
                    reason: e,
                });
            };
        }

        Ok(end_slot - 1)
    }

    async fn process_slot_with_retry(&self, slot: u32) -> Result<(), SingleSlotProcessingError> {
        let backoff_config = get_exp_backoff_config();

        retry_notify(
            backoff_config,
            || async move { self.process_slot(slot).await },
            |e, duration: Duration| {
                let duration = duration.as_secs();
                warn!("Slot processing failed. Retrying in {duration} seconds… (Reason: {e})");
            },
        )
        .await
    }

    pub async fn process_slot(
        &self,
        slot: u32,
    ) -> Result<(), backoff::Error<SingleSlotProcessingError>> {
        let beacon_client = self.context.beacon_client();
        let blobscan_client = self.context.blobscan_client();
        let provider = self.context.provider();

        // Fetch execution block data from a given slot and perform some checks

        let beacon_block = match beacon_client
            .get_block(Some(slot))
            .await
            .map_err(SingleSlotProcessingError::BeaconClient)?
        {
            Some(block) => block,
            None => {
                info!("Skipping as there is no beacon block");

                return Ok(());
            }
        };

        let execution_payload = match beacon_block.body.execution_payload {
            Some(payload) => payload,
            None => {
                info!("Skipping as beacon block doesn't contain execution payload");

                return Ok(());
            }
        };

        match beacon_block.body.blob_kzg_commitments {
            Some(commitments) => commitments,
            None => {
                info!("Skipping as beacon block doesn't contain blob kzg commitments");

                return Ok(());
            }
        };

        let execution_block_hash = execution_payload.block_hash;

        // Fetch execution block and perform some checks

        let execution_block = provider
            .get_block_with_txs(execution_block_hash)
            .await
            .map_err(|err| BackoffError::permanent(SingleSlotProcessingError::Provider(err)))?
            .with_context(|| format!("Execution block {execution_block_hash} not found"))
            .map_err(|err| BackoffError::permanent(SingleSlotProcessingError::Other(err)))?;

        let tx_hash_to_versioned_hashes = create_tx_hash_versioned_hashes_mapping(&execution_block)
            .map_err(|err| BackoffError::permanent(SingleSlotProcessingError::Other(err)))?;

        if tx_hash_to_versioned_hashes.is_empty() {
            info!("Skipping as execution block doesn't contain blob txs");

            return Ok(());
        }

        // Fetch blobs and perform some checks

        let blobs = match beacon_client
            .get_blobs(slot)
            .await
            .map_err(SingleSlotProcessingError::BeaconClient)?
        {
            Some(blobs) => {
                if blobs.is_empty() {
                    info!("Skipping as blobs sidecar is empty");

                    return Ok(());
                } else {
                    blobs
                }
            }
            None => {
                info!("Skipping as there is no blobs sidecar");

                return Ok(());
            }
        };

        // Create entities to be indexed

        let block_entity = BlockEntity::try_from((&execution_block, slot))
            .map_err(|err| BackoffError::Permanent(SingleSlotProcessingError::Other(err)))?;

        let transactions_entities = execution_block
            .transactions
            .iter()
            .filter(|tx| tx_hash_to_versioned_hashes.contains_key(&tx.hash))
            .map(|tx| TransactionEntity::try_from((tx, &execution_block)))
            .collect::<Result<Vec<TransactionEntity>>>()
            .map_err(|err| BackoffError::Permanent(SingleSlotProcessingError::Other(err)))?;

        let versioned_hash_to_blob = create_versioned_hash_blob_mapping(&blobs)
            .map_err(|err| BackoffError::Permanent(SingleSlotProcessingError::Other(err)))?;
        let mut blob_entities: Vec<BlobEntity> = vec![];

        for (tx_hash, versioned_hashes) in tx_hash_to_versioned_hashes.iter() {
            for (i, versioned_hash) in versioned_hashes.iter().enumerate() {
                let blob = *versioned_hash_to_blob.get(versioned_hash).with_context(|| format!("Sidecar not found for blob {i} with versioned hash {versioned_hash} from tx {tx_hash}")).map_err(|err| BackoffError::Permanent(SingleSlotProcessingError::Other(err)))?;

                blob_entities.push(BlobEntity::from((blob, versioned_hash, i, tx_hash)));
            }
        }

        blobscan_client
            .index(block_entity, transactions_entities, blob_entities)
            .await
            .map_err(SingleSlotProcessingError::BlobscanClient)?;

        info!("Block, txs and blobs indexed successfully");

        Ok(())
    }
}
