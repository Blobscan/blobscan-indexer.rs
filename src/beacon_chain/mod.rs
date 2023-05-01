use reqwest::{Client, StatusCode};
use std::time::Duration;

use self::types::{
    BeaconAPIError, BeaconAPIResult, BlobsSidecar, BlobsSidecarResponse, BlockMessage as Block,
    BlockResponse,
};

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
    pub fn try_from(base_url: String, options: Option<Options>) -> BeaconAPIResult<Self> {
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

    pub async fn get_block(&self, slot: Option<u32>) -> BeaconAPIResult<Option<Block>> {
        let slot = match slot {
            Some(slot) => slot.to_string(),
            None => String::from("head"),
        };

        let url = self.build_url(&format!("eth/v2/beacon/blocks/{slot}"));

        let block_response = self.client.get(url).send().await?;

        match block_response.status() {
            StatusCode::OK => Ok(Some(
                block_response.json::<BlockResponse>().await?.data.message,
            )),
            StatusCode::NOT_FOUND => Ok(None),
            _ => Err(BeaconAPIError::JsonRpcClientError(
                block_response.text().await?,
            )),
        }
    }

    pub async fn get_blobs_sidecar(&self, slot: u32) -> BeaconAPIResult<Option<BlobsSidecar>> {
        let url = self.build_url(&format!("eth/v1/beacon/blobs/{slot}"));

        let sidecar_response = self.client.get(url).send().await?;

        match sidecar_response.status() {
            StatusCode::OK => Ok(Some(
                sidecar_response.json::<BlobsSidecarResponse>().await?.data,
            )),
            StatusCode::NOT_FOUND => Ok(None),
            _ => Err(BeaconAPIError::JsonRpcClientError(
                sidecar_response.text().await?,
            )),
        }
    }

    fn build_url(&self, path: &String) -> String {
        format!("{}/{}", self.base_url, path)
    }
}
