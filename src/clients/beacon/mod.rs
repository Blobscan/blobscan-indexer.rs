use anyhow::Context as AnyhowContext;
use backoff::ExponentialBackoff;
use reqwest::{Client, Url};

use crate::{clients::common::ClientResult, json_get};

use self::types::{Blob, BlobsResponse, BlockId, BlockMessage as Block, BlockResponse};

pub mod types;

#[derive(Debug, Clone)]
pub struct BeaconClient {
    base_url: Url,
    client: Client,
    exp_backoff: Option<ExponentialBackoff>,
}

pub struct Config {
    pub base_url: String,
    pub exp_backoff: Option<ExponentialBackoff>,
}

impl BeaconClient {
    pub fn try_with_client(client: Client, config: Config) -> ClientResult<Self> {
        let base_url = Url::parse(&format!("{}/eth/", config.base_url))
            .with_context(|| "Failed to parse base URL")?;
        let exp_backoff = config.exp_backoff;

        Ok(Self {
            base_url,
            client,
            exp_backoff,
        })
    }

    pub async fn get_block(&self, slot: BlockId) -> ClientResult<Option<Block>> {
        let path = format!("v2/beacon/blocks/{slot}");
        let url = self.base_url.join(path.as_str())?;

        json_get!(&self.client, url, BlockResponse, self.exp_backoff.clone()).map(|res| match res {
            Some(r) => Some(r.data.message),
            None => None,
        })
    }

    pub async fn get_blobs(&self, slot: BlockId) -> ClientResult<Option<Vec<Blob>>> {
        let path = format!("v1/beacon/blob_sidecars/{slot}");
        let url = self.base_url.join(path.as_str())?;

        json_get!(&self.client, url, BlobsResponse, self.exp_backoff.clone()).map(|res| match res {
            Some(r) => Some(r.data),
            None => None,
        })
    }
}
