use anyhow::Result;
use ethers::prelude::*;

use crate::{
    beacon_chain::{BeaconChainAPI, Options as BeaconChainAPIOptions},
    db::{blob_db_manager::DBManager, mongodb::MongoDBManager},
};

use super::env::{get_env_vars, Environment};

#[derive(Debug)]
pub struct Context {
    pub beacon_api: BeaconChainAPI,
    pub db_manager: MongoDBManager,
    pub provider: Provider<Http>,
}

pub async fn create_context<'a>() -> Result<Context> {
    let Environment {
        beacon_node_rpc,
        db_connection_uri,
        db_name,
        execution_node_rpc,
        ..
    } = get_env_vars();

    Ok(Context {
        beacon_api: BeaconChainAPI::try_from(
            beacon_node_rpc,
            Some(BeaconChainAPIOptions { timeout: Some(8) }),
        )?,
        db_manager: MongoDBManager::new(&db_connection_uri, &db_name).await?,
        provider: Provider::<Http>::try_from(execution_node_rpc)?,
    })
}
