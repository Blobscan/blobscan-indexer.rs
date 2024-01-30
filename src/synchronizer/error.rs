use crate::slot_processor::error::SlotsProcessorError;

#[derive(Debug, thiserror::Error)]
pub enum SynchronizerError {
    #[error("Failed to process slots from {initial_slot} to {final_slot}:\n{chunk_errors}")]
    FailedParallelSlotsProcessing {
        initial_slot: u32,
        final_slot: u32,
        chunk_errors: MultipleSlotsChunkErrors,
    },
    #[error(transparent)]
    FailedSlotsProcessing(#[from] SlotsProcessorError),
    #[error(transparent)]
    ClientError(#[from] crate::clients::common::ClientError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug)]
pub struct MultipleSlotsChunkErrors(pub Vec<SlotsProcessorError>);

impl std::fmt::Display for MultipleSlotsChunkErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for err in self.0.iter() {
            writeln!(f, "- {}", err)?;
        }
        Ok(())
    }
}
