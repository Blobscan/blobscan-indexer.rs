use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    Mainnet,
    Goerli,
    Sepolia,
    Holesky,
    Devnet,
    Gnosis,
    Chiado,
}

impl Network {
    pub fn dencun_fork_slot(&self) -> u32 {
        match self {
            Network::Mainnet => 8626176, // Epoch 269568
            Network::Goerli => 7413760,  // Epoch 231680
            Network::Sepolia => 4243456, // Epoch 132608
            Network::Holesky => 950272,  // Epoch 29696
            Network::Devnet => 0,
            Network::Gnosis => 14237696,
            Network::Chiado => 8265728, // Epoch 516608
        }
    }
}
