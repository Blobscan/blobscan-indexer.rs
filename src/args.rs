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

    /// Amount of slots to be processed before saving latest slot in the database
    #[arg(short, long, default_value_t = 1000)]
    pub slots_per_save: u32,
}
