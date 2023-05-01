use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct Environment {
    #[serde(default = "default_blobscan_api_endpoint")]
    pub blobscan_api_endpoint: String,
    #[serde(default = "default_beacon_node_rpc")]
    pub beacon_node_rpc: String,
    #[serde(default = "default_execution_node_rpc")]
    pub execution_node_rpc: String,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_logger")]
    #[allow(dead_code)] // Temporal until we move to tracing
    logger: String,
}

pub const DEV_MODE: &str = "development";

fn default_blobscan_api_endpoint() -> String {
    "http://localhost:3000".to_string()
}

fn default_beacon_node_rpc() -> String {
    "http://localhost:3500".to_string()
}

fn default_execution_node_rpc() -> String {
    "http://localhost:8545".to_string()
}

fn default_mode() -> String {
    DEV_MODE.to_string()
}

fn default_logger() -> String {
    "default".to_string()
}

pub fn get_env_vars() -> Environment {
    match envy::from_env::<Environment>() {
        Ok(env) => env,
        Err(e) => {
            panic!("Couldn't read environment variables: {}", e);
        }
    }
}
