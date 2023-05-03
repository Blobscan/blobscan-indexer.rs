use reqwest::{Client, StatusCode};
use std::time::Duration;

use self::types::{
    BeaconClientError, BeaconClientResult, BlobData, BlobsResponse, BlockMessage as Block,
    BlockResponse,
};

pub mod types;

#[derive(Debug, Clone)]
pub struct BeaconClient {
    base_url: String,
    client: reqwest::Client,
}

pub struct Config {
    pub base_url: String,
    pub timeout: Option<Duration>,
}

impl BeaconClient {
    pub fn try_from(config: Config) -> BeaconClientResult<Self> {
        let mut client_builder = Client::builder();

        if let Some(timeout) = config.timeout {
            client_builder = client_builder.timeout(timeout);
        }

        Ok(Self {
            base_url: config.base_url,
            client: client_builder.build()?,
        })
    }

    pub async fn get_block(&self, slot: Option<u32>) -> BeaconClientResult<Option<Block>> {
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
            _ => Err(BeaconClientError::JsonRpcClientError(
                block_response.text().await?,
            )),
        }
    }

    pub async fn get_blobs(&self, slot: u32) -> BeaconClientResult<Option<Vec<BlobData>>> {
        let url = self.build_url(&format!("eth/v1/beacon/blobs/{slot}"));

        let blobs_response = self.client.get(url).send().await?;

        match blobs_response.status() {
            StatusCode::OK => Ok(Some(blobs_response.json::<BlobsResponse>().await?.data)),
            StatusCode::NOT_FOUND => Ok(None),
            _ => Err(BeaconClientError::JsonRpcClientError(
                blobs_response.text().await?,
            )),
        }
    }

    fn build_url(&self, path: &String) -> String {
        format!("{}/{}", self.base_url, path)
    }
}
