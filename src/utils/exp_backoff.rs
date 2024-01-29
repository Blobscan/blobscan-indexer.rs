use backoff::{ExponentialBackoff, ExponentialBackoffBuilder};

pub fn build_exp_backoff_config() -> ExponentialBackoff {
    ExponentialBackoffBuilder::default().build()
}
