use ethers::types::H256;

pub fn build_blob_id(datahash: &H256) -> String {
    format!("{:#x}", datahash)
}

pub fn build_tx_id(tx_hash: &H256) -> String {
    format!("{:#x}", tx_hash)
}

pub fn build_block_id(block_hash: &H256) -> String {
    format!("{:#x}", block_hash)
}
