#[derive(Debug, thiserror::Error)]
pub enum SlotProcessorError {
    #[error(transparent)]
    ClientError(#[from] crate::clients::common::ClientError),
    #[error(transparent)]
    Provider(#[from] ethers::providers::ProviderError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
