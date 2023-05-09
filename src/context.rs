use std::time::Duration;

use anyhow::Result;
use ethers::prelude::*;

use crate::{
    beacon_client::{BeaconClient, Config as BeaconClientConfig},
    blobscan_client::{BlobscanClient, Config as BlobscanClientConfig},
};

use super::env::{get_env_vars, Environment};

#[derive(Debug, Clone)]
pub struct Context {
    pub beacon_client: BeaconClient,
    pub blobscan_client: BlobscanClient,
    pub provider: Provider<Http>,
}

pub fn create_context() -> Result<Context> {
    let Environment {
        blobscan_api_endpoint,
        beacon_node_rpc,
        execution_node_rpc,
        secret_key,
        ..
    } = get_env_vars();
    let request_timeout = Some(Duration::from_secs(8));

    Ok(Context {
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
    })
}
