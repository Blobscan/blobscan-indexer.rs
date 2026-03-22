use std::time::Duration;

use alloy::{
    consensus::Transaction,
    eips::{eip4844::kzg_to_versioned_hash, BlockId as ExecutionBlockId},
    primitives::B256,
};
use anyhow::{anyhow, Context as AnyhowContext, Result};

use crate::{
    clients::beacon::types::{BlockHeader, BlockId},
    utils::{alloy::BlobTransactionExt, futures::retry_on_none},
};
use tracing::{debug, info, Instrument};

use crate::{
    clients::{
        blobscan::types::{Blob, BlobscanBlock, Block, Transaction as BlobscanTransaction},
        common::ClientError,
    },
    context::CommonContext,
};

use self::error::{SlotProcessingError, SlotsProcessorError};

pub mod error;

const MAX_ALLOWED_REORG_DEPTH: u32 = 100;

const RETRY_MAX_ATTEMPTS: u32 = 5;
const RETRY_DELAY: Duration = Duration::from_millis(500);

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

pub struct SlotsProcessor {
    context: Box<dyn CommonContext>,
    pub last_processed_block: Option<BlockHeader>,
}

impl SlotsProcessor {
    pub fn new(
        context: Box<dyn CommonContext>,
        last_processed_block: Option<BlockHeader>,
    ) -> SlotsProcessor {
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
        let is_reverse_processing = initial_slot > final_slot;
        let slots = if is_reverse_processing {
            (final_slot..initial_slot).rev().collect::<Vec<_>>()
        } else {
            (initial_slot..final_slot).collect::<Vec<_>>()
        };

        for current_slot in slots {
            let block_header = match self
                .context
                .beacon_client()
                .get_block_header(current_slot.into())
                .await?
            {
                Some(header) => header,
                None => {
                    debug!(current_slot, "Skipping - empty slot");

                    continue;
                }
            };

            self.process_block_header(block_header, !is_reverse_processing)
                .await
                .map_err(|error| SlotsProcessorError::FailedSlotsProcessing {
                    initial_slot,
                    final_slot,
                    failed_slot: current_slot,
                    error,
                })?;
        }

        Ok(())
    }

    pub async fn process_block(&mut self, block_id: BlockId) -> Result<(), SlotsProcessorError> {
        let block_header = retry_on_none(
            || {
                let beacon_client = self.context.beacon_client();
                let block_id = block_id.clone();

                async move { beacon_client.get_block_header(block_id).await }
            },
            RETRY_MAX_ATTEMPTS,
            RETRY_DELAY,
        )
        .await?
        .with_context(|| format!("Block header with id '{block_id}' not found"))?;

        self.process_block_header(block_header.clone(), true)
            .await
            .map_err(|error| SlotsProcessorError::FailedBlockProcessing {
                block_root: block_header.root,
                error,
                slot: block_header.slot,
            })
    }

    async fn process_block_header(
        &mut self,
        block_header: BlockHeader,
        detect_reorgs: bool,
    ) -> Result<(), SlotProcessingError> {
        if detect_reorgs && self.check_reorg(&block_header) {
            self.process_reorg(&block_header).await?;
        }

        let block_root = block_header.root;
        let block_slot = block_header.slot;

        self.index_block(block_header.root).await.with_context(|| {
            format!("Failed to index block with root '{block_root}' at slot {block_slot}")
        })?;

        self.last_processed_block = Some(block_header);

        Ok(())
    }

    async fn index_block(&self, block_root: B256) -> Result<(), SlotProcessingError> {
        let blobscan_client = self.context.blobscan_client();
        let provider = self.context.provider();

        let beacon_block = retry_on_none(
            || {
                let beacon_client = self.context.beacon_client();

                async move { beacon_client.get_block(block_root.into()).await }
            },
            RETRY_MAX_ATTEMPTS,
            RETRY_DELAY,
        )
        .await?
        .with_context(|| "Block not found".to_string())?;

        let slot = beacon_block.slot;

        let execution_payload = match beacon_block.execution_payload {
            Some(payload) => payload,
            None => {
                debug!(
                    block_root = ?block_root,
                    slot, "Skipping - block doesn't contain execution payload"
                );

                return Ok(());
            }
        };

        let has_blobs = match beacon_block.blob_kzg_commitments {
            Some(commitments) => !commitments.is_empty(),
            None => false,
        };

        if !has_blobs {
            debug!(
                block_root = ?block_root,
                slot, "Skipping - block doesn't contain blob kzg commitments"
            );

            return Ok(());
        }

        let execution_block_hash = execution_payload.block_hash;

        // Fetch execution block and perform some checks

        let execution_block = provider
            .get_block(ExecutionBlockId::Hash(execution_block_hash.into()))
            .full()
            .await?
            .with_context(|| format!("Execution block '{execution_block_hash}' not found"))?;

        let blob_txs = execution_block.transactions.filter_blob_transactions();

        if blob_txs.is_empty() {
            return Err(anyhow!("Blocks mismatch: Consensus block \"{block_root}\" contains blob KZG commitments, but the corresponding execution block \"{execution_block_hash:#?}\" does not contain any blob transactions").into());
        }

        let blobs = retry_on_none(
            || {
                let beacon_client = self.context.beacon_client();

                async move {
                    let blobs = beacon_client.get_blobs(block_root.into()).await?;

                    match blobs {
                        Some(blobs) if blobs.is_empty() => Ok::<_, ClientError>(None),
                        other => Ok(other),
                    }
                }
            },
            RETRY_MAX_ATTEMPTS,
            RETRY_DELAY,
        )
        .await?
        .with_context(|| "Blobs sidecar not found".to_string())?;

        if blobs.is_empty() {
            return Err(anyhow!("Blobs sidecar is empty").into());
        }

        // Create entities to be indexed
        let block_entity = Block::try_from((&execution_block, slot))?;
        let tx_entities = blob_txs
            .iter()
            .map(|tx| BlobscanTransaction::try_from((*tx, &execution_block)))
            .collect::<Result<Vec<BlobscanTransaction>>>()?;

        let blob_entities = blob_txs
            .into_iter()
            .flat_map(|tx| {
               tx.blob_versioned_hashes()
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .map( |(i, versioned_hash)| {
                        let tx_hash = tx.inner.hash();
                        let blob = blobs
                            .iter()
                            .find(|blob| {
                                let vh = kzg_to_versioned_hash(blob.kzg_commitment.as_ref());

                                vh.eq(versioned_hash)
                            })
                            .with_context(|| format!(
                                "Sidecar not found for blob {i:?} with versioned hash {versioned_hash:?} from tx {tx_hash:?}"
                            ))?;

                        Ok(Blob::from((blob, (i as u32), tx_hash)))
                    })
            })
            .collect::<Result<Vec<Blob>, anyhow::Error>>()?;

        blobscan_client
            .index(block_entity, tx_entities, blob_entities)
            .await
            .map_err(SlotProcessingError::ClientError)?;

        let block_number = execution_block.header.number;

        info!(slot, block_number, "Block indexed successfully");

        Ok(())
    }

    /// Returns true if the current block's parent root doesn't match the last processed block root,
    /// indicating the chain has reorged.
    fn check_reorg(&self, curr_block_header: &BlockHeader) -> bool {
        if let Some(prev_block_header) = self.last_processed_block.as_ref() {
            if prev_block_header.root != B256::ZERO
                && prev_block_header.root != curr_block_header.parent_root
            {
                info!(
                    new_head_slot = curr_block_header.slot,
                    old_head_slot = prev_block_header.slot,
                    new_head_block_root = ?curr_block_header.root,
                    old_head_block_root = ?prev_block_header.root,
                    "Reorg detected!",
                );

                return true;
            }
        }

        false
    }

    /// Handles reorgs by rewinding the blobscan blocks to the common ancestor and forwarding to the new head.
    async fn process_reorg(&mut self, new_head_header: &BlockHeader) -> Result<(), anyhow::Error> {
        if let Some(old_head_header) = self.last_processed_block.as_ref() {
            let mut current_old_slot = old_head_header.slot;
            let mut reorg_depth = 0;

            let mut rewinded_blocks: Vec<B256> = vec![];

            while reorg_depth <= MAX_ALLOWED_REORG_DEPTH && current_old_slot > 0 {
                // We iterate over blocks by slot and not block root as blobscan blocks don't
                // have parent root we can use to traverse the chain
                if let Some(old_blobscan_block) = self
                    .context
                    .blobscan_client()
                    .get_block(current_old_slot)
                    .await?
                {
                    let canonical_block_path = self
                        .get_canonical_block_path(&old_blobscan_block, new_head_header.root)
                        .await?;

                    // If a path exists, we've found the common ancient block
                    if !canonical_block_path.is_empty() {
                        let canonical_block_path =
                            canonical_block_path.into_iter().rev().collect::<Vec<_>>();

                        let forwarded_blocks = canonical_block_path
                            .iter()
                            .map(|block| block.execution_block_hash)
                            .collect::<Vec<_>>();

                        self.context
                            .blobscan_client()
                            .handle_reorg(rewinded_blocks.clone(), forwarded_blocks.clone())
                            .await?;

                        info!(rewinded_blocks = ?rewinded_blocks, forwarded_blocks = ?forwarded_blocks, "Reorg handled!");

                        let canonical_block_headers: Vec<BlockHeader> = canonical_block_path
                            .iter()
                            .map(|block| block.into())
                            .collect::<Vec<_>>();

                        // If the new canonical block path includes blocks beyond the new head block,
                        // they were skipped and must be processed.
                        for block in canonical_block_headers.iter() {
                            if block.slot != new_head_header.slot {
                                let reorg_span = tracing::info_span!(
                                    parent: &tracing::Span::current(),
                                    "forwarded_block",
                                );

                                self.index_block(block.root)
                                    .instrument(reorg_span)
                                    .await
                                    .with_context(|| {
                                        "Failed to sync forwarded block".to_string()
                                    })?;
                            }
                        }

                        return Ok(());
                    }

                    rewinded_blocks.push(old_blobscan_block.hash);
                }

                current_old_slot -= 1;
                reorg_depth += 1;
            }

            let rewinded_blocks_count = rewinded_blocks.len();

            if rewinded_blocks_count > 0 {
                return Err(anyhow!("{rewinded_blocks_count} Blobscan blocks to rewind detected but no common ancestor found"));
            }

            info!("Skipping reorg handling: no Blobscan blocks to rewind found");
        }

        Ok(())
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

        if let Some(execution_payload) = &canonical_block.execution_payload {
            if execution_payload.block_hash == blobscan_block.hash {
                return Ok(vec![]);
            }
        }

        let mut current_canonical_block_root = head_block_root;

        while canonical_block.parent_root != B256::ZERO {
            let canonical_block_parent_root = canonical_block.parent_root;

            if canonical_block.slot < blobscan_block.slot {
                return Ok(vec![]);
            }

            if let Some(execution_payload) = &canonical_block.execution_payload {
                if execution_payload.block_hash == blobscan_block.hash {
                    return Ok(canonical_execution_blocks);
                }

                canonical_execution_blocks.push(BlockData {
                    root: current_canonical_block_root,
                    parent_root: canonical_block_parent_root,
                    slot: canonical_block.slot,
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
