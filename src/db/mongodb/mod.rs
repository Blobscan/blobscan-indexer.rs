use async_trait::async_trait;
use ethers::types::{Block, Transaction, TxHash, H256};
use mongodb::{
    bson::doc,
    error::UNKNOWN_TRANSACTION_COMMIT_RESULT,
    options::{ClientOptions, UpdateOptions},
    Client, ClientSession, Database,
};
use std::error::Error;

use crate::utils::web3::get_tx_versioned_hashes;

use self::types::{BlobDocument, BlockDocument, IndexerMetadataDocument, TransactionDocument};

use super::{
    blob_db_manager::DBManager,
    types::{Blob, IndexerMetadata},
    utils::{build_blob_id, build_block_id, build_tx_id},
};

mod types;

pub struct MongoDBManager {
    pub session: ClientSession,
    pub db: Database,
}

pub struct MongoDBManagerOptions {
    pub use_session: bool,
}

const INDEXER_METADATA_ID: &str = "indexer_metadata";

pub async fn connect() -> Result<MongoDBManager, Box<dyn Error>> {
    let connection_url = std::env::var("MONGODB_URI").unwrap();
    let database_name = std::env::var("MONGODB_DB").unwrap();

    let mut client_options = ClientOptions::parse(connection_url).await?;

    client_options.app_name = Some("Blobscan".to_string());

    let client = Client::with_options(client_options)?;
    let session = client.start_session(None).await?;
    let db = client.database(&database_name);

    Ok(MongoDBManager { session, db })
}

#[async_trait]
impl DBManager for MongoDBManager {
    type Options = MongoDBManagerOptions;

    async fn commit_transaction(
        &mut self,
        _options: Option<Self::Options>,
    ) -> Result<(), Box<dyn Error>> {
        // An "UnknownTransactionCommitResult" label indicates that it is unknown whether the
        // commit has satisfied the write concern associated with the transaction. If an error
        // with this label is returned, it is safe to retry the commit until the write concern is
        // satisfied or an error without the label is returned.
        loop {
            let result = self.session.commit_transaction().await;

            if let Err(ref error) = result {
                println!("Commit result: {:?}", error);
                if error.contains_label(UNKNOWN_TRANSACTION_COMMIT_RESULT) {
                    continue;
                }
            }

            break;
        }

        Ok(())
    }

    async fn insert_block(
        &mut self,
        execution_block: &Block<TxHash>,
        block_blob_txs: &Vec<Transaction>,
        slot: u32,
        _options: Option<Self::Options>,
    ) -> Result<(), Box<dyn Error>> {
        let tx_hashes = block_blob_txs
            .iter()
            .map(|tx| tx.hash.clone())
            .collect::<Vec<H256>>();
        let block_document = BlockDocument {
            _id: build_block_id(&execution_block.hash.unwrap()),
            hash: execution_block.hash.unwrap(),
            parent_hash: execution_block.parent_hash,
            number: execution_block.number.unwrap().as_u64(),
            slot: slot,
            timestamp: execution_block.timestamp,
            transactions: tx_hashes,
        };

        let blocks_collection = self.db.collection::<BlockDocument>("blocks");

        blocks_collection
            .insert_one_with_session(block_document, None, &mut self.session)
            .await?;

        Ok(())
    }

    async fn insert_blob(
        &mut self,
        blob: &Blob,
        tx_hash: H256,
        _options: Option<Self::Options>,
    ) -> Result<(), Box<dyn Error>> {
        let blob_document = &BlobDocument {
            _id: build_blob_id(&tx_hash, blob.index),
            data: blob.data.to_string(),
            commitment: blob.commitment.clone(),
            index: blob.index,
            hash: blob.versioned_hash,
            tx_hash: tx_hash,
        };

        let blobs_collection = self.db.collection::<BlobDocument>("blobs");

        blobs_collection
            .insert_one_with_session(blob_document, None, &mut self.session)
            .await?;

        Ok(())
    }

    async fn insert_tx(
        &mut self,
        tx: &Transaction,
        index: u32,
        _options: Option<Self::Options>,
    ) -> Result<(), Box<dyn Error>> {
        let blob_versioned_hashes = get_tx_versioned_hashes(&tx);

        let tx_document = TransactionDocument {
            _id: build_tx_id(&tx.hash),
            hash: tx.hash,
            from: tx.from,
            to: tx.to.unwrap(),
            value: tx.value,
            block_hash: tx.block_hash.unwrap(),
            block_number: tx.block_number.unwrap().as_u64(),
            block_versioned_hashes: blob_versioned_hashes,
            index: index,
        };

        let txs_collection = self.db.collection::<TransactionDocument>("txs");

        txs_collection
            .insert_one_with_session(tx_document, None, &mut self.session)
            .await?;

        Ok(())
    }

    async fn start_transaction(&mut self) -> Result<(), Box<dyn Error>> {
        self.session.start_transaction(None).await?;

        Ok(())
    }

    async fn update_last_slot(
        &mut self,
        _slot: u32,
        options: Option<Self::Options>,
    ) -> Result<(), Box<dyn Error>> {
        let use_session = match options {
            Some(options) => options.use_session,
            None => true,
        };
        let indexer_metadata_collection = self
            .db
            .collection::<IndexerMetadataDocument>("indexer_metadata");
        let query = doc! { "_id": INDEXER_METADATA_ID};
        let update = doc! { "$set": { "last_slot": _slot }};
        let mut options = UpdateOptions::default();
        options.upsert = Some(true);

        if use_session {
            indexer_metadata_collection
                .update_one_with_session(query, update, options, &mut self.session)
                .await?;
        } else {
            indexer_metadata_collection
                .update_one(query, update, options)
                .await?;
        }

        Ok(())
    }

    async fn read_metadata(
        &mut self,
        _options: Option<Self::Options>,
    ) -> Result<Option<IndexerMetadata>, Box<dyn Error>> {
        let query = doc! { "_id": INDEXER_METADATA_ID};
        let indexer_metadata_collection = self
            .db
            .collection::<IndexerMetadataDocument>("indexer_metadata");

        match indexer_metadata_collection.find_one(query, None).await? {
            Some(indexer_metadata) => Ok(Some(IndexerMetadata::from(indexer_metadata))),
            None => Ok(None),
        }
    }
}
