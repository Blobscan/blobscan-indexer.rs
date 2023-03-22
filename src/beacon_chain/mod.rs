use reqwest::StatusCode;

use crate::types::StdError;

use self::types::{BlobsSidecar, BlobsSidecarResponse, BlockMessage as Block, BlockResponse};

mod types;

#[derive(Debug)]
pub struct BeaconChainAPI {
    rpc_url: String,
}

impl BeaconChainAPI {
    pub fn new(rpc_url: String) -> Self {
        Self { rpc_url }
    }

    pub async fn get_block(&self, slot: Option<u32>) -> Result<Option<Block>, StdError> {
        let slot = match slot {
            Some(slot) => slot.to_string(),
            None => String::from("head"),
        };
        let block_response =
            reqwest::get(format!("{}/eth/v2/beacon/blocks/{}", self.rpc_url, slot)).await?;

        if block_response.status() != StatusCode::OK {
            if block_response.status() == StatusCode::NOT_FOUND {
                return Ok(None);
            }

            return Err("Couldn't fetch beacon block".into());
        }

        let block_response = block_response.json::<BlockResponse>().await?;

        Ok(Some(block_response.data.message))
    }

    pub async fn get_blobs_sidecar(&self, slot: u32) -> Result<Option<BlobsSidecar>, StdError> {
        let sidecar_response = reqwest::get(format!(
            "{}/eth/v1/beacon/blobs_sidecars/{}",
            self.rpc_url, slot
        ))
        .await?;

        if sidecar_response.status() != StatusCode::OK {
            if sidecar_response.status() == StatusCode::NOT_FOUND {
                return Ok(None);
            }

            return Err("Couldn't fetch blobs sidecar".into());
        }

        let sidecar_response = sidecar_response.json::<BlobsSidecarResponse>().await?;

        Ok(Some(sidecar_response.data))
    }
}
