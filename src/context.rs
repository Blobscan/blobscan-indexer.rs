use std::{fmt::Debug, sync::Arc, time::Duration};

use anyhow::Result as AnyhowResult;
use backoff::ExponentialBackoffBuilder;
use dyn_clone::DynClone;
use ethers::providers::{Http as HttpProvider, MockProvider, Provider};

use crate::{
    clients::{
        beacon::{BeaconClient, CommonBeaconClient, Config as BeaconClientConfig},
        blobscan::{BlobscanClient, CommonBlobscanClient, Config as BlobscanClientConfig},
    },
    env::Environment,
};

#[cfg(test)]
use crate::clients::{beacon::MockCommonBeaconClient, blobscan::MockCommonBlobscanClient};

pub trait CommonContext<T>: Send + Sync + Debug + DynClone {
    fn beacon_client(&self) -> &Box<dyn CommonBeaconClient>;
    fn blobscan_client(&self) -> &Box<dyn CommonBlobscanClient>;
    fn provider(&self) -> &Provider<T>;
}

dyn_clone::clone_trait_object!(CommonContext<HttpProvider>);
dyn_clone::clone_trait_object!(CommonContext<MockProvider>);

pub struct Config {
    pub blobscan_api_endpoint: String,
    pub beacon_node_url: String,
    pub execution_node_endpoint: String,
    pub secret_key: String,
}

#[derive(Debug)]
struct ContextRef<T> {
    pub beacon_client: Box<dyn CommonBeaconClient>,
    pub blobscan_client: Box<dyn CommonBlobscanClient>,
    pub provider: Provider<T>,
}

#[derive(Debug, Clone)]
pub struct Context<T> {
    inner: Arc<ContextRef<T>>,
}

impl Context<HttpProvider> {
    pub fn try_new(config: Config) -> AnyhowResult<Self> {
        let Config {
            blobscan_api_endpoint,
            beacon_node_url,
            execution_node_endpoint,
            secret_key,
        } = config;
        let exp_backoff = Some(ExponentialBackoffBuilder::default().build());

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .build()?;

        Ok(Self {
            inner: Arc::new(ContextRef {
                blobscan_client: Box::new(BlobscanClient::try_with_client(
                    client.clone(),
                    BlobscanClientConfig {
                        base_url: blobscan_api_endpoint,
                        secret_key,
                        exp_backoff: exp_backoff.clone(),
                    },
                )?),
                beacon_client: Box::new(BeaconClient::try_with_client(
                    client,
                    BeaconClientConfig {
                        base_url: beacon_node_url,
                        exp_backoff,
                    },
                )?),
                provider: Provider::<HttpProvider>::try_from(execution_node_endpoint)?,
            }),
        })
    }
}

impl CommonContext<HttpProvider> for Context<HttpProvider> {
    fn beacon_client(&self) -> &Box<dyn CommonBeaconClient> {
        &self.inner.beacon_client
    }

    fn blobscan_client(&self) -> &Box<dyn CommonBlobscanClient> {
        &self.inner.blobscan_client
    }

    fn provider(&self) -> &Provider<HttpProvider> {
        &self.inner.provider
    }
}

impl From<&Environment> for Config {
    fn from(env: &Environment) -> Self {
        Self {
            blobscan_api_endpoint: env.blobscan_api_endpoint.clone(),
            beacon_node_url: env.beacon_node_endpoint.clone(),
            execution_node_endpoint: env.execution_node_endpoint.clone(),
            secret_key: env.secret_key.clone(),
        }
    }
}

#[cfg(test)]
impl Context<MockProvider> {
    pub fn new(
        beacon_client: Option<MockCommonBeaconClient>,
        blobscan_client: Option<MockCommonBlobscanClient>,
        provider: Option<Provider<MockProvider>>,
    ) -> Box<Self> {
        Box::new(Self {
            inner: Arc::new(ContextRef {
                beacon_client: Box::new(beacon_client.unwrap_or(MockCommonBeaconClient::new())),
                blobscan_client: Box::new(
                    blobscan_client.unwrap_or(MockCommonBlobscanClient::new()),
                ),
                provider: provider.unwrap_or(Provider::mocked().0),
            }),
        })
    }
}

#[cfg(test)]
impl CommonContext<MockProvider> for Context<MockProvider> {
    fn beacon_client(&self) -> &Box<dyn CommonBeaconClient> {
        &self.inner.beacon_client
    }

    fn blobscan_client(&self) -> &Box<dyn CommonBlobscanClient> {
        &self.inner.blobscan_client
    }

    fn provider(&self) -> &Provider<MockProvider> {
        &self.inner.provider
    }
}
