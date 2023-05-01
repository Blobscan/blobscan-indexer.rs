use anyhow::{Error, Result};
use ethers::types::{
    Address, Block as EthersBlock, Bytes, Transaction as EthersTransaction, H256, U256, U64,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::utils::web3::get_tx_versioned_hashes;

#[derive(Serialize, Deserialize, Debug)]
pub struct BlockEntity {
    pub number: U64,
    pub hash: H256,
    pub timestamp: U256,
    pub slot: u32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TransactionEntity {
    pub hash: H256,
    pub from: Address,
    pub to: Address,
    pub block_number: U64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BlobEntity {
    pub versioned_hash: H256,
    pub commitment: String,
    pub data: Bytes,
    pub tx_hash: H256,
    pub index: u32,
}

#[derive(Debug)]
pub struct BlockData<'a> {
    pub block: &'a EthersBlock<EthersTransaction>,
    pub slot: u32,
    pub tx_to_versioned_hashes: HashMap<H256, Vec<H256>>,
}

#[derive(Debug)]
pub struct TransactionData<'a> {
    pub tx: &'a EthersTransaction,
    pub blob_versioned_hashes: &'a Vec<H256>,
}

#[derive(Debug)]
pub struct BlobData<'a> {
    pub data: &'a Bytes,
    pub commitment: String,
    pub versioned_hash: H256,
    pub tx_hash: H256,
}

impl<'a> TryFrom<(&'a EthersBlock<EthersTransaction>, u32)> for BlockData<'a> {
    type Error = Error;

    fn try_from(
        (block, slot): (&'a EthersBlock<EthersTransaction>, u32),
    ) -> Result<Self, Self::Error> {
        let mut tx_to_versioned_hashes = HashMap::new();

        for tx in &block.transactions {
            match get_tx_versioned_hashes(tx)? {
                Some(versioned_hashes) => {
                    tx_to_versioned_hashes.insert(tx.hash, versioned_hashes);
                }
                None => continue,
            }
        }

        Ok(Self {
            block,
            slot,
            tx_to_versioned_hashes,
        })
    }
}
