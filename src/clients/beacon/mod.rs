use anyhow::Context as AnyhowContext;
use reqwest::{Client, Url};
use reqwest_eventsource::EventSource;
use std::time::Duration;

use crate::{clients::common::ClientResult, json_get};

use self::types::{Blob, BlobsResponse, BlockMessage as Block, BlockResponse, Topic};

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
        let path = format!("v1/beacon/blob_sidecars/{slot}");
        let url = self.base_url.join(path.as_str())?;

        json_get!(&self.client, url, BlobsResponse).map(|res| match res {
            Some(r) => Some(r.data),
            None => None,
        })
    }

    pub fn subscribe_to_events(&self, topics: Vec<Topic>) -> ClientResult<EventSource> {
        let topics = topics
            .iter()
            .map(|topic| topic.to_string())
            .collect::<Vec<String>>()
            .join("&");
        let path = format!("v1/events?topics={topics}");
        let url = self.base_url.join(&path)?;

        let event_source = EventSource::get(url);

        Ok(event_source)
    }
}
