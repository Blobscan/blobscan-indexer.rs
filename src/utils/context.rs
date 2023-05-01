use anyhow::Result;
use ethers::prelude::*;

use crate::{
    beacon_chain::{BeaconChainAPI, Options as BeaconChainAPIOptions},
    blobscan::{BlobscanAPI, Options as BlobscanAPIOptions},
};

use super::env::{get_env_vars, Environment};

#[derive(Debug)]
pub struct Context {
    pub beacon_api: BeaconChainAPI,
    pub blobscan_api: BlobscanAPI,
    pub provider: Provider<Http>,
}

pub async fn create_context<'a>() -> Result<Context> {
    let Environment {
        blobscan_api_endpoint,
        beacon_node_rpc,
        execution_node_rpc,
        ..
    } = get_env_vars();

    Ok(Context {
        blobscan_api: BlobscanAPI::try_from(
            blobscan_api_endpoint,
            Some(BlobscanAPIOptions { timeout: Some(8) }),
        )?,
        beacon_api: BeaconChainAPI::try_from(
            beacon_node_rpc,
            Some(BeaconChainAPIOptions { timeout: Some(8) }),
        )?,
        provider: Provider::<Http>::try_from(execution_node_rpc)?,
    })
}
