use anyhow::{anyhow, Context as AnyhowContext, Result as AnyhowResult};
use clap::Parser;
use tracing::error;

use blob_indexer::{
    context::{Context, ContextConfig, SyncingSettings},
    indexer::{Indexer, IndexerResult},
    network::{Network, NetworkName},
    utils::telemetry::{get_subscriber, init_subscriber},
};

use crate::{args::Args, banner::print_banner, env::Environment};

mod args;
mod banner;
mod env;

async fn run() -> AnyhowResult<()> {
    dotenv::dotenv().ok();
    let env = match Environment::from_env() {
        Ok(env) => env,
        Err(err) => return Err(anyhow!(format!("Failed to load env variables: {}", err))),
    };

    let mut _guard;

    if let Some(sentry_dsn) = env.sentry_dsn.clone() {
        _guard = sentry::init((
            sentry_dsn,
            sentry::ClientOptions {
                release: sentry::release_name!(),
                ..Default::default()
            },
        ));
    }

    let subscriber = get_subscriber("info".into(), std::io::stdout);
    init_subscriber(subscriber);

    let args = Args::parse();

    print_banner(&args, &env);

    let network = match env.network_name {
        NetworkName::Preset(name) => Network::new(name),
        NetworkName::Devnet => Network::new_devnet(0, env.dencun_fork_slot.unwrap_or(0), 0),
    };
    let syncing_settings = SyncingSettings {
        checkpoint_size: args.slots_per_save,
        concurrency: args.num_threads.resolve(),
        disable_checkpoints: args.disable_sync_checkpoint_save,
    };
    let config = ContextConfig {
        beacon_api_base_url: env.beacon_node_endpoint,
        blobscan_api_base_url: env.blobscan_api_endpoint,
        blobscan_secret_key: env.secret_key,
        execution_node_base_url: env.execution_node_endpoint,
        network,
        syncing_settings,
    };
    let context = Context::try_new(config)
        .await
        .with_context(|| "Failed to create context")?;
    let mut indexer = Indexer::new(context, args.disable_sync_historical);
    let res: IndexerResult<()>;

    if let Some(from_slot) = args.from_slot {
        if let Some(to_slot) = args.to_slot {
            res = indexer.index_block_range(from_slot, to_slot).await;
        } else {
            res = indexer.index_from(from_slot).await;
        }
    } else {
        res = indexer.index().await;
    }

    res.map_err(|err| err.into())
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        error!("Error: {err:?}");

        std::process::exit(1);
    }
}
