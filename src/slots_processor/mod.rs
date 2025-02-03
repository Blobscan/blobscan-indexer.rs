use alloy::{
    primitives::B256, rpc::types::BlockTransactionsKind, transports::http::ReqwestTransport,
};
use anyhow::{anyhow, Context as AnyhowContext, Result};

use tracing::{debug, info};

use crate::{
    clients::{
        beacon::types::BlockHeader,
        blobscan::types::{Blob, BlobscanBlock, Block, Transaction},
        common::ClientError,
    },
    context::CommonContext,
};

use self::error::{SlotProcessingError, SlotsProcessorError};
use self::helpers::{create_tx_hash_versioned_hashes_mapping, create_versioned_hash_blob_mapping};

pub mod error;
mod helpers;

pub struct SlotsProcessor<T> {
    context: Box<dyn CommonContext<T>>,
    pub last_processed_block: Option<BlockHeader>,
}

impl SlotsProcessor<ReqwestTransport> {
    pub fn new(
        context: Box<dyn CommonContext<ReqwestTransport>>,
        last_processed_block: Option<BlockHeader>,
    ) -> SlotsProcessor<ReqwestTransport> {
        Self {
            context,
            last_processed_block,
        }
    }

    pub async fn process_slots(
        &mut self,
        initial_slot: u32,
        final_slot: u32,
    ) -> Result<(), SlotsProcessorError> {
        let is_reverse = initial_slot > final_slot;
        let slots = if is_reverse {
            (final_slot..initial_slot).rev().collect::<Vec<_>>()
        } else {
            (initial_slot..final_slot).collect::<Vec<_>>()
        };

        let mut last_processed_block = self.last_processed_block.clone();

        for current_slot in slots {
            let block_header = match self.process_slot(current_slot).await {
                Ok(block_header) => block_header,
                Err(error) => {
                    return Err(SlotsProcessorError::FailedSlotsProcessing {
                        initial_slot,
                        final_slot,
                        failed_slot: current_slot,
                        error,
                    });
                }
            };

            if let Some(block_header) = block_header {
                if let Some(prev_block_header) = last_processed_block {
                    if prev_block_header.root != block_header.parent_root {
                        self.process_reorg(&prev_block_header, &block_header)
                            .await
                            .map_err(|err| SlotsProcessorError::ReorgedFailure(err))?;
                    }
                }

                last_processed_block = Some(block_header);
            }
        }

        self.last_processed_block = last_processed_block;

        Ok(())
    }

    pub async fn process_slot(
        &mut self,
        slot: u32,
    ) -> Result<Option<BlockHeader>, SlotProcessingError> {
        let beacon_client = self.context.beacon_client();
        let blobscan_client = self.context.blobscan_client();
        let provider = self.context.provider();

        let beacon_block_header = Some(match beacon_client.get_block_header(slot.into()).await? {
            Some(header) => header,
            None => {
                debug!(slot, "Skipping as there is no beacon block header");

                return Ok(None);
            }
        });

        let beacon_block = match beacon_client.get_block(slot.into()).await? {
            Some(block) => block,
            None => {
                debug!(slot = slot, "Skipping as there is no beacon block");

                return Ok(None);
            }
        };

        let execution_payload = match beacon_block.message.body.execution_payload {
            Some(payload) => payload,
            None => {
                debug!(
                    slot,
                    "Skipping as beacon block doesn't contain execution payload"
                );

                return Ok(beacon_block_header);
            }
        };

        let has_kzg_blob_commitments = match beacon_block.message.body.blob_kzg_commitments {
            Some(commitments) => !commitments.is_empty(),
            None => false,
        };

        if !has_kzg_blob_commitments {
            debug!(
                slot,
                "Skipping as beacon block doesn't contain blob kzg commitments"
            );

            return Ok(beacon_block_header);
        }

        let execution_block_hash = execution_payload.block_hash;

        // Fetch execution block and perform some checks

        let execution_block = provider
            .get_block(execution_block_hash.into(), BlockTransactionsKind::Full)
            .await?
            .with_context(|| format!("Execution block {execution_block_hash} not found"))?;

        let tx_hash_to_versioned_hashes =
            create_tx_hash_versioned_hashes_mapping(&execution_block)?;

        if tx_hash_to_versioned_hashes.is_empty() {
            return Err(anyhow!("Blocks mismatch: Beacon block contains blob KZG commitments, but the corresponding execution block does not contain any blob transactions").into());
        }

        // Fetch blobs and perform some checks

        let blobs = match beacon_client
            .get_blobs(slot.into())
            .await
            .map_err(SlotProcessingError::ClientError)?
        {
            Some(blobs) => {
                if blobs.is_empty() {
                    debug!(slot, "Skipping as blobs sidecar is empty");

                    return Ok(beacon_block_header);
                } else {
                    blobs
                }
            }
            None => {
                debug!(slot, "Skipping as there is no blobs sidecar");

                return Ok(beacon_block_header);
            }
        };

        // Create entities to be indexed

        let block_entity = Block::try_from((&execution_block, slot))?;
        let block_transactions = execution_block
            .transactions
            .as_transactions()
            .ok_or_else(|| anyhow!("Failed to parse transactions"))?;

        let transactions_entities = block_transactions
            .iter()
            .filter(|tx| tx_hash_to_versioned_hashes.contains_key(&tx.hash))
            .map(|tx| Transaction::try_from((tx, &execution_block)))
            .collect::<Result<Vec<Transaction>>>()?;

        let versioned_hash_to_blob = create_versioned_hash_blob_mapping(&blobs)?;
        let mut blob_entities: Vec<Blob> = vec![];

        for (tx_hash, versioned_hashes) in tx_hash_to_versioned_hashes.iter() {
            for (i, versioned_hash) in versioned_hashes.iter().enumerate() {
                let blob = *versioned_hash_to_blob.get(versioned_hash).with_context(|| format!("Sidecar not found for blob {i} with versioned hash {versioned_hash} from tx {tx_hash}"))?;

                blob_entities.push(Blob::from((blob, versioned_hash, i, tx_hash)));
            }
        }

        /*
        let tx_hashes = transactions_entities
            .iter()
            .map(|tx| tx.hash.to_string())
            .collect::<Vec<String>>();
        let blob_versioned_hashes = blob_entities
            .iter()
            .map(|blob| blob.versioned_hash.to_string())
            .collect::<Vec<String>>();
         */

        let block_number = block_entity.number;

        blobscan_client
            .index(block_entity, transactions_entities, blob_entities)
            .await
            .map_err(SlotProcessingError::ClientError)?;

        info!(slot, block_number, "Block indexed successfully");

        Ok(beacon_block_header)
    }

    async fn process_reorg(
        &mut self,
        old_head_header: &BlockHeader,
        new_head_header: &BlockHeader,
    ) -> Result<(), ClientError> {
        let mut current_old_slot = old_head_header.slot;

        let mut rewinded_execution_blocks: Vec<B256> = vec![];

        loop {
            let old_blobscan_block = match self
                .context
                .blobscan_client()
                .get_block(current_old_slot)
                .await?
            {
                Some(block) => block,
                None => {
                    current_old_slot -= 1;

                    if current_old_slot == 0 {
                        return Err(anyhow!(
                            "No blobscan block found for old head slot {}",
                            old_head_header.slot
                        )
                        .into());
                    }

                    continue;
                }
            };

            let forwarded_execution_blocks = self
                .get_canonical_execution_blocks(new_head_header.root, &old_blobscan_block)
                .await?;

            rewinded_execution_blocks.push(old_blobscan_block.hash);

            if !forwarded_execution_blocks.is_empty() {
                let rewinded_blocks_count = rewinded_execution_blocks.len();
                let forwarded_blocks_count = forwarded_execution_blocks.len();

                info!(
                    new_slot = new_head_header.slot,
                    old_slot = old_head_header.slot,
                    "Reorg detected! rewinded blocks: {rewinded_blocks_count}, forwarded blocks: {forwarded_blocks_count}",
                );
                self.context
                    .blobscan_client()
                    .handle_reorg(rewinded_execution_blocks, forwarded_execution_blocks)
                    .await?;

                return Ok(());
            }
        }
    }

    async fn get_canonical_execution_blocks(
        &mut self,
        canonical_block_root: B256,
        blobscan_block: &BlobscanBlock,
    ) -> Result<Vec<B256>, ClientError> {
        let beacon_client = self.context.beacon_client();
        let mut canonical_execution_blocks: Vec<B256> = vec![];

        let mut canonical_block = match beacon_client.get_block(canonical_block_root.into()).await?
        {
            Some(block) => block,
            None => {
                return Ok(canonical_execution_blocks);
            }
        };

        if let Some(execution_payload) = &canonical_block.message.body.execution_payload {
            if execution_payload.block_hash == blobscan_block.hash {
                return Ok(vec![]);
            }
        }

        while canonical_block.message.parent_root != B256::ZERO {
            if canonical_block.message.slot < blobscan_block.slot {
                return Ok(vec![]);
            }

            if let Some(execution_payload) = canonical_block.message.body.execution_payload {
                if execution_payload.block_hash == blobscan_block.hash {
                    return Ok(canonical_execution_blocks);
                }

                canonical_execution_blocks.push(execution_payload.block_hash);
            }

            canonical_block = match beacon_client
                .get_block(canonical_block.message.parent_root.into())
                .await?
            {
                Some(block) => block,
                None => {
                    return Ok(canonical_execution_blocks);
                }
            };
        }

        Ok(vec![])
    }
}
