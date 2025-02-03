use crate::clients::common::ClientError;

#[derive(Debug, thiserror::Error)]
pub enum SlotProcessingError {
    #[error(transparent)]
    ClientError(#[from] crate::clients::common::ClientError),
    #[error(transparent)]
    Provider(#[from] alloy::transports::TransportError),

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
    #[error("Failed to handle reorged slots")]
    ReorgedFailure(#[from] ClientError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
