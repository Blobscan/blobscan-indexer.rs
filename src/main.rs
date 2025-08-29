use anyhow::{anyhow, Result as AnyhowResult};
use args::Args;
use clap::Parser;
use env::Environment;
use indexer::Indexer;
use utils::{
    banner::print_banner,
    telemetry::{get_subscriber, init_subscriber},
};

use crate::indexer::IndexerResult;

mod args;
mod clients;
mod context;
mod env;
mod indexer;
mod network;
mod slots_processor;
mod synchronizer;
mod utils;

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

    let mut indexer = Indexer::try_new(&env, &args)?;
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
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}
