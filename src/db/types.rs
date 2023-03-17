use ethers::types::{Bytes, H256};

pub struct Blob {
    pub data: Bytes,
    pub commitment: String,
    pub versioned_hash: H256,
    pub index: u32,
}

pub struct IndexerMetadata {
    pub last_slot: u32,
}
