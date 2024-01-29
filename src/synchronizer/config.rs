use std::thread;

use anyhow::anyhow;

#[derive(Debug)]
pub struct Config {
    pub num_threads: u32,
    pub slots_checkpoint: u32,
}

#[derive(Debug)]
pub struct ConfigBuilder {
    num_threads: u32,
    slots_checkpoint: u32,
}

impl ConfigBuilder {
    pub fn new() -> Result<Self, anyhow::Error> {
        ConfigBuilder::default()
    }

    pub fn default() -> Result<Self, anyhow::Error> {
        Ok(Self {
            num_threads: thread::available_parallelism()
                .map_err(|err| anyhow!("Failed to get number of available threads: {:?}", err))?
                .get() as u32,
            slots_checkpoint: 1000,
        })
    }

    pub fn with_num_threads(&mut self, num_threads: u32) -> &mut Self {
        self.num_threads = num_threads;
        self
    }

    pub fn with_slots_checkpoint(&mut self, slots_checkpoint: u32) -> &mut Self {
        self.slots_checkpoint = slots_checkpoint;
        self
    }

    pub fn build(&self) -> Config {
        Config {
            num_threads: self.num_threads,
            slots_checkpoint: self.slots_checkpoint,
        }
    }
}
