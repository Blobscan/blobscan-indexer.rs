use backoff::ExponentialBackoff;
use reqwest::{Client, Url};

use crate::{clients::common::ClientResult, json_get, json_put};

use self::{
    jwt_manager::{Config as JWTManagerConfig, JWTManager},
    types::{Blob, Block, IndexRequest, SlotRequest, SlotResponse, Transaction},
};

mod jwt_manager;

pub mod types;
#[derive(Debug, Clone)]
pub struct BlobscanClient {
    base_url: Url,
    client: reqwest::Client,
    jwt_manager: JWTManager,
    exp_backoff: Option<ExponentialBackoff>,
}

pub struct Config {
    pub base_url: String,
    pub secret_key: String,
    pub exp_backoff: Option<ExponentialBackoff>,
}

impl BlobscanClient {
    pub fn try_with_client(client: Client, config: Config) -> ClientResult<Self> {
        let base_url = Url::parse(&format!("{}/api/indexer/", config.base_url))?;
        let jwt_manager = JWTManager::new(JWTManagerConfig {
            secret_key: config.secret_key,
            refresh_interval: chrono::Duration::hours(1),
            safety_magin: None,
        });
        let exp_backoff = config.exp_backoff;

        Ok(Self {
            base_url,
            client,
            jwt_manager,
            exp_backoff,
        })
    }

    pub async fn index(
        &self,
        block: Block,
        transactions: Vec<Transaction>,
        blobs: Vec<Blob>,
    ) -> ClientResult<()> {
        let url = self.base_url.join("block-txs-blobs")?;
        let token = self.jwt_manager.get_token()?;
        let req = IndexRequest {
            block,
            transactions,
            blobs,
        };

        json_put!(&self.client, url, token, &req).map(|_: Option<()>| ())
    }

    pub async fn update_slot(&self, slot: u32) -> ClientResult<()> {
        let token = self.jwt_manager.get_token()?;
        let req = SlotRequest { slot };
        let url = self.base_url.join("slot")?;

        json_put!(&self.client, url, token, &req).map(|_: Option<()>| ())
    }

    pub async fn get_slot(&self) -> ClientResult<Option<u32>> {
        let url = self.base_url.join("slot")?;

        json_get!(&self.client, url, SlotResponse, self.exp_backoff.clone())
            .map(|res: Option<SlotResponse>| Some(res.unwrap().slot))
    }
}
