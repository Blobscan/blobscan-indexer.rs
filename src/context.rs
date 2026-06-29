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
    /// Number of slots per epoch, as reported by the consensus client's spec.
    fn slots_per_epoch(&self) -> u32;
}

dyn_clone::clone_trait_object!(CommonContext);
// dyn_clone::clone_trait_object!(CommonContext<MockProvider>);

struct ContextRef {
    pub network: Network,
    pub slots_per_epoch: u32,
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
        let provider: Box<dyn Provider<Ethereum>> = Box::new(
            ProviderBuilder::new()
                .network::<Ethereum>()
                .connect_http(config.execution_node_base_url.parse()?),
        );
        let beacon_client: Box<dyn CommonBeaconClient> = Box::new(BeaconClient::try_with_client(
            client.clone(),
            BeaconClientConfig {
                base_url: config.beacon_api_base_url.clone(),
                exp_backoff: exp_backoff.clone(),
            },
        )?);
        let blobscan_client: Box<dyn CommonBlobscanClient> =
            Box::new(BlobscanClient::try_with_client(
                client,
                BlobscanClientConfig {
                    base_url: config.blobscan_api_base_url.clone(),
                    secret_key: config.blobscan_secret_key.clone(),
                    exp_backoff,
                },
            )?);

        let slots_per_epoch = Self::validate_clients_consistency(
            provider.as_ref(),
            beacon_client.as_ref(),
            &config.network,
        )
        .await?;

        Ok(Self {
            inner: Arc::new(ContextRef {
                network: config.network,
                slots_per_epoch,
                syncing_settings: config.syncing_settings,
                blobscan_client,
                beacon_client,
                provider,
            }),
        })
    }

    /// Cross-checks the execution and consensus clients against the configured network
    /// and returns the consensus `SLOTS_PER_EPOCH` value.
    async fn validate_clients_consistency(
        provider: &dyn Provider<Ethereum>,
        beacon_client: &dyn CommonBeaconClient,
        network: &Network,
    ) -> AnyhowResult<u32> {
        let execution_chain_id = provider.get_chain_id().await?;
        let consensus_spec = beacon_client.get_spec().await?;

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

                if spec.slots_per_epoch == 0 {
                    bail!("Consensus spec reported SLOTS_PER_EPOCH = 0");
                }

                Ok(spec.slots_per_epoch as u32)
            }
            None => Err(anyhow!("No consensus spec found")),
        }
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

    fn slots_per_epoch(&self) -> u32 {
        self.inner.slots_per_epoch
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
