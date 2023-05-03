use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::types::{BlobEntity, BlockEntity, TransactionEntity};

#[derive(Serialize, Deserialize, Debug)]
pub struct SlotResponse {
    pub slot: u32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SlotRequest {
    pub slot: u32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IndexRequest {
    pub block: BlockEntity,
    pub transactions: Vec<TransactionEntity>,
    pub blobs: Vec<BlobEntity>,
}

#[derive(Debug, thiserror::Error)]
pub enum BlobscanClientError {
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),

    #[error("Blobscan client error: {0}")]
    BlobscanClientError(String),

    #[error(transparent)]
    JWTError(#[from] anyhow::Error),
}

pub type BlobscanClientResult<T> = Result<T, BlobscanClientError>;
