use reqwest::{Client, StatusCode};
use std::time::Duration;

use crate::types::StdError;

use self::types::{BlobsSidecar, BlobsSidecarResponse, BlockMessage as Block, BlockResponse};

mod types;

#[derive(Debug, Clone)]
pub struct BeaconChainAPI {
    base_url: String,
    client: reqwest::Client,
}

pub struct Options {
    pub timeout: Option<u64>,
}

impl BeaconChainAPI {
    pub fn try_from(base_url: String, options: Option<Options>) -> Result<Self, StdError> {
        let mut client_builder = Client::builder();

        if let Some(options) = options {
            if let Some(timeout) = options.timeout {
                client_builder = client_builder.timeout(Duration::from_secs(timeout));
            }
        }

        Ok(Self {
            base_url,
            client: client_builder.build()?,
        })
    }

    pub async fn get_block(&self, slot: Option<u32>) -> Result<Option<Block>, StdError> {
        let slot = match slot {
            Some(slot) => slot.to_string(),
            None => String::from("head"),
        };
        let url = self.build_url(&format!("/eth/v2/beacon/blocks/{}", slot));

        let block_response = self.client.get(url).send().await?;

        match block_response.status() {
            StatusCode::OK => Ok(Some(
                block_response.json::<BlockResponse>().await?.data.message,
            )),
            StatusCode::NOT_FOUND => Ok(None),
            _ => Err(format!(
                "Couldn't fetch beacon block at slot {}: {}",
                slot,
                block_response.text().await?
            )
            .into()),
        }
    }

    pub async fn get_blobs_sidecar(&self, slot: u32) -> Result<Option<BlobsSidecar>, StdError> {
        let url = self.build_url(&format!("/eth/v1/beacon/blobs_sidecars/{}", slot));

        let sidecar_response = self.client.get(url).send().await?;

        match sidecar_response.status() {
            StatusCode::OK => Ok(Some(
                sidecar_response.json::<BlobsSidecarResponse>().await?.data,
            )),
            StatusCode::NOT_FOUND => Ok(None),
            _ => Err(format!(
                "Couldn't fetch blobs sidecar at slot {}: {}",
                slot,
                sidecar_response.text().await?
            )
            .into()),
        }
    }

    fn build_url(&self, path: &String) -> String {
        format!("{}/{}", self.base_url, path)
    }
}
