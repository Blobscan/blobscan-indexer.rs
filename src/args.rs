use clap::{ArgAction, Parser};

use crate::clients::beacon::types::BlockId;

/// Blobscan's indexer for the EIP-4844 upgrade.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Slot to start indexing from
    #[arg(short, long)]
    pub from_slot: Option<BlockId>,

    /// Number of threads used for parallel indexing
    #[arg(short, long)]
    pub num_threads: Option<u32>,

    /// Amount of slots to be processed before saving latest slot in the database
    #[arg(short, long)]
    pub slots_per_save: Option<u32>,

    /// Disable slot checkpoint saving when syncing
    #[arg(short = 'c', long, action = ArgAction::SetTrue)]
    pub disable_sync_checkpoint_save: bool,

    /// Disable historical synchronization
    #[arg(short = 'd', long, action = ArgAction::SetTrue)]
    pub disable_sync_historical: bool,
}
