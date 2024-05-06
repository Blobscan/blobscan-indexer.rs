use anyhow::{anyhow, Result as AnyhowResult};
use args::Args;
use clap::Parser;
use env::Environment;
use indexer::{Indexer, RunOptions};
use url::Url;
use utils::telemetry::{get_subscriber, init_subscriber};

mod args;
mod clients;
mod context;
mod env;
mod indexer;
mod network;
mod slots_processor;
mod synchronizer;
mod utils;

fn remove_credentials_from_url(url_string: &str) -> Option<String> {
    match Url::parse(url_string) {
        Ok(mut url) => {
            url.set_username("******").unwrap();
            url.set_password(None).unwrap();
            Some(url.into())
        }
        Err(_) => None,
    }
}

pub fn print_banner(args: &Args, env: &Environment) {
    println!("____  _       _                         ");
    println!("| __ )| | ___ | |__  ___  ___ __ _ _ __  ");
    println!("|  _ \\| |/ _ \\| '_ \\/ __|/ __/ _` | '_ \\ ");
    println!("| |_) | | (_) | |_) \\__ \\ (_| (_| | | | |");
    println!("|____/|_|\\___/|_.__/|___/\\___\\__,_|_| |_|\n");
    println!("Blobscan indexer (EIP-4844 blob indexer) - blobscan.com");
    println!("=======================================================");

    println!("Network: {:?}", env.network_name);
    if let Some(dencun_fork_slot) = env.dencun_fork_slot {
        println!("Dencun fork slot: {dencun_fork_slot}");
    } else {
        println!("Dencun fork slot: {}", env.network_name.dencun_fork_slot());
    }

    if let Some(from_slot) = args.from_slot.clone() {
        println!("Custom start slot: {}", from_slot.to_detailed_string());
    }

    if let Some(num_threads) = args.num_threads {
        println!("Number of threads: {}", num_threads);
    } else {
        println!("Number of threads: auto");
    }

    if let Some(slots_per_save) = args.slots_per_save {
        println!("Slots checkpoint size: {}", slots_per_save);
    } else {
        println!("Slots checkpoint size: 1000");
    }

    println!(
        "Disable sync checkpoint saving: {}",
        if args.disable_sync_checkpoint_save {
            "yes"
        } else {
            "no"
        }
    );

    println!(
        "Disable historical sync: {}",
        if args.disable_sync_historical {
            "yes"
        } else {
            "no"
        }
    );

    println!("Blobscan API endpoint: {}", env.blobscan_api_endpoint);
    println!(
        "CL endpoint: {:?}",
        remove_credentials_from_url(env.beacon_node_endpoint.as_str())
    );
    println!(
        "EL endpoint: {:?}",
        remove_credentials_from_url(env.execution_node_endpoint.as_str())
    );

    if let Some(sentry_dsn) = env.sentry_dsn.clone() {
        println!("Sentry DSN: {}", sentry_dsn);
    }

    println!("\n");
}

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
    let run_opts = Some(RunOptions {
        disable_sync_historical: args.disable_sync_historical,
    });

    print_banner(&args, &env);

    Indexer::try_new(&env, &args)?
        .run(args.from_slot, run_opts)
        .await
        .map_err(|err| anyhow!(err))
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}
