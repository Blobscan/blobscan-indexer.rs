use async_trait::async_trait;
use mongodb::{
    bson::doc,
    error::UNKNOWN_TRANSACTION_COMMIT_RESULT,
    options::{ClientOptions, UpdateOptions},
    Client, ClientSession, Database,
};

use crate::types::{Blob, BlockData, IndexerMetadata, StdError, TransactionData};

use self::types::{BlobDocument, BlockDocument, IndexerMetadataDocument, TransactionDocument};

use super::blob_db_manager::DBManager;

mod types;

#[derive(Debug)]
pub struct MongoDBManager {
    pub session: ClientSession,
    pub db: Database,
}

pub struct MongoDBManagerOptions {
    pub use_session: bool,
}

const INDEXER_METADATA_ID: &str = "indexer_metadata";

#[async_trait]
impl DBManager for MongoDBManager {
    type Options = MongoDBManagerOptions;

    async fn new(connection_uri: &String, db_name: &String) -> Result<Self, StdError>
    where
        Self: Sized,
    {
        let mut client_options = ClientOptions::parse(connection_uri).await?;

        client_options.app_name = Some("Blobscan".to_string());

        let client = Client::with_options(client_options)?;
        let session = client.start_session(None).await?;
        let db = client.database(db_name);

        Ok(MongoDBManager { session, db })
    }

    async fn commit_transaction(
        &mut self,
        _options: Option<Self::Options>,
    ) -> Result<(), StdError> {
        // An "UnknownTransactionCommitResult" label indicates that it is unknown whether the
        // commit has satisfied the write concern associated with the transaction. If an error
        // with this label is returned, it is safe to retry the commit until the write concern is
        // satisfied or an error without the label is returned.
        loop {
            let result = self.session.commit_transaction().await;

            if let Err(ref error) = result {
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
        block_data: &BlockData,
        _options: Option<Self::Options>,
    ) -> Result<(), StdError> {
        let block_document = BlockDocument::try_from(block_data)?;

        let blocks_collection = self.db.collection::<BlockDocument>("blocks");

        blocks_collection
            .insert_one_with_session(block_document, None, &mut self.session)
            .await?;

        Ok(())
    }

    async fn insert_blob(
        &mut self,
        blob: &Blob,
        _options: Option<Self::Options>,
    ) -> Result<(), StdError> {
        let blob_document = BlobDocument::try_from(blob)?;

        let blobs_collection = self.db.collection::<BlobDocument>("blobs");

        blobs_collection
            .insert_one_with_session(blob_document, None, &mut self.session)
            .await?;

        Ok(())
    }

    async fn insert_tx(
        &mut self,
        tx: &TransactionData,
        _options: Option<Self::Options>,
    ) -> Result<(), StdError> {
        let tx_document = TransactionDocument::try_from(tx)?;

        let txs_collection = self.db.collection::<TransactionDocument>("txs");

        txs_collection
            .insert_one_with_session(tx_document, None, &mut self.session)
            .await?;

        Ok(())
    }

    async fn start_transaction(&mut self) -> Result<(), StdError> {
        self.session.start_transaction(None).await?;

        Ok(())
    }

    async fn update_last_slot(
        &mut self,
        _slot: u32,
        options: Option<Self::Options>,
    ) -> Result<(), StdError> {
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
    ) -> Result<Option<IndexerMetadata>, StdError> {
        let query = doc! { "_id": INDEXER_METADATA_ID};
        let indexer_metadata_collection = self
            .db
            .collection::<IndexerMetadataDocument>("indexer_metadata");

        match indexer_metadata_collection.find_one(query, None).await? {
            Some(indexer_metadata) => Ok(Some(IndexerMetadata::try_from(indexer_metadata)?)),
            None => Ok(None),
        }
    }
}
