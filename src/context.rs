use std::{sync::Arc, time::Duration};

use anyhow::Result as AnyhowResult;
use backoff::ExponentialBackoffBuilder;
use ethers::prelude::*;

use crate::{
    clients::beacon::{BeaconClient, Config as BeaconClientConfig},
    clients::blobscan::{BlobscanClient, Config as BlobscanClientConfig},
    env::Environment,
};

#[derive(Debug, Clone)]
struct ContextRef {
    pub beacon_client: BeaconClient,
    pub blobscan_client: BlobscanClient,
    pub provider: Provider<Http>,
}

pub struct Config {
    pub blobscan_api_endpoint: String,
    pub beacon_node_url: String,
    pub execution_node_endpoint: String,
    pub secret_key: String,
}

#[derive(Debug, Clone)]
pub struct Context {
    inner: Arc<ContextRef>,
}

impl Context {
    pub fn try_new(config: Config) -> AnyhowResult<Self> {
        let Config {
            blobscan_api_endpoint,
            beacon_node_url,
            execution_node_endpoint,
            secret_key,
        } = config;
        let exp_backoff = Some(ExponentialBackoffBuilder::default().build());

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .build()?;

        Ok(Self {
            inner: Arc::new(ContextRef {
                blobscan_client: BlobscanClient::try_with_client(
                    client.clone(),
                    BlobscanClientConfig {
                        base_url: blobscan_api_endpoint,
                        secret_key,
                        exp_backoff: exp_backoff.clone(),
                    },
                )?,
                beacon_client: BeaconClient::try_with_client(
                    client,
                    BeaconClientConfig {
                        base_url: beacon_node_url,
                        exp_backoff,
                    },
                )?,
                provider: Provider::<Http>::try_from(execution_node_endpoint)?,
            }),
        })
    }

    pub fn beacon_client(&self) -> &BeaconClient {
        &self.inner.beacon_client
    }

    pub fn blobscan_client(&self) -> &BlobscanClient {
        &self.inner.blobscan_client
    }

    pub fn provider(&self) -> &Provider<Http> {
        &self.inner.provider
    }
}

impl From<Environment> for Config {
    fn from(env: Environment) -> Self {
        Self {
            blobscan_api_endpoint: env.blobscan_api_endpoint,
            beacon_node_url: env.beacon_node_endpoint,
            execution_node_endpoint: env.execution_node_endpoint,
            secret_key: env.secret_key,
        }
    }
}
