use url::Url;

use crate::{args::Args, env::Environment};

fn mask_quik_node_url(url_string: &str) -> Option<String> {
    match Url::parse(url_string) {
        Ok(mut url) => {
            // Get the path segments as a vector of strings.
            let mut segments: Vec<&str> = url.path_segments().map_or(vec![], |c| c.collect());

            if !segments.is_empty() {
                *segments.last_mut().unwrap() = "*******";

                url.set_path(&segments.join("/"));
            }

            if let Some(host) = url.host_str() {
                // Split the host by '.' to isolate the subdomain part.
                let parts: Vec<&str> = host.split('.').collect();
                if parts.len() > 2 {
                    // Join the parts back into a string with the subdomain masked.
                    let masked_host = format!(
                        "{}.{}.{}{}{}",
                        "*******",
                        parts[1],
                        parts[2],
                        if parts.len() > 3 { "." } else { "" },
                        parts[3..].join(".")
                    );
                    // Set the new host.
                    url.set_host(Some(&masked_host)).ok()?;
                }
            }

            // Return the modified URL as a string.
            Some(url.to_string())
        }
        // Return None if the URL parsing fails.
        Err(_) => None,
    }
}
pub fn remove_credentials_from_url(url_string: &str) -> Option<String> {
    match Url::parse(url_string) {
        Ok(mut url) => {
            if let Some(host) = url.host_str() {
                if host.contains("quiknode.pro") {
                    return mask_quik_node_url(url_string);
                }
            }
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

    if let Some(to_slot) = args.to_slot.clone() {
        println!("Custom end slot: {}", to_slot.to_detailed_string());
    }

    println!("Number of threads: {}", args.num_threads);

    println!("Slots checkpoint size: {}", args.slots_per_save);

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
