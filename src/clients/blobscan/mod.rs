use std::fmt::Debug;

use alloy::primitives::B256;
use async_trait::async_trait;
use backoff::ExponentialBackoff;
use chrono::TimeDelta;
use reqwest::{Client, Url};

#[cfg(test)]
use mockall::automock;
use types::{BlobscanBlock, ReorgedBlocksRequestBody};

use crate::{clients::common::ClientResult, json_get, json_put};

use self::{
    jwt_manager::{Config as JWTManagerConfig, JWTManager},
    types::{
        Blob, Block, BlockchainSyncState, BlockchainSyncStateRequest, BlockchainSyncStateResponse,
        IndexRequest, Transaction,
    },
};

mod jwt_manager;

pub mod types;

#[async_trait]
#[cfg_attr(test, automock)]
pub trait CommonBlobscanClient: Send + Sync + Debug {
    fn try_with_client(client: Client, config: Config) -> ClientResult<Self>
    where
        Self: Sized;
    async fn index(
        &self,
        block: Block,
        transactions: Vec<Transaction>,
        blobs: Vec<Blob>,
    ) -> ClientResult<()>;
    async fn get_block(&self, slot: u32) -> ClientResult<Option<BlobscanBlock>>;
    async fn handle_reorg(
        &self,
        rewinded_blocks: Vec<B256>,
        forwarded_blocks: Vec<B256>,
    ) -> ClientResult<()>;
    async fn update_sync_state(&self, sync_state: BlockchainSyncState) -> ClientResult<()>;
    async fn get_sync_state(&self) -> ClientResult<Option<BlockchainSyncState>>;
}

#[derive(Debug, Clone)]
pub struct BlobscanClient {
    base_url: Url,
    client: reqwest::Client,
    jwt_manager: JWTManager,
    exp_backoff: ExponentialBackoff,
}

pub struct Config {
    pub base_url: String,
    pub secret_key: String,
    pub exp_backoff: ExponentialBackoff,
}

#[async_trait]

impl CommonBlobscanClient for BlobscanClient {
    fn try_with_client(client: Client, config: Config) -> ClientResult<Self> {
        let base_url = Url::parse(&format!("{}/", config.base_url))?;
        let jwt_manager = JWTManager::new(JWTManagerConfig {
            secret_key: config.secret_key,
            refresh_interval: TimeDelta::try_hours(1).unwrap(),
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

    async fn index(
        &self,
        block: Block,
        transactions: Vec<Transaction>,
        blobs: Vec<Blob>,
    ) -> ClientResult<()> {
        let url = self.base_url.join("indexer/block-txs-blobs")?;
        let token = self.jwt_manager.get_token()?;
        let req = IndexRequest {
            block,
            transactions,
            blobs,
        };

        json_put!(&self.client, url, token, &req, self.exp_backoff.clone()).map(|_: Option<()>| ())
    }

    async fn get_block(&self, slot: u32) -> ClientResult<Option<BlobscanBlock>> {
        let url = self.base_url.join(&format!("slots/{}", slot))?;

        json_get!(&self.client, url, BlobscanBlock, self.exp_backoff.clone())
    }

    async fn handle_reorg(
        &self,
        rewinded_blocks: Vec<B256>,
        forwarded_blocks: Vec<B256>,
    ) -> ClientResult<()> {
        let url = self.base_url.join("indexer/reorged-blocks")?;
        let token = self.jwt_manager.get_token()?;

        let req = ReorgedBlocksRequestBody {
            forwarded_blocks,
            rewinded_blocks,
        };

        json_put!(&self.client, url, ReorgedBlocksRequestBody, token, &req, self.exp_backoff.clone()).map(|_| ())
    }

    async fn update_sync_state(&self, sync_state: BlockchainSyncState) -> ClientResult<()> {
        let url = self.base_url.join("blockchain-sync-state")?;
        let token = self.jwt_manager.get_token()?;
        let req: BlockchainSyncStateRequest = sync_state.into();

        json_put!(&self.client, url, token, &req, self.exp_backoff.clone()).map(|_: Option<()>| ())
    }

    async fn get_sync_state(&self) -> ClientResult<Option<BlockchainSyncState>> {
        let url = self.base_url.join("blockchain-sync-state")?;
        json_get!(
            &self.client,
            url,
            BlockchainSyncStateResponse,
            self.exp_backoff.clone()
        )
        .map(|res: Option<BlockchainSyncStateResponse>| Some(res.unwrap().into()))
    }
}
