use std::time::Duration;

use alloy::{eips::BlockId as ExecutionBlockId, network::Ethereum, providers::Provider};
use anyhow::{anyhow, Result};

use crate::{clients::beacon::types::BlockExecutionPayload, utils::futures::retry_on_none};

/// Returns the number of the last execution block that is part of the chain at a beacon block.
///
/// Before Gloas this is the payload of the beacon block, and the block has its number. A Gloas
/// block has a bid, and the builder can withhold the payload of the bid. Thus this uses the
/// payload that the bid builds on (`parent_block_hash`). That payload is part of the chain, but
/// it can be one payload older than the beacon block. The number comes from the execution client.
///
/// `Ok(None)` means that the execution client does not know the hash after `attempts` tries. The
/// payload is part of the chain, so the execution client is behind the beacon node.
pub async fn resolve_execution_block_number(
    provider: &dyn Provider<Ethereum>,
    payload: &BlockExecutionPayload,
    attempts: u32,
    delay: Duration,
) -> Result<Option<u32>> {
    let block_hash = match payload {
        BlockExecutionPayload::InBody { block_number, .. } => return Ok(Some(*block_number)),
        BlockExecutionPayload::Bid {
            parent_block_hash, ..
        } => *parent_block_hash,
    };
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
        match block_number {
            Some(block_number) => BlockExecutionPayload::InBody {
                block_hash: B256::repeat_byte(0x07),
                block_number,
            },
            None => BlockExecutionPayload::Bid {
                block_hash: B256::repeat_byte(0x07),
                parent_block_hash: B256::repeat_byte(0x06),
            },
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
    async fn resolves_the_payload_the_bid_builds_on_through_the_execution_client() {
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());

        asserter.push_success(&execution_block(11942470));

        let number = resolve_execution_block_number(&provider, &payload(None), 3, DELAY)
            .await
            .unwrap();

        assert_eq!(number, Some(11942470));
    }

    #[tokio::test]
    async fn waits_for_an_execution_client_that_is_a_little_behind() {
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
    async fn an_execution_client_that_stays_behind_gives_none_not_an_error() {
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
