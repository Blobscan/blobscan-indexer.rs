use clap::Parser;

/// Blobscan's indexer for the EIP-4844 upgrade.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Slot to start indexing from
    #[arg(short, long)]
    pub from_slot: Option<u32>,

    /// Number of threads used for parallel indexing
    #[arg(short, long)]
    pub num_threads: Option<u32>,
}
