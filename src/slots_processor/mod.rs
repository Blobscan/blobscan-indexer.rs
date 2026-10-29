use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use alloy::{
    consensus::Transaction,
    eips::{
        eip4844::{kzg_to_versioned_hash, HeapBlob},
        BlockId as ExecutionBlockId,
    },
    primitives::B256,
};
use anyhow::{anyhow, Context as AnyhowContext, Result};

use crate::{
    clients::beacon::types::{Block as BeaconBlock, BlockExecutionPayload, BlockHeader, BlockId},
    utils::{alloy::BlobTransactionExt, futures::retry_on_none, kzg::compute_blob_proofs},
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

#[cfg(test)]
mod tests;

const MAX_ALLOWED_REORG_DEPTH: u32 = 100;

const RETRY_MAX_ATTEMPTS: u32 = 5;
const RETRY_DELAY: Duration = Duration::from_millis(500);

/// Whether the beacon node serves `/eth/v1/beacon/blobs` (the default) or only
/// `/eth/v1/beacon/blob_sidecars`. It depends on the node, not on the network. Measured on the
/// RPC endpoints (2026-10-24): gnosis, Lighthouse v8.2.1, only sidecars; sepolia,
/// Lighthouse v8.3.0-rc.0, only blobs; mainnet (v8.2.3) and hoodi (v8.3.2) both. The indexer
/// finds it out by itself:
/// the first route that answers 404 makes it try the other one, and it remembers the one that
/// worked. The value is process wide, so every processor starts with the right route.
static RAW_BLOBS_FIRST: AtomicBool = AtomicBool::new(true);

/// Number of beacon blocks that the processor keeps after it gets them.
///
/// A Gloas block needs its parent, and the processor goes through the slots in order. When it
/// goes forward, the parent of a block is the previous block. When it goes back, the next block
/// is the parent of the previous block. Three blocks are the minimum for the two directions.
const RECENT_BLOCKS_CAPACITY: usize = 4;

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
    /// The beacon blocks that the processor got last, by block root. The block that the
    /// processor used most recently is last. A block root identifies the content of a block, so
    /// an entry stays correct after a reorg. After a reorg, the processor does not ask for the old
    /// entries again, and new entries replace them.
    recent_blocks: Mutex<VecDeque<(B256, BeaconBlock)>>,
    /// The route to try first. `RAW_BLOBS_FIRST` in production; tests pass their own so they do
    /// not share it.
    raw_blobs_first: &'static AtomicBool,
}

impl SlotsProcessor {
    pub fn new(
        context: Box<dyn CommonContext>,
        last_processed_block: Option<BlockHeader>,
    ) -> SlotsProcessor {
        Self {
            context,
            last_processed_block,
            recent_blocks: Mutex::new(VecDeque::with_capacity(RECENT_BLOCKS_CAPACITY)),
            raw_blobs_first: &RAW_BLOBS_FIRST,
        }
    }

    #[cfg(test)]
    fn with_route_memory(mut self, memory: &'static AtomicBool) -> Self {
        self.raw_blobs_first = memory;
        self
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

    /// Indexes the execution payload that the block at `block_root` makes part of the chain.
    ///
    /// Before Gloas, this is the payload in the block body. From Gloas on (EIP-7732, ePBS), the
    /// block has only a bid, and the builder can keep the payload and not send it. The next block
    /// shows if the payload became part of the chain:
    ///
    /// - If the bid of the next block builds on that payload, the block is full.
    /// - If the bid of the next block builds on an older payload, the block is empty.
    ///
    /// For this reason, a Gloas block indexes the payload of its parent, if the parent is full.
    /// The indexer indexes the payload of a Gloas block when it processes the child.
    async fn index_block(&self, block_root: B256) -> Result<(), SlotProcessingError> {
        let blobscan_client = self.context.blobscan_client();
        let provider = self.context.provider();

        let beacon_block = self
            .get_beacon_block(block_root)
            .await?
            .with_context(|| "Block not found".to_string())?;

        let parent = if beacon_block.has_bid() {
            let parent_root = beacon_block.parent_root;

            Some(
                self.get_beacon_block(parent_root)
                    .await?
                    .with_context(|| format!("Parent block '{parent_root}' not found"))?,
            )
        } else {
            None
        };

        let Some(execution_block_hash) = beacon_block.canonical_payload_hash(parent.as_ref())
        else {
            match &parent {
                None => debug!(
                    block_root = ?block_root,
                    slot = beacon_block.slot,
                    "Skipping - block doesn't contain execution payload"
                ),
                // The builder did not send the payload, or sent it too late. Log it at INFO, so
                // that these payloads show in the normal logs.
                Some(
                    parent @ BeaconBlock {
                        execution_payload: Some(BlockExecutionPayload::Bid { block_hash, .. }),
                        ..
                    },
                ) => info!(
                    parent_block_root = ?beacon_block.parent_root,
                    parent_slot = parent.slot,
                    parent_bid_block_hash = ?block_hash,
                    slot = beacon_block.slot,
                    "Skipping - the parent block is empty: its payload is not part of the chain"
                ),
                // The last block before Gloas. The indexer indexed its payload with it.
                Some(parent) => debug!(
                    block_root = ?block_root,
                    slot = beacon_block.slot,
                    parent_slot = parent.slot,
                    "Skipping - the parent block is not a Gloas block"
                ),
            }

            return Ok(());
        };

        // The beacon block that has the payload: the parent for a Gloas block, the block itself
        // before Gloas.
        let (block_root, beacon_block) = match parent {
            Some(parent) => (beacon_block.parent_root, parent),
            None => (block_root, beacon_block),
        };

        let slot = beacon_block.slot;
        let blob_kzg_commitments = beacon_block.blob_kzg_commitments.unwrap_or_default();

        if blob_kzg_commitments.is_empty() {
            debug!(
                block_root = ?block_root,
                slot, "Skipping - block doesn't contain blob kzg commitments"
            );

            return Ok(());
        }

        // Fetch the execution block and do some checks. The payload is part of the chain, so the
        // execution client must know it. But the execution client can be some time behind the
        // beacon node. For this reason, try again before the error.
        let execution_block = retry_on_none(
            || async move {
                provider
                    .get_block(ExecutionBlockId::Hash(execution_block_hash.into()))
                    .full()
                    .await
            },
            RETRY_MAX_ATTEMPTS,
            RETRY_DELAY,
        )
        .await?
        .with_context(|| format!("Execution block '{execution_block_hash}' not found"))?;

        let blob_txs = execution_block.transactions.filter_blob_transactions();

        if blob_txs.is_empty() {
            return Err(anyhow!("Blocks mismatch: Consensus block \"{block_root}\" contains blob KZG commitments, but the corresponding execution block \"{execution_block_hash:#?}\" does not contain any blob transactions").into());
        }

        let raw_blobs = self.fetch_raw_blobs(slot).await?;

        // Create entities to be indexed
        let block_entity = Block::try_from((&execution_block, slot))?;
        let tx_entities = blob_txs
            .iter()
            .map(|tx| BlobscanTransaction::try_from((*tx, &execution_block)))
            .collect::<Result<Vec<BlobscanTransaction>>>()?;

        // Blobscan stores and serves the KZG proof of each blob, but `/eth/v1/beacon/blobs` does
        // not give it. Compute it here, after the block and transaction conversions, because
        // they cost less and can also fail. The proof is also verified. If the number of blobs is
        // not correct, or if a blob does not agree with its commitment, the slot fails. The
        // indexer does not store such a blob. Usually, this occurs after a reorg, because the
        // blobs come by slot.
        let blobs = compute_blob_proofs(raw_blobs, blob_kzg_commitments)
            .await
            .with_context(|| format!("Blobs of slot {slot}. The block was possibly reorged"))?;

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
                                "Blob not found for blob {i:?} with versioned hash {versioned_hash:?} from tx {tx_hash:?}"
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
        let time_since_block = SystemTime::now()
            .duration_since(UNIX_EPOCH + Duration::from_secs(execution_block.header.timestamp))
            .unwrap_or_default();

        info!(
            slot,
            block_number,
            time_since_block = %humantime::format_duration(time_since_block),
            "Block indexed successfully"
        );

        Ok(())
    }

    /// Returns the beacon block at `block_root`. It comes from `recent_blocks` if the processor
    /// got it a short time before, and from the beacon node if not.
    async fn get_beacon_block(&self, block_root: B256) -> Result<Option<BeaconBlock>> {
        {
            let mut recent_blocks = self.recent_blocks.lock().unwrap();

            if let Some(i) = recent_blocks
                .iter()
                .position(|(root, _)| *root == block_root)
            {
                let entry = recent_blocks.remove(i).unwrap();
                let block = entry.1.clone();

                recent_blocks.push_back(entry);

                return Ok(Some(block));
            }
        }

        let block = self.fetch_beacon_block(block_root).await?;

        if let Some(block) = &block {
            let mut recent_blocks = self.recent_blocks.lock().unwrap();

            if recent_blocks.len() == RECENT_BLOCKS_CAPACITY {
                recent_blocks.pop_front();
            }

            recent_blocks.push_back((block_root, block.clone()));
        }

        Ok(block)
    }

    async fn fetch_beacon_block(&self, block_root: B256) -> Result<Option<BeaconBlock>> {
        retry_on_none(
            || {
                let beacon_client = self.context.beacon_client();

                async move { beacon_client.get_block(block_root.into()).await }
            },
            RETRY_MAX_ATTEMPTS,
            RETRY_DELAY,
        )
        .await
        .map_err(Into::into)
    }

    /// Fetches the raw blobs of the block at `slot`, in the order of its KZG commitments.
    ///
    /// `/eth/v1/beacon/blobs` gives only the raw blobs, without commitments and proofs. The
    /// commitments come from the block, and `compute_blob_proofs` computes the proofs.
    ///
    /// The indexer gets the blobs by slot and not by block root. Some beacon nodes (for example,
    /// nodes that are not supernodes) cannot rebuild old blobs by block root. After a reorg, the
    /// blobs can be the blobs of a different block at the slot. `compute_blob_proofs` finds this,
    /// because it compares the number of blobs and verifies each blob.
    ///
    /// The route is `/eth/v1/beacon/blobs`, or `/eth/v1/beacon/blob_sidecars` for a node that
    /// does not serve the first one (see `RAW_BLOBS_FIRST`). A 404 on one route is tried on the
    /// other at once; only when both answer nothing does the fetch wait and try again, because
    /// a node can take a moment to have the blobs of a new head.
    async fn fetch_raw_blobs(&self, slot: u32) -> Result<Vec<HeapBlob>> {
        let raw_blobs = retry_on_none(
            || async move {
                let beacon_client = self.context.beacon_client();
                let raw_first = self.raw_blobs_first.load(Ordering::Relaxed);

                for raw in [raw_first, !raw_first] {
                    let blobs = if raw {
                        beacon_client.get_blobs(slot.into()).await?
                    } else {
                        beacon_client.get_blob_sidecar_blobs(slot.into()).await?
                    };

                    if let Some(blobs) = blobs.filter(|blobs| !blobs.is_empty()) {
                        if raw != raw_first {
                            info!(slot, raw, "The beacon node serves the blobs on the other route; using it from now on");
                            self.raw_blobs_first.store(raw, Ordering::Relaxed);
                        }

                        return Ok::<_, ClientError>(Some(blobs));
                    }
                }

                Ok(None)
            },
            RETRY_MAX_ATTEMPTS,
            RETRY_DELAY,
        )
        .await?
        .with_context(|| "Blobs not found".to_string())?;

        Ok(raw_blobs)
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
                    if let Some(canonical_block_path) = canonical_block_path {
                        let canonical_block_path =
                            canonical_block_path.into_iter().rev().collect::<Vec<_>>();

                        let forwarded_blocks = canonical_block_path
                            .iter()
                            .map(|block| block.execution_block_hash)
                            .collect::<Vec<_>>();

                        if rewinded_blocks.is_empty() && forwarded_blocks.is_empty() {
                            info!(
                                "Skipping reorg handling: no Blobscan blocks to rewind or forward"
                            );

                            return Ok(());
                        }

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

    /// Returns the canonical blocks from the head block down to the block that makes the payload
    /// of `blobscan_block` part of the chain. That last block is not in the result.
    ///
    /// Returns `None` if `blobscan_block` is not on the chain of the head block.
    ///
    /// Each block in the result makes a new execution payload part of the chain (see
    /// `Block::canonical_payload_hash`). `execution_block_hash` is the hash of that payload. A
    /// block that makes no payload part of the chain is not in the result.
    async fn get_canonical_block_path(
        &self,
        blobscan_block: &BlobscanBlock,
        head_block_root: B256,
    ) -> Result<Option<Vec<BlockData>>> {
        let mut canonical_execution_blocks: Vec<BlockData> = vec![];

        let mut canonical_block = match self.get_beacon_block(head_block_root).await? {
            Some(block) => block,
            None => {
                return Ok(None);
            }
        };

        let mut current_canonical_block_root = head_block_root;

        while canonical_block.parent_root != B256::ZERO {
            let canonical_block_parent_root = canonical_block.parent_root;

            if canonical_block.slot < blobscan_block.slot {
                return Ok(None);
            }

            // Only a Gloas block needs its parent to find its payload. For a block before Gloas,
            // compare the hash first: if it is the match, the parent is not necessary.
            let parent_block = if canonical_block.has_bid() {
                match self.get_beacon_block(canonical_block_parent_root).await? {
                    Some(block) => Some(block),
                    None => {
                        return Ok(None);
                    }
                }
            } else {
                None
            };

            if let Some(execution_block_hash) =
                canonical_block.canonical_payload_hash(parent_block.as_ref())
            {
                if execution_block_hash == blobscan_block.hash {
                    return Ok(Some(canonical_execution_blocks));
                }

                canonical_execution_blocks.push(BlockData {
                    root: current_canonical_block_root,
                    parent_root: canonical_block_parent_root,
                    slot: canonical_block.slot,
                    execution_block_hash,
                });
            }

            canonical_block = match parent_block {
                Some(block) => block,
                None => match self.get_beacon_block(canonical_block_parent_root).await? {
                    Some(block) => block,
                    None => {
                        return Ok(None);
                    }
                },
            };
            current_canonical_block_root = canonical_block_parent_root;
        }

        Ok(None)
    }
}
