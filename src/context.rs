use ethers::prelude::*;
use serde::Deserialize;
use std::error;

use crate::{
    beacon_chain::BeaconChainAPI,
    db::{blob_db_manager::DBManager, mongodb::MongoDBManager},
};

#[derive(Deserialize, Debug)]
struct Environment {
    db_connection_uri: String,
    db_name: String,
    #[serde(default = "default_execution_node_rpc")]
    execution_node_rpc: String,
    #[serde(default = "default_beacon_node_rpc")]
    beacon_node_rpc: String,
    #[serde(default = "default_logger")]
    logger: String,
}

pub struct Context {
    pub beacon_api: BeaconChainAPI,
    pub db_manager: MongoDBManager,
    pub provider: Provider<Http>,
    pub logger: String,
}

fn default_execution_node_rpc() -> String {
    "http://localhost:8545".to_string()
}

fn default_beacon_node_rpc() -> String {
    "http://localhost:3500".to_string()
}

fn default_logger() -> String {
    "default".to_string()
}

pub async fn create_context() -> Result<Context, Box<dyn error::Error>> {
    let Environment {
        beacon_node_rpc,
        db_connection_uri,
        db_name,
        execution_node_rpc,
        logger,
    } = match envy::from_env::<Environment>() {
        Ok(env) => env,
        Err(e) => {
            return Err(format!("Couldn't read environment variables: {}", e).into());
        }
    };

    Ok(Context {
        beacon_api: BeaconChainAPI::new(beacon_node_rpc),
        db_manager: MongoDBManager::new(&db_connection_uri, &db_name).await?,
        provider: Provider::<Http>::try_from(execution_node_rpc)?,
        logger,
    })
}
