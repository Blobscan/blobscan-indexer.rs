#[derive(Debug, thiserror::Error)]
pub enum SingleSlotProcessingError {
    #[error(transparent)]
    BlobscanClient(#[from] crate::blobscan_client::types::BlobscanClientError),
    #[error(transparent)]
    BeaconClient(#[from] crate::beacon_client::types::BeaconClientError),
    #[error(transparent)]
    Provider(#[from] ethers::providers::ProviderError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum SlotProcessorError {
    #[error("Failed to process slot {slot}: {reason}")]
    ProcessingError {
        slot: u32,
        target_slot: u32,
        reason: SingleSlotProcessingError,
    },
}
