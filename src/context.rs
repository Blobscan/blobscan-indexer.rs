use std::{sync::Arc, time::Duration};

use alloy::{
    network::Ethereum,
    providers::{Provider, ProviderBuilder},
};
use anyhow::Result as AnyhowResult;
use backoff::ExponentialBackoffBuilder;
use dyn_clone::DynClone;

use crate::{
    args::Args,
    clients::{
        beacon::{BeaconClient, CommonBeaconClient, Config as BeaconClientConfig},
        blobscan::{BlobscanClient, CommonBlobscanClient, Config as BlobscanClientConfig},
    },
    env::Environment,
    network::Network,
};

pub struct SyncingSettings {
    pub concurrency: u32,
    pub checkpoint_size: u32,
    pub disable_checkpoints: bool,
}

impl From<&Args> for SyncingSettings {
    fn from(args: &Args) -> Self {
        SyncingSettings {
            concurrency: args.num_threads.resolve(),
            checkpoint_size: args.slots_per_save,
            disable_checkpoints: args.disable_sync_checkpoint_save,
        }
    }
}

// #[cfg(test)]
// use crate::clients::{beacon::MockCommonBeaconClient, blobscan::MockCommonBlobscanClient};

pub trait CommonContext: Send + Sync + DynClone {
    fn beacon_client(&self) -> &dyn CommonBeaconClient;
    fn blobscan_client(&self) -> &dyn CommonBlobscanClient;
    fn network(&self) -> &Network;
    fn provider(&self) -> &dyn Provider<Ethereum>;
    fn syncing_settings(&self) -> &SyncingSettings;
}

dyn_clone::clone_trait_object!(CommonContext);
// dyn_clone::clone_trait_object!(CommonContext<MockProvider>);

struct ContextRef {
    pub network: Network,
    pub beacon_client: Box<dyn CommonBeaconClient>,
    pub blobscan_client: Box<dyn CommonBlobscanClient>,
    pub provider: Box<dyn Provider<Ethereum>>,
    pub syncing_settings: SyncingSettings,
}

#[derive(Clone)]
pub struct Context {
    inner: Arc<ContextRef>,
}

impl Context {
    pub fn try_new(env: &Environment, args: &Args) -> AnyhowResult<Self> {
        let exp_backoff = Some(ExponentialBackoffBuilder::default().build());

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .build()?;
        let provider = ProviderBuilder::new()
            .network::<Ethereum>()
            .connect_http(env.execution_node_endpoint.parse()?);
        let network = Network::from(env);

        Ok(Self {
            inner: Arc::new(ContextRef {
                network,
                syncing_settings: SyncingSettings {
                    concurrency: args.num_threads.resolve(),
                    checkpoint_size: args.slots_per_save,
                    disable_checkpoints: args.disable_sync_checkpoint_save,
                },
                blobscan_client: Box::new(BlobscanClient::try_with_client(
                    client.clone(),
                    BlobscanClientConfig {
                        base_url: env.blobscan_api_endpoint.clone(),
                        secret_key: env.secret_key.clone(),
                        exp_backoff: exp_backoff.clone(),
                    },
                )?),
                beacon_client: Box::new(BeaconClient::try_with_client(
                    client,
                    BeaconClientConfig {
                        base_url: env.beacon_node_endpoint.clone(),
                        exp_backoff,
                    },
                )?),
                // Provider::<HttpProvider>::try_from(execution_node_endpoint)?
                provider: Box::new(provider),
            }),
        })
    }
}

impl CommonContext for Context {
    fn beacon_client(&self) -> &dyn CommonBeaconClient {
        self.inner.beacon_client.as_ref()
    }

    fn blobscan_client(&self) -> &dyn CommonBlobscanClient {
        self.inner.blobscan_client.as_ref()
    }

    fn provider(&self) -> &dyn Provider<Ethereum> {
        self.inner.provider.as_ref()
    }

    fn syncing_settings(&self) -> &SyncingSettings {
        &self.inner.syncing_settings
    }

    fn network(&self) -> &Network {
        &self.inner.network
    }
}

// #[cfg(test)]
// impl Context<MockProvider> {
//     pub fn new(
//         beacon_client: Option<MockCommonBeaconClient>,
//         blobscan_client: Option<MockCommonBlobscanClient>,
//         provider: Option<Provider<MockProvider>>,
//     ) -> Box<Self> {
//         Box::new(Self {
//             inner: Arc::new(ContextRef {
//                 beacon_client: Box::new(beacon_client.unwrap_or(MockCommonBeaconClient::new())),
//                 blobscan_client: Box::new(
//                     blobscan_client.unwrap_or(MockCommonBlobscanClient::new()),
//                 ),
//                 provider: provider.unwrap_or(Provider::mocked().0),
//             }),
//         })
//     }
// }

// #[cfg(test)]
// impl CommonContext<MockProvider> for Context<MockProvider> {
//     fn beacon_client(&self) -> &dyn CommonBeaconClient {
//         self.inner.beacon_client.as_ref()
//     }

//     fn blobscan_client(&self) -> &dyn CommonBlobscanClient {
//         self.inner.blobscan_client.as_ref()
//     }

//     fn provider(&self) -> &Provider<MockProvider> {
//         &self.inner.provider
//     }
// }
