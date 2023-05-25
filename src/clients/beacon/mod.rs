use anyhow::Context as AnyhowContext;
use reqwest::{Client, Url};
use std::time::Duration;

use crate::{clients::common::ClientResult, json_get};

use self::types::{Blob, BlobsResponse, BlockMessage as Block, BlockResponse};

pub mod types;

#[derive(Debug, Clone)]
pub struct BeaconClient {
    base_url: Url,
    client: Client,
}

pub struct Config {
    pub base_url: String,
    pub timeout: Option<Duration>,
}

impl BeaconClient {
    pub fn try_with_client(client: Client, config: Config) -> ClientResult<Self> {
        let base_url = Url::parse(&format!("{}/eth/", config.base_url))
            .with_context(|| "Failed to parse base URL")?;

        Ok(Self { base_url, client })
    }

    pub async fn get_block(&self, slot: Option<u32>) -> ClientResult<Option<Block>> {
        let slot = match slot {
            Some(slot) => slot.to_string(),
            None => String::from("head"),
        };
        let path = format!("v2/beacon/blocks/{slot}");
        let url = self.base_url.join(path.as_str())?;

        json_get!(&self.client, url, BlockResponse).map(|res| match res {
            Some(r) => Some(r.data.message),
            None => None,
        })
    }

    pub async fn get_blobs(&self, slot: u32) -> ClientResult<Option<Vec<Blob>>> {
        let path = format!("v1/beacon/blobs/{slot}");
        let url = self.base_url.join(path.as_str())?;

        json_get!(&self.client, url, BlobsResponse).map(|res| match res {
            Some(r) => Some(r.data),
            None => None,
        })
    }
}
