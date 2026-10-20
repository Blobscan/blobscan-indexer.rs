use std::time::Duration;

use alloy::{eips::BlockId as ExecutionBlockId, network::Ethereum, providers::Provider};
use anyhow::{anyhow, Result};

use crate::{clients::beacon::types::BlockExecutionPayload, utils::futures::retry_on_none};

/// Returns the execution block number of a beacon block's payload.
///
/// Pre-Gloas beacon blocks carry the number. A Gloas bid only commits to the payload by hash, so
/// the number comes from the execution client. `Ok(None)` means the execution client does not
/// know the hash after `attempts` tries: under ePBS the payload is revealed after the beacon
/// block, and a builder can withhold it, so this is not an error by itself.
pub async fn resolve_execution_block_number(
    provider: &dyn Provider<Ethereum>,
    payload: &BlockExecutionPayload,
    attempts: u32,
    delay: Duration,
) -> Result<Option<u32>> {
    if let Some(number) = payload.block_number {
        return Ok(Some(number));
    }

    let block_hash = payload.block_hash;
    let block = retry_on_none(
        || async move {
            provider
                .get_block(ExecutionBlockId::Hash(block_hash.into()))
                .await
                .map_err(anyhow::Error::from)
        },
        attempts,
        delay,
    )
    .await?;

    block
        .map(|block| {
            u32::try_from(block.header.number).map_err(|_| {
                anyhow!(
                    "Execution block number {} of block '{block_hash}' does not fit in a u32",
                    block.header.number
                )
            })
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use alloy::{primitives::B256, providers::ProviderBuilder, transports::mock::Asserter};
    use serde_json::{json, Value};

    use super::*;

    const DELAY: Duration = Duration::from_millis(1);

    fn payload(block_number: Option<u32>) -> BlockExecutionPayload {
        BlockExecutionPayload {
            block_hash: B256::repeat_byte(0x07),
            block_number,
            from_bid: block_number.is_none(),
        }
    }

    /// An `eth_getBlockByHash` result with just the fields alloy requires to read the number.
    fn execution_block(number: u64) -> Value {
        let mut block: Value = serde_json::from_str(include_str!(
            "../slots_processor/test_data/sepolia_execution_block.json"
        ))
        .unwrap();

        block["number"] = json!(format!("{number:#x}"));
        block["transactions"] = json!([]);
        block
    }

    #[tokio::test]
    async fn uses_the_number_the_block_already_has_without_asking_the_execution_client() {
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());

        let number = resolve_execution_block_number(&provider, &payload(Some(5)), 3, DELAY)
            .await
            .unwrap();

        assert_eq!(number, Some(5));
        assert!(asserter.read_q().is_empty());
    }

    #[tokio::test]
    async fn resolves_a_bid_hash_through_the_execution_client() {
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());

        asserter.push_success(&execution_block(11942470));

        let number = resolve_execution_block_number(&provider, &payload(None), 3, DELAY)
            .await
            .unwrap();

        assert_eq!(number, Some(11942470));
    }

    #[tokio::test]
    async fn waits_for_a_payload_that_is_revealed_late() {
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());

        asserter.push_success(&Value::Null);
        asserter.push_success(&Value::Null);
        asserter.push_success(&execution_block(42));

        let number = resolve_execution_block_number(&provider, &payload(None), 5, DELAY)
            .await
            .unwrap();

        assert_eq!(number, Some(42));
    }

    #[tokio::test]
    async fn a_payload_that_never_shows_up_is_none_not_an_error() {
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());

        for _ in 0..3 {
            asserter.push_success(&Value::Null);
        }

        let number = resolve_execution_block_number(&provider, &payload(None), 3, DELAY)
            .await
            .unwrap();

        assert_eq!(number, None);
        assert!(asserter.read_q().is_empty(), "it must try every attempt");
    }
}
