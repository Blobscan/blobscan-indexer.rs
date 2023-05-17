use std::time::Duration;

use reqwest::{Client, StatusCode};

use self::{
    jwt_manager::{Config as JWTManagerConfig, JWTManager},
    types::{
        Blob, BlobscanClientError, BlobscanClientResult, Block, IndexRequest, SlotRequest,
        SlotResponse, Transaction,
    },
};

mod jwt_manager;

pub mod types;

#[derive(Debug, Clone)]
pub struct BlobscanClient {
    base_url: String,
    client: reqwest::Client,
    jwt_manager: JWTManager,
}

pub struct Config {
    pub base_url: String,
    pub secret_key: String,
    pub timeout: Option<Duration>,
}

pub fn build_jwt_manager(secret_key: String) -> JWTManager {
    JWTManager::new(JWTManagerConfig {
        secret_key,
        refresh_interval: chrono::Duration::minutes(30),
        safety_magin: None,
    })
}

impl BlobscanClient {
    pub fn with_client(client: Client, config: Config) -> Self {
        Self {
            base_url: config.base_url,
            client,
            jwt_manager: build_jwt_manager(config.secret_key),
        }
    }

    pub async fn index(
        &self,
        block: Block,
        transactions: Vec<Transaction>,
        blobs: Vec<Blob>,
    ) -> BlobscanClientResult<()> {
        let path = String::from("index");
        let url = self.build_url(&path);
        let token = self.jwt_manager.get_token()?;
        let index_request = IndexRequest {
            block,
            transactions,
            blobs,
        };

        let index_response = self
            .client
            .put(url)
            .bearer_auth(token)
            .json(&index_request)
            .send()
            .await?;

        match index_response.status() {
            StatusCode::OK => Ok(()),
            _ => Err(BlobscanClientError::ApiError(index_response.text().await?)),
        }
    }

    pub async fn update_slot(&self, slot: u32) -> BlobscanClientResult<()> {
        let path = String::from("slot");
        let url = self.build_url(&path);
        let token = self.jwt_manager.get_token()?;

        let slot_response = self
            .client
            .post(url)
            .bearer_auth(token)
            .json(&SlotRequest { slot })
            .send()
            .await?;

        match slot_response.status() {
            StatusCode::OK => Ok(()),
            _ => Err(BlobscanClientError::ApiError(slot_response.text().await?)),
        }
    }

    pub async fn get_slot(&self) -> BlobscanClientResult<Option<u32>> {
        let path = String::from("slot");
        let url = self.build_url(&path);
        let token = self.jwt_manager.get_token()?;
        let slot_response = self.client.get(url).bearer_auth(token).send().await?;

        match slot_response.status() {
            StatusCode::OK => Ok(Some(slot_response.json::<SlotResponse>().await?.slot)),
            StatusCode::NOT_FOUND => Ok(None),
            _ => Err(BlobscanClientError::ApiError(slot_response.text().await?)),
        }
    }

    fn build_url(&self, path: &String) -> String {
        format!("{}/api/{}", self.base_url, path)
    }
}
