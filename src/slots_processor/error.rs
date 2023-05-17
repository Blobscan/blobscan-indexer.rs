use super::slot_processor::error::SlotProcessorError;

#[derive(Debug, thiserror::Error)]
pub enum SlotsChunkThreadError {
    #[error(
        "Couldn't process slots chunk {initial_slot} to {final_slot}. Slot {failed_slot} failed: {error}"
    )]
    FailedChunkProcessing {
        initial_slot: u32,
        final_slot: u32,
        failed_slot: u32,
        error: SlotProcessorError,
    },

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum SlotsProcessorError {
    #[error("Failed to process slots from {initial_slot} to {final_slot}:\n{chunk_errors}")]
    FailedSlotsProcessing {
        initial_slot: u32,
        final_slot: u32,
        chunk_errors: MultipleSlotChunkErrors,
    },

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug)]
pub struct MultipleSlotChunkErrors(pub Vec<SlotsChunkThreadError>);

impl std::fmt::Display for MultipleSlotChunkErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for err in self.0.iter() {
            writeln!(f, "- {}", err)?;
        }
        Ok(())
    }
}
