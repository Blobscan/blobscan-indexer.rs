use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct Environment {
    #[serde(default = "default_blobscan_api_endpoint")]
    pub blobscan_api_endpoint: String,
    #[serde(default = "default_beacon_node_rpc")]
    pub beacon_node_rpc: String,
    #[serde(default = "default_execution_node_rpc")]
    pub execution_node_rpc: String,
    pub num_processing_threads: Option<u32>,
    pub secret_key: String,
}

fn default_blobscan_api_endpoint() -> String {
    "http://localhost:3001".to_string()
}

fn default_beacon_node_rpc() -> String {
    "http://localhost:5052".to_string()
}

fn default_execution_node_rpc() -> String {
    "http://localhost:8545".to_string()
}

impl Environment {
    pub fn from_env() -> Result<Self, envy::Error> {
        envy::from_env::<Environment>()
    }
}
