use anyhow::Result;
use ethers::prelude::*;

use crate::{
    beacon_chain::{BeaconChainAPI, Options as BeaconChainAPIOptions},
    blobscan::{BlobscanAPI, Config as BlobscanAPIConfig},
};

use super::env::{get_env_vars, Environment};

#[derive(Debug)]
pub struct Context {
    pub beacon_api: BeaconChainAPI,
    pub blobscan_api: BlobscanAPI,
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
    let request_timeout = Some(8);

    Ok(Context {
        blobscan_api: BlobscanAPI::try_from(BlobscanAPIConfig {
            base_url: blobscan_api_endpoint,
            secret_key,
            timeout: request_timeout,
        })?,
        beacon_api: BeaconChainAPI::try_from(
            beacon_node_rpc,
            Some(BeaconChainAPIOptions {
                timeout: request_timeout,
            }),
        )?,
        provider: Provider::<Http>::try_from(execution_node_rpc)?,
    })
}
