use envy::Error::MissingValue;
use serde::Deserialize;

use crate::network::{EVMNetworkName, NetworkName};

#[derive(Deserialize, Debug)]
pub struct Environment {
    #[serde(default = "default_network")]
    pub network_name: NetworkName,
    #[serde(default = "default_blobscan_api_endpoint")]
    pub blobscan_api_endpoint: String,
    #[serde(default = "default_beacon_node_endpoint")]
    pub beacon_node_endpoint: String,
    #[serde(default = "default_execution_node_endpoint")]
    pub execution_node_endpoint: String,
    pub secret_key: String,
    pub dencun_fork_slot: Option<u32>,
    pub sentry_dsn: Option<String>,
}

fn default_network() -> NetworkName {
    NetworkName::Preset(EVMNetworkName::Mainnet)
}

fn default_blobscan_api_endpoint() -> String {
    "http://localhost:3001".into()
}

fn default_beacon_node_endpoint() -> String {
    "http://localhost:3500".into()
}

fn default_execution_node_endpoint() -> String {
    "http://localhost:8545".into()
}

impl Environment {
    pub fn from_env() -> Result<Self, envy::Error> {
        match envy::from_env::<Environment>() {
            Ok(config) => {
                if config.beacon_node_endpoint.is_empty() {
                    return Err(MissingValue("BEACON_NODE_ENDPOINT"));
                } else if config.blobscan_api_endpoint.is_empty() {
                    return Err(MissingValue("BLOBSCAN_API_ENDPOINT"));
                } else if config.execution_node_endpoint.is_empty() {
                    return Err(MissingValue("EXECUTION_NODE_ENDPOINT"));
                } else if config.secret_key.is_empty() {
                    return Err(MissingValue("SECRET_KEY"));
                }

                Ok(config)
            }
            Err(err) => Err(err),
        }
    }
}
