use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    Mainnet,
    Goerli,
    Sepolia,
    Holesky,
    Devnet,
}

impl Network {
    pub fn dencun_fork_slot(&self) -> u32 {
        match self {
            Network::Mainnet => 8626176,
            Network::Goerli => 7413760,
            Network::Sepolia => 4243456,
            Network::Holesky => 950272,
            Network::Devnet => 0,
        }
    }
}
