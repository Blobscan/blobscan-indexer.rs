use alloy::{
    consensus::Transaction as ConsensusTx,
    primitives::B256,
    rpc::types::{BlockTransactions, Transaction},
};

pub trait B256Ext {
    fn to_full_hex(&self) -> String;
}

impl B256Ext for B256 {
    fn to_full_hex(&self) -> String {
        format!("0x{:x}", self)
    }
}

pub trait BlobTransactionExt {
    /// Returns all shard blob transactions.
    fn filter_blob_transactions(&self) -> Vec<&Transaction>;
}

impl BlobTransactionExt for BlockTransactions<Transaction> {
    fn filter_blob_transactions(&self) -> Vec<&Transaction> {
        match self.as_transactions() {
            Some(txs) => txs
                .into_iter()
                .filter(|tx| tx.inner.blob_versioned_hashes().is_some())
                .collect(),
            None => vec![],
        }
    }
}
