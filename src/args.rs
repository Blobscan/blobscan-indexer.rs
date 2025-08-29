use clap::{ArgAction, Parser};

use std::str::FromStr;

use crate::clients::beacon::types::BlockId;

#[derive(Debug, Clone)]
pub enum NumThreads {
    Auto,
    Number(u32),
}

impl FromStr for NumThreads {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("auto") {
            Ok(NumThreads::Auto)
        } else {
            s.parse::<u32>()
                .map(NumThreads::Number)
                .map_err(|_| format!("Invalid value for num_threads: {}", s))
        }
    }
}

impl std::fmt::Display for NumThreads {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NumThreads::Auto => write!(f, "auto"),
            NumThreads::Number(n) => write!(f, "{}", n),
        }
    }
}

impl NumThreads {
    pub fn resolve(&self) -> u32 {
        match self {
            NumThreads::Auto => std::thread::available_parallelism()
                .map(|n| n.get() as u32)
                .unwrap_or(1),
            NumThreads::Number(n) => *n,
        }
    }
}

/// Blobscan's indexer for the EIP-4844 upgrade.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Slot to start indexing from
    #[arg(short, long)]
    pub from_slot: Option<BlockId>,

    /// Slot to stop indexing at
    #[arg(short, long)]
    pub to_slot: Option<BlockId>,

    /// Number of threads used for parallel indexing ("auto" or a number)
    #[arg(short, long, default_value_t = NumThreads::Auto)]
    pub num_threads: NumThreads,

    /// Amount of slots to be processed before saving latest synced slot in the db
    #[arg(short, long, default_value_t = 1000)]
    pub slots_per_save: u32,

    /// Disable slot checkpoint saving when syncing
    #[arg(short = 'c', long, action = ArgAction::SetTrue)]
    pub disable_sync_checkpoint_save: bool,

    /// Disable historical synchronization
    #[arg(short = 'd', long, action = ArgAction::SetTrue)]
    pub disable_sync_historical: bool,
}
