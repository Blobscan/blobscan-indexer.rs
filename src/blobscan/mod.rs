use reqwest::{Client, StatusCode};
use std::time::Duration;

use crate::types::{BlobEntity, BlockEntity, TransactionEntity};

use self::types::{BlobscanAPIError, BlobscanAPIResult, IndexRequest, SlotRequest, SlotResponse};

mod types;

#[derive(Debug, Clone)]
pub struct BlobscanAPI {
    base_url: String,
    client: reqwest::Client,
}

pub struct Options {
    pub timeout: Option<u64>,
}

impl BlobscanAPI {
    pub fn try_from(base_url: String, options: Option<Options>) -> BlobscanAPIResult<Self> {
        let mut client_builder = Client::builder();

        if let Some(options) = options {
            if let Some(timeout) = options.timeout {
                client_builder = client_builder.timeout(Duration::from_secs(timeout));
            }
        }

        Ok(Self {
            base_url,
            client: client_builder.build()?,
        })
    }

    pub async fn index(
        &self,
        block: BlockEntity,
        transactions: Vec<TransactionEntity>,
        blobs: Vec<BlobEntity>,
    ) -> BlobscanAPIResult<()> {
        let path = String::from("index");
        let url = self.build_url(&path);

        let index_request = IndexRequest {
            block: block,
            transactions,
            blobs,
        };

        let index_response = self.client.post(url).json(&index_request).send().await?;

        match index_response.status() {
            StatusCode::OK => Ok(()),
            _ => Err(BlobscanAPIError::BlobscanClientError(
                index_response.text().await?,
            )),
        }
    }

    pub async fn update_slot(&self, slot: u32) -> BlobscanAPIResult<()> {
        let path = String::from("slot");
        let url = self.build_url(&path);

        let slot_response = self
            .client
            .post(url)
            .json(&SlotRequest { slot })
            .send()
            .await?;

        match slot_response.status() {
            StatusCode::OK => Ok(()),
            _ => Err(BlobscanAPIError::BlobscanClientError(
                slot_response.text().await?,
            )),
        }
    }

    pub async fn get_slot(&self) -> BlobscanAPIResult<Option<u32>> {
        let path = String::from("slot");
        let url = self.build_url(&path);

        let slot_response = self.client.get(url).send().await?;

        match slot_response.status() {
            StatusCode::OK => Ok(Some(slot_response.json::<SlotResponse>().await?.slot)),
            StatusCode::NOT_FOUND => Ok(None),
            _ => Err(BlobscanAPIError::BlobscanClientError(
                slot_response.text().await?,
            )),
        }
    }

    fn build_url(&self, path: &String) -> String {
        format!("{}/api/{}", self.base_url, path)
    }
}
