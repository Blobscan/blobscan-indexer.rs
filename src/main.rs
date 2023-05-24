use anyhow::Result;
use env::Environment;
use utils::telemetry::{get_subscriber, init_subscriber};

mod args;
mod clients;
mod context;
mod env;
mod indexer;
mod slots_processor;
mod utils;

async fn run() -> Result<()> {
    dotenv::dotenv().ok();
    let env = Environment::from_env()?;

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

    let subscriber = get_subscriber("blobscan_indexer".into(), "info".into(), std::io::stdout);
    init_subscriber(subscriber);

    indexer::run().await
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}
