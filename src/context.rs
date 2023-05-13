use std::{sync::Arc, time::Duration};

use anyhow::Result;
use ethers::prelude::*;

use crate::{
    beacon_client::{BeaconClient, Config as BeaconClientConfig},
    blobscan_client::{BlobscanClient, Config as BlobscanClientConfig},
};

use super::env::{get_env_vars, Environment};

#[derive(Debug, Clone)]
struct ContextRef {
    pub beacon_client: BeaconClient,
    pub blobscan_client: BlobscanClient,
    pub provider: Provider<Http>,
}

#[derive(Debug, Clone)]
pub struct Context {
    inner: Arc<ContextRef>,
}

impl Context {
    pub fn try_new() -> Result<Self> {
        let Environment {
            blobscan_api_endpoint,
            beacon_node_rpc,
            execution_node_rpc,
            secret_key,
            ..
        } = get_env_vars();

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .build()?;

        Ok(Self {
            inner: Arc::new(ContextRef {
                blobscan_client: BlobscanClient::with_client(
                    client.clone(),
                    BlobscanClientConfig {
                        base_url: blobscan_api_endpoint,
                        secret_key,
                        timeout: None,
                    },
                ),
                beacon_client: BeaconClient::with_client(
                    client,
                    BeaconClientConfig {
                        base_url: beacon_node_rpc,
                        timeout: None,
                    },
                ),
                provider: Provider::<Http>::try_from(execution_node_rpc)?,
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
