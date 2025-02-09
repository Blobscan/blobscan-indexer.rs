use alloy::{
    primitives::B256, rpc::types::BlockTransactionsKind, transports::http::ReqwestTransport,
};
use anyhow::{anyhow, Context as AnyhowContext, Result};

use crate::clients::beacon::types::BlockHeader;
use tracing::{debug, info};

use crate::{
    clients::{
        blobscan::types::{Blob, BlobscanBlock, Block, Transaction},
        common::ClientError,
    },
    context::CommonContext,
};

use self::error::{SlotProcessingError, SlotsProcessorError};
use self::helpers::{create_tx_hash_versioned_hashes_mapping, create_versioned_hash_blob_mapping};

pub mod error;
mod helpers;

const MAX_ALLOWED_REORG_DEPTH: u32 = 100;

pub struct BlockData {
    pub root: B256,
    pub parent_root: B256,
    pub slot: u32,
    pub execution_block_hash: B256,
}

impl From<&BlockData> for BlockHeader {
    fn from(block: &BlockData) -> Self {
        BlockHeader {
            root: block.root,
            parent_root: block.parent_root,
            slot: block.slot,
        }
    }
}

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
            let block_header = match self
                .context
                .beacon_client()
                .get_block_header(current_slot.into())
                .await?
            {
                Some(header) => header,
                None => {
                    debug!(current_slot, "Skipping as there is no beacon block header");

                    continue;
                }
            };

            if !is_reverse {
                if let Some(prev_block_header) = last_processed_block {
                    if prev_block_header.root != B256::ZERO
                        && prev_block_header.root != block_header.parent_root
                    {
                        info!(
                            new_head_slot = block_header.slot,
                            old__head_slot = prev_block_header.slot,
                            new_head_block_root = ?block_header.root,
                            old_head_block_root = ?prev_block_header.root,
                            "Reorg detected!",
                        );

                        self.process_reorg(&prev_block_header, &block_header)
                            .await
                            .map_err(|error| SlotsProcessorError::FailedReorgProcessing {
                                old_slot: prev_block_header.slot,
                                new_slot: block_header.slot,
                                new_head_block_root: block_header.root,
                                old_head_block_root: prev_block_header.root,
                                error,
                            })?;
                    }
                }
            }

            if let Err(error) = self.process_block(&block_header).await {
                return Err(SlotsProcessorError::FailedSlotsProcessing {
                    initial_slot,
                    final_slot,
                    failed_slot: current_slot,
                    error,
                });
            }

            last_processed_block = Some(block_header);
        }

        self.last_processed_block = last_processed_block;

        Ok(())
    }

    async fn process_block(
        &self,
        beacon_block_header: &BlockHeader,
    ) -> Result<(), SlotProcessingError> {
        let beacon_client = self.context.beacon_client();
        let blobscan_client = self.context.blobscan_client();
        let provider = self.context.provider();
        let slot = beacon_block_header.slot;

        let beacon_block = match beacon_client.get_block(slot.into()).await? {
            Some(block) => block,
            None => {
                debug!(slot = slot, "Skipping as there is no beacon block");

                return Ok(());
            }
        };

        let execution_payload = match beacon_block.message.body.execution_payload {
            Some(payload) => payload,
            None => {
                debug!(
                    slot,
                    "Skipping as beacon block doesn't contain execution payload"
                );

                return Ok(());
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

            return Ok(());
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

                    return Ok(());
                } else {
                    blobs
                }
            }
            None => {
                debug!(slot, "Skipping as there is no blobs sidecar");

                return Ok(());
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

        Ok(())
    }

    /// Handles reorgs by rewinding the blobscan blocks to the common ancestor and forwarding to the new head.
    async fn process_reorg(
        &mut self,
        old_head_header: &BlockHeader,
        new_head_header: &BlockHeader,
    ) -> Result<(), anyhow::Error> {
        let mut current_old_slot = old_head_header.slot;
        let mut reorg_depth = 0;

        let mut rewinded_blocks: Vec<B256> = vec![];

        while reorg_depth <= MAX_ALLOWED_REORG_DEPTH && current_old_slot > 0 {
            reorg_depth += 1;

            // We iterate over blocks by slot and not block root as blobscan blocks don't
            // have parent root we can use to traverse the chain
            let old_blobscan_block = match self
                .context
                .blobscan_client()
                .get_block(current_old_slot)
                .await?
            {
                Some(block) => block,
                None => {
                    current_old_slot -= 1;

                    continue;
                }
            };

            let canonical_block_path = self
                .get_canonical_block_path(&old_blobscan_block, new_head_header.root)
                .await?;

            // If a path exists, we've found the common ancient block
            // and can proceed with handling the reorg.
            if !canonical_block_path.is_empty() {
                let canonical_block_path =
                    canonical_block_path.into_iter().rev().collect::<Vec<_>>();

                let canonical_block_headers: Vec<BlockHeader> = canonical_block_path
                    .iter()
                    .map(|block| block.into())
                    .collect::<Vec<_>>();

                // If the new canonical block path includes blocks beyond the new head block,
                // they were skipped and must be processed.
                for block in canonical_block_headers.iter() {
                    if block.slot != new_head_header.slot {
                        self.process_block(block)
                            .await
                            .with_context(|| format!("Failed to sync forwarded block"))?;
                    }
                }

                let forwarded_blocks = canonical_block_path
                    .iter()
                    .map(|block| block.execution_block_hash)
                    .collect::<Vec<_>>();

                self.context
                    .blobscan_client()
                    .handle_reorg(rewinded_blocks.clone(), forwarded_blocks.clone())
                    .await?;

                info!(rewinded_blocks = ?rewinded_blocks, forwarded_blocks = ?forwarded_blocks, "Reorg handled!",);

                return Ok(());
            }

            rewinded_blocks.push(old_blobscan_block.hash);
        }

        Err(anyhow!("No common block found").into())
    }

    /// Returns the path of blocks with execution payload from the head block to the provided block.
    async fn get_canonical_block_path(
        &mut self,
        blobscan_block: &BlobscanBlock,
        head_block_root: B256,
    ) -> Result<Vec<BlockData>, ClientError> {
        let beacon_client = self.context.beacon_client();
        let mut canonical_execution_blocks: Vec<BlockData> = vec![];

        let mut canonical_block = match beacon_client.get_block(head_block_root.into()).await? {
            Some(block) => block,
            None => {
                return Ok(vec![]);
            }
        };

        if let Some(execution_payload) = &canonical_block.message.body.execution_payload {
            if execution_payload.block_hash == blobscan_block.hash {
                return Ok(vec![]);
            }
        }

        let mut current_canonical_block_root = head_block_root;

        while canonical_block.message.parent_root != B256::ZERO {
            let canonical_block_parent_root = canonical_block.message.parent_root;

            if canonical_block.message.slot < blobscan_block.slot {
                return Ok(vec![]);
            }

            if let Some(execution_payload) = &canonical_block.message.body.execution_payload {
                if execution_payload.block_hash == blobscan_block.hash {
                    return Ok(canonical_execution_blocks);
                }

                canonical_execution_blocks.push(BlockData {
                    root: current_canonical_block_root,
                    parent_root: canonical_block_parent_root,
                    slot: canonical_block.message.slot,
                    execution_block_hash: execution_payload.block_hash,
                });
            }

            canonical_block = match beacon_client
                .get_block(canonical_block_parent_root.into())
                .await?
            {
                Some(block) => block,
                None => {
                    return Ok(vec![]);
                }
            };

            current_canonical_block_root = canonical_block_parent_root;
        }

        Ok(vec![])
    }
}
