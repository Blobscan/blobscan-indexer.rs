use std::time::Duration;

use anyhow::{Context as AnyhowContext, Result};
use backoff::{future::retry_notify, Error as BackoffError};

use ethers::prelude::*;
use tracing::{info, warn};

use crate::{
    blobscan_client::types::{Blob, Block, Transaction},
    context::Context,
    utils::exp_backoff::get_exp_backoff_config,
};

use self::error::SlotProcessorError;
use self::helpers::{create_tx_hash_versioned_hashes_mapping, create_versioned_hash_blob_mapping};

pub mod error;
mod helpers;

pub struct SlotProcessor {
    context: Context,
}

impl SlotProcessor {
    pub fn new(context: Context) -> SlotProcessor {
        Self { context }
    }

    pub async fn process_slot(&self, slot: u32) -> Result<(), SlotProcessorError> {
        let backoff_config = get_exp_backoff_config();

        retry_notify(
            backoff_config,
            || async move { self._process_slot(slot).await },
            |e, duration: Duration| {
                let duration = duration.as_secs();
                warn!(
                    slot,
                    "Failed to process slot. Retrying in {duration} seconds… (Reason: {e})"
                );
            },
        )
        .await
    }

    async fn _process_slot(&self, slot: u32) -> Result<(), backoff::Error<SlotProcessorError>> {
        let beacon_client = self.context.beacon_client();
        let blobscan_client = self.context.blobscan_client();
        let provider = self.context.provider();

        // Fetch execution block data from a given slot and perform some checks

        let beacon_block = match beacon_client
            .get_block(Some(slot))
            .await
            .map_err(SlotProcessorError::BeaconClient)?
        {
            Some(block) => block,
            None => {
                info!(slot, "Skipping as there is no beacon block");

                return Ok(());
            }
        };

        let execution_payload = match beacon_block.body.execution_payload {
            Some(payload) => payload,
            None => {
                info!(
                    slot,
                    "Skipping as beacon block doesn't contain execution payload"
                );

                return Ok(());
            }
        };

        match beacon_block.body.blob_kzg_commitments {
            Some(commitments) => commitments,
            None => {
                info!(
                    slot,
                    "Skipping as beacon block doesn't contain blob kzg commitments"
                );

                return Ok(());
            }
        };

        let execution_block_hash = execution_payload.block_hash;

        // Fetch execution block and perform some checks

        let execution_block = provider
            .get_block_with_txs(execution_block_hash)
            .await
            .map_err(|err| BackoffError::permanent(SlotProcessorError::Provider(err)))?
            .with_context(|| format!("Execution block {execution_block_hash} not found"))
            .map_err(|err| BackoffError::permanent(SlotProcessorError::Other(err)))?;

        let tx_hash_to_versioned_hashes = create_tx_hash_versioned_hashes_mapping(&execution_block)
            .map_err(|err| BackoffError::permanent(SlotProcessorError::Other(err)))?;

        if tx_hash_to_versioned_hashes.is_empty() {
            info!(slot, "Skipping as execution block doesn't contain blob txs");

            return Ok(());
        }

        // Fetch blobs and perform some checks

        let blobs = match beacon_client
            .get_blobs(slot)
            .await
            .map_err(SlotProcessorError::BeaconClient)?
        {
            Some(blobs) => {
                if blobs.is_empty() {
                    info!(slot, "Skipping as blobs sidecar is empty");

                    return Ok(());
                } else {
                    blobs
                }
            }
            None => {
                info!(slot, "Skipping as there is no blobs sidecar");

                return Ok(());
            }
        };

        // Create entities to be indexed

        let block_entity = Block::try_from((&execution_block, slot))
            .map_err(|err| BackoffError::Permanent(SlotProcessorError::Other(err)))?;

        let transactions_entities = execution_block
            .transactions
            .iter()
            .filter(|tx| tx_hash_to_versioned_hashes.contains_key(&tx.hash))
            .map(|tx| Transaction::try_from((tx, &execution_block)))
            .collect::<Result<Vec<Transaction>>>()
            .map_err(|err| BackoffError::Permanent(SlotProcessorError::Other(err)))?;

        let versioned_hash_to_blob = create_versioned_hash_blob_mapping(&blobs)
            .map_err(|err| BackoffError::Permanent(SlotProcessorError::Other(err)))?;
        let mut blob_entities: Vec<Blob> = vec![];

        for (tx_hash, versioned_hashes) in tx_hash_to_versioned_hashes.iter() {
            for (i, versioned_hash) in versioned_hashes.iter().enumerate() {
                let blob = *versioned_hash_to_blob.get(versioned_hash).with_context(|| format!("Sidecar not found for blob {i} with versioned hash {versioned_hash} from tx {tx_hash}")).map_err(|err| BackoffError::Permanent(SlotProcessorError::Other(err)))?;

                blob_entities.push(Blob::from((blob, versioned_hash, i, tx_hash)));
            }
        }

        blobscan_client
            .index(block_entity, transactions_entities, blob_entities)
            .await
            .map_err(SlotProcessorError::BlobscanClient)?;

        info!(slot, "Block, txs and blobs indexed successfully");

        Ok(())
    }
}
