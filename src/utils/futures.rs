use std::{future::Future, time::Duration};

use tracing::debug;

/// Retries an async operation that returns `Ok(None)` when the resource is not yet available.
/// Useful for handling race conditions where data is expected but may not be immediately queryable.
pub async fn retry_on_none<F, Fut, T, E>(
    fetch: F,
    max_attempts: u32,
    retry_delay: Duration,
) -> Result<Option<T>, E>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<Option<T>, E>>,
{
    for attempt in 1..=max_attempts {
        match fetch().await? {
            Some(value) => return Ok(Some(value)),
            None if attempt < max_attempts => {
                debug!(
                    attempt,
                    "Resource not yet available, retrying in {}ms…",
                    retry_delay.as_millis()
                );

                tokio::time::sleep(retry_delay).await;
            }
            None => break,
        }
    }

    Ok(None)
}
