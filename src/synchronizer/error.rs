use crate::{
    clients::beacon::types::BlockIdResolutionError, slots_processor::error::SlotsProcessorError,
};

#[derive(Debug, thiserror::Error)]
pub enum SynchronizerError {
    #[error(
        "Failed to parallel process slots from {initial_slot} to {final_slot}:\n{chunk_errors}"
    )]
    FailedParallelSlotsProcessing {
        initial_slot: u32,
        final_slot: u32,
        chunk_errors: SlotsChunksErrors,
    },
    #[error(transparent)]
    FailedBlockIdResolution(#[from] BlockIdResolutionError),
    #[error("Failed to save slot checkpoint for slot {slot}: {error}")]
    FailedSlotCheckpointSave {
        slot: u32,
        error: crate::clients::common::ClientError,
    },
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug)]
pub struct SlotsChunksErrors(pub Vec<SlotsProcessorError>);

impl std::fmt::Display for SlotsChunksErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for err in self.0.iter() {
            writeln!(f, "- {}", err)?;
        }
        Ok(())
    }
}
