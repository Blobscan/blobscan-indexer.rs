use std::fmt;

use serde::{Deserialize, Serialize};

use crate::env::Environment;

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EVMNetworkName {
    Mainnet,
    Goerli,
    Sepolia,
    Holesky,
    Hoodi,
    Gnosis,
    Chiado,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkName {
    Preset(EVMNetworkName),
    Devnet,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct Network {
    pub name: NetworkName,
    pub chain_id: u32,
    pub dencun_fork_slot: u32,
    pub epoch: u32,
}

impl Network {
    /// Construct a `Network` from a known network with baked-in params.
    pub fn new(name: EVMNetworkName) -> Self {
        match name {
            EVMNetworkName::Mainnet => Network {
                name: name.into(),
                chain_id: 1,
                dencun_fork_slot: 8_626_176, // epoch 269_568
                epoch: 269_568,
            },
            EVMNetworkName::Goerli => Network {
                name: name.into(),
                chain_id: 5,
                dencun_fork_slot: 7_413_760, // epoch 231_680
                epoch: 231_680,
            },
            EVMNetworkName::Sepolia => Network {
                name: name.into(),
                chain_id: 11155111,
                dencun_fork_slot: 4_243_456, // epoch 132_608
                epoch: 132_608,
            },
            EVMNetworkName::Holesky => Network {
                name: name.into(),
                chain_id: 17000,
                dencun_fork_slot: 950_272, // epoch 29_696
                epoch: 29_696,
            },
            EVMNetworkName::Hoodi => Network {
                name: name.into(),
                chain_id: 560_048,
                dencun_fork_slot: 0,
                epoch: 0,
            },
            EVMNetworkName::Gnosis => Network {
                name: name.into(),
                chain_id: 100,
                dencun_fork_slot: 14_237_696, // epoch 889_856
                epoch: 889_856,
            },
            EVMNetworkName::Chiado => Network {
                name: name.into(),
                chain_id: 10_200,
                dencun_fork_slot: 8_265_728, // epoch 516_608
                epoch: 516_608,
            },
        }
    }

    /// Construct a custom devnet with your own parameters.
    pub fn new_devnet(chain_id: u32, dencun_fork_slot: u32, epoch: u32) -> Self {
        Network {
            name: NetworkName::Devnet,
            chain_id,
            dencun_fork_slot,
            epoch,
        }
    }
}

impl From<EVMNetworkName> for NetworkName {
    fn from(value: EVMNetworkName) -> Self {
        NetworkName::Preset(value)
    }
}

impl fmt::Display for EVMNetworkName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            EVMNetworkName::Mainnet => "mainnet",
            EVMNetworkName::Goerli => "goerli",
            EVMNetworkName::Sepolia => "sepolia",
            EVMNetworkName::Holesky => "holesky",
            EVMNetworkName::Hoodi => "hoodi",
            EVMNetworkName::Gnosis => "gnosis",
            EVMNetworkName::Chiado => "chiado",
        };
        write!(f, "{}", s)
    }
}

impl fmt::Display for NetworkName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetworkName::Preset(net) => net.fmt(f),
            NetworkName::Devnet => write!(f, "devnet"),
        }
    }
}

impl From<&Environment> for Network {
    fn from(env: &Environment) -> Self {
        match env.network_name {
            NetworkName::Preset(name) => Network::new(name),
            NetworkName::Devnet => Network::new_devnet(0, env.dencun_fork_slot.unwrap_or(0), 0),
        }
    }
}
