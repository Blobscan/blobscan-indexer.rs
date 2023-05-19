use backoff::{ExponentialBackoff, ExponentialBackoffBuilder};

pub fn get_exp_backoff_config() -> ExponentialBackoff {
    ExponentialBackoffBuilder::default().build()
}
