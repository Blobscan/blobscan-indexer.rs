use crate::slot_processor::error::SlotProcessorError;

#[derive(Debug, thiserror::Error)]
pub enum SynchronizerThreadError {
    #[error(
        "Error processing slots chunk {initial_slot}-{final_slot}. Slot {failed_slot} failed: {error}"
    )]
    FailedSlotsChunkProcessing {
        initial_slot: u32,
        final_slot: u32,
        failed_slot: u32,
        error: SlotProcessorError,
    },

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum SynchronizerError {
    #[error("Failed to process slots from {initial_slot} to {final_slot}:\n{chunk_errors}")]
    FailedSlotsProcessing {
        initial_slot: u32,
        final_slot: u32,
        chunk_errors: MultipleSlotsChunkErrors,
    },
    #[error(transparent)]
    ClientError(#[from] crate::clients::common::ClientError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug)]
pub struct MultipleSlotsChunkErrors(pub Vec<SynchronizerThreadError>);

impl std::fmt::Display for MultipleSlotsChunkErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for err in self.0.iter() {
            writeln!(f, "- {}", err)?;
        }
        Ok(())
    }
}
