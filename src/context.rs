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
        let request_timeout = Some(Duration::from_secs(8));

        Ok(Self {
            inner: Arc::new(ContextRef {
                blobscan_client: BlobscanClient::try_from(BlobscanClientConfig {
                    base_url: blobscan_api_endpoint,
                    secret_key,
                    timeout: request_timeout,
                })?,
                beacon_client: BeaconClient::try_from(BeaconClientConfig {
                    base_url: beacon_node_rpc,
                    timeout: request_timeout,
                })?,
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
