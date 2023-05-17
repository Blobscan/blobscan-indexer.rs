#[derive(Debug, thiserror::Error)]
pub enum SlotProcessorError {
    #[error(transparent)]
    BlobscanClient(#[from] crate::blobscan_client::types::BlobscanClientError),
    #[error(transparent)]
    BeaconClient(#[from] crate::beacon_client::types::BeaconClientError),
    #[error(transparent)]
    Provider(#[from] ethers::providers::ProviderError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
