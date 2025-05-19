use alloy::primitives::B256;

use crate::{clients::common::ClientError, synchronizer::error::SynchronizerError};

#[derive(Debug, thiserror::Error)]
pub enum SlotProcessingError {
    #[error(transparent)]
    ClientError(#[from] crate::clients::common::ClientError),
    #[error(transparent)]
    Provider(#[from] alloy::transports::TransportError),
    #[error("Operation timed out: {operation} for slot {slot}")]
    OperationTimeout {
        operation: String,
        slot: u32,
    },
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum SlotsProcessorError {
    #[error(
        "Error processing slots range {initial_slot}-{final_slot}. Slot {failed_slot} failed: {error}"
    )]
    FailedSlotsProcessing {
        initial_slot: u32,
        final_slot: u32,
        failed_slot: u32,
        error: SlotProcessingError,
    },
    #[error("Failed to process reorg. old slot {old_slot}, new slot {new_slot}, new head block root {new_head_block_root}, old head block root {old_head_block_root}")]
    FailedReorgProcessing {
        old_slot: u32,
        new_slot: u32,
        new_head_block_root: B256,
        old_head_block_root: B256,
        #[source]
        error: anyhow::Error,
    },
    #[error("Failed to handle reorged slots")]
    ReorgedFailure(#[from] ClientError),
    #[error("Failed to handle forwarded blocks")]
    ForwardedBlocksFailure(#[from] SynchronizerError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
