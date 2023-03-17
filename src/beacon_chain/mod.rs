use std::error;

use reqwest::StatusCode;

use self::types::{BlobsSidecar, BlobsSidecarResponse, BlockMessage as Block, BlockResponse};

mod types;

pub struct BeaconChainAPI {
    rpc_url: String,
}

impl BeaconChainAPI {
    pub fn new(rpc_url: String) -> Self {
        Self { rpc_url }
    }

    pub async fn get_block(&self, slot: Option<u32>) -> Result<Block, Box<dyn error::Error>> {
        let slot = match slot {
            Some(slot) => slot.to_string(),
            None => String::from("head"),
        };
        let block_response =
            reqwest::get(format!("{}/eth/v2/beacon/blocks/{}", self.rpc_url, slot)).await?;

        // TODO: handle rest of the response cases. For now, we just skip the slot if there is no block
        if block_response.status() != StatusCode::OK {
            return Err(format!("No block found on slot {}", slot).into());
        }

        let block_response = block_response.json::<BlockResponse>().await?;

        Ok(block_response.data.message)
    }

    pub async fn get_blobs_sidecar(
        &self,
        slot: u32,
    ) -> Result<BlobsSidecar, Box<dyn error::Error>> {
        let sidecar_response = reqwest::get(format!(
            "{}/eth/v1/beacon/blobs_sidecars/{}",
            self.rpc_url, slot
        ))
        .await?;

        // TODO: handle rest of the response cases. For now, we just skip the slot if there is no sidecar
        if sidecar_response.status() != StatusCode::OK {
            return Err(format!("No sidecar found on slot {}", slot).into());
        }

        let sidecar_response = sidecar_response.json::<BlobsSidecarResponse>().await?;

        Ok(sidecar_response.data)
    }
}
