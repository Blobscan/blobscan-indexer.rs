use std::{sync::Arc, time::Duration};

use alloy::{
    network::Ethereum,
    providers::{Provider, ProviderBuilder},
};
use anyhow::{anyhow, bail, Result as AnyhowResult};
use backoff::ExponentialBackoffBuilder;
use dyn_clone::DynClone;

use crate::{
    clients::{
        beacon::{BeaconClient, CommonBeaconClient, Config as BeaconClientConfig},
        blobscan::{BlobscanClient, CommonBlobscanClient, Config as BlobscanClientConfig},
    },
    network::{Network, NetworkName},
};

pub struct SyncingSettings {
    pub concurrency: u32,
    pub checkpoint_size: u32,
    pub disable_checkpoints: bool,
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

pub struct ContextConfig {
    pub network: Network,
    pub beacon_api_base_url: String,
    pub blobscan_api_base_url: String,
    pub blobscan_secret_key: String,
    pub execution_node_base_url: String,
    pub syncing_settings: SyncingSettings,
}

impl Context {
    pub async fn try_new(config: ContextConfig) -> AnyhowResult<Self> {
        let exp_backoff = Some(ExponentialBackoffBuilder::default().build());
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(16))
            .build()?;
        let provider = ProviderBuilder::new()
            .network::<Ethereum>()
            .connect_http(config.execution_node_base_url.parse()?);

        let ctx = Self {
            inner: Arc::new(ContextRef {
                network: config.network,
                syncing_settings: config.syncing_settings,
                blobscan_client: Box::new(BlobscanClient::try_with_client(
                    client.clone(),
                    BlobscanClientConfig {
                        base_url: config.blobscan_api_base_url.clone(),
                        secret_key: config.blobscan_secret_key.clone(),
                        exp_backoff: exp_backoff.clone(),
                    },
                )?),
                beacon_client: Box::new(BeaconClient::try_with_client(
                    client,
                    BeaconClientConfig {
                        base_url: config.beacon_api_base_url.clone(),
                        exp_backoff,
                    },
                )?),
                // Provider::<HttpProvider>::try_from(execution_node_endpoint)?
                provider: Box::new(provider),
            }),
        };

        ctx.validate_clients_consistency().await?;

        Ok(ctx)
    }

    async fn validate_clients_consistency(&self) -> AnyhowResult<()> {
        let execution_chain_id = self.provider().get_chain_id().await?;
        let consensus_spec = self.beacon_client().get_spec().await?;
        let network = self.network();

        match consensus_spec {
            Some(spec) => {
                let deposit_network_id = spec.deposit_network_id;
                if deposit_network_id != execution_chain_id {
                    bail!(
                        "Execution and Consensus clients mismatch: \n consensus deposit_network_id = {deposit_network_id},  execution chain_id = {execution_chain_id}"
                    );
                }

                if let NetworkName::Preset(p) = network.name {
                    if network.chain_id != execution_chain_id {
                        bail!("Environment network mismatch for '{p}': expected chain_id={}, got {} from execution client", network.chain_id, execution_chain_id);
                    }
                }
            }
            None => {
                return Err(anyhow!("No consensus spec found"));
            }
        };

        Ok(())
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
