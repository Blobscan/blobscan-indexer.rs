use anyhow::{anyhow, Result};
use args::Args;
use clap::Parser;
use env::Environment;
use indexer::Indexer;
use utils::telemetry::{get_subscriber, init_subscriber};

mod args;
mod clients;
mod context;
mod env;
mod indexer;
mod slots_processor;
mod synchronizer;
mod utils;

pub fn print_banner(args: &Args, env: &Environment) {
    println!("____  _       _                         ");
    println!("| __ )| | ___ | |__  ___  ___ __ _ _ __  ");
    println!("|  _ \\| |/ _ \\| '_ \\/ __|/ __/ _` | '_ \\ ");
    println!("| |_) | | (_) | |_) \\__ \\ (_| (_| | | | |");
    println!("|____/|_|\\___/|_.__/|___/\\___\\__,_|_| |_|\n");
    println!("Blobscan indexer (EIP-4844 blob indexer) - blobscan.com");
    println!("=======================================================");

    if let Some(num_threads) = args.num_threads {
        println!("Number of threads: {}", num_threads);
    } else {
        println!("Number of threads: auto");
    }

    if let Some(slots_per_save) = args.slots_per_save {
        println!("Slot chunk size: {}", slots_per_save);
    } else {
        println!("Slot chunk size: auto");
    }

    println!("Blobscan API endpoint: {}", env.blobscan_api_endpoint);
    println!("CL endpoint: {}", env.beacon_node_endpoint);
    println!("EL endpoint: {}", env.execution_node_endpoint);

    if let Some(sentry_dsn) = env.sentry_dsn.clone() {
        println!("Sentry DSN: {}", sentry_dsn);
    }

    println!("\n");
}

async fn run() -> Result<()> {
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

    let subscriber = get_subscriber("blobscan_indexer".into(), "info".into(), std::io::stdout);
    init_subscriber(subscriber);

    let args = Args::parse();

    print_banner(&args, &env);

    let mut indexer = Indexer::try_new(&env, &args)?;

    indexer.run(args.from_slot).await
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}
