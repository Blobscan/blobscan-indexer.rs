#[derive(Debug, thiserror::Error)]
pub enum SlotProcessorError {
    #[error(transparent)]
    ClientError(#[from] crate::clients::common::ClientError),
    #[error(transparent)]
    Provider(#[from] ethers::providers::ProviderError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum SlotsProcessorError {
    #[error(
        "Error processing slots range {initial_slot}-{final_slot}. Slot {failed_slot} failed: {error}"
    )]
    FailedSlotsRange {
        initial_slot: u32,
        final_slot: u32,
        failed_slot: u32,
        error: SlotProcessorError,
    },
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
