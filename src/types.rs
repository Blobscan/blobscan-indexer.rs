use std::{collections::HashMap, error};

use ethers::types::{Block as EthersBlock, Bytes, Transaction, H256};

use crate::utils::web3::get_tx_versioned_hashes;

pub type StdError = Box<dyn error::Error + Send + Sync>;

#[derive(Debug)]
pub struct BlockData<'a> {
    pub block: &'a EthersBlock<Transaction>,
    pub slot: u32,
    pub tx_to_versioned_hashes: HashMap<H256, Vec<H256>>,
}

#[derive(Debug)]
pub struct TransactionData<'a> {
    pub tx: &'a Transaction,
    pub blob_versioned_hashes: &'a Vec<H256>,
}

#[derive(Debug)]
pub struct Blob<'a> {
    pub data: &'a Bytes,
    pub commitment: String,
    pub versioned_hash: H256,
    pub tx_hash: H256,
}

#[derive(Debug)]
pub struct IndexerMetadata {
    pub last_slot: u32,
}

impl<'a> TryFrom<(&'a EthersBlock<Transaction>, u32)> for BlockData<'a> {
    type Error = StdError;

    fn try_from((block, slot): (&'a EthersBlock<Transaction>, u32)) -> Result<Self, Self::Error> {
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
