use std::cmp;

use ethers::providers::JsonRpcClient;
use ethers::types::H256;
use tracing::info;

use crate::{
    clients::{
        beacon::types::{BlockHeader, BlockId, HeadEventData},
        blobscan::types::BlockchainSyncState,
        common::ClientError,
    },
    context::CommonContext,
    synchronizer::{error::SynchronizerError, CommonSynchronizer},
};

#[derive(Debug, thiserror::Error)]
pub enum HeadEventHandlerError {
    #[error(transparent)]
    EventDeserializationFailure(#[from] serde_json::Error),
    #[error("failed to retrieve header for block \"{0}\"")]
    BlockHeaderRetrievalError(BlockId, #[source] ClientError),
    #[error("header for block \"{0}\" not found")]
    BlockHeaderNotFound(BlockId),
    #[error("failed to index head block")]
    BlockSyncedError(#[from] SynchronizerError),
    #[error("failed to handle reorged slots")]
    BlobscanReorgedSlotsFailure(#[source] ClientError),
    #[error("failed to update blobscan's sync state")]
    BlobscanSyncStateUpdateError(#[source] ClientError),
}

pub struct HeadEventHandler<T> {
    context: Box<dyn CommonContext<T>>,
    synchronizer: Box<dyn CommonSynchronizer>,
    start_block_id: BlockId,
    last_block_hash: Option<H256>,
}

impl<T> HeadEventHandler<T>
where
    T: JsonRpcClient + Send + Sync + 'static,
{
    pub fn new(
        context: Box<dyn CommonContext<T>>,
        synchronizer: Box<dyn CommonSynchronizer>,
        start_block_id: BlockId,
    ) -> Self {
        HeadEventHandler {
            context,
            synchronizer,
            start_block_id,
            last_block_hash: None,
        }
    }

    pub async fn handle(&mut self, event_data: String) -> Result<(), HeadEventHandlerError> {
        let head_block_data = serde_json::from_str::<HeadEventData>(&event_data)?;

        let head_block_slot = head_block_data.slot;
        let head_block_hash = head_block_data.block;

        let head_block_id = BlockId::Slot(head_block_data.slot);
        let initial_block_id = if self.last_block_hash.is_none() {
            self.start_block_id.clone()
        } else {
            head_block_id.clone()
        };

        let head_block_header = self.get_block_header(&head_block_id).await?.header;

        if let Some(last_block_hash) = self.last_block_hash {
            if last_block_hash != head_block_header.message.parent_root {
                let parent_block_header = self
                    .get_block_header(&BlockId::Hash(head_block_header.message.parent_root))
                    .await?
                    .header;
                let parent_block_slot = parent_block_header.message.slot;
                let reorg_start_slot = parent_block_slot + 1;
                let reorg_final_slot = head_block_slot;
                let reorged_slots = (reorg_start_slot..reorg_final_slot).collect::<Vec<u32>>();

                let result: Result<(), HeadEventHandlerError> = async {
                    let total_updated_slots = self.context
                        .blobscan_client()
                        .handle_reorged_slots(reorged_slots.as_slice())
                        .await
                        .map_err(HeadEventHandlerError::BlobscanReorgedSlotsFailure)?;


                    info!(slot=head_block_slot, "Reorganization detected. Found the following reorged slots: {:#?}. Total slots marked as reorged: {total_updated_slots}", reorged_slots);

                    // Re-index parent block as it may be mark as reorged and not indexed
                    self.synchronizer
                        .run(
                            &BlockId::Slot(parent_block_slot),
                            &BlockId::Slot(parent_block_slot + 1),
                        )
                        .await?;

                    Ok(())
                }
                .await;

                if let Err(err) = result {
                    // If an error occurred while handling the reorg try to update the latest synced slot to the last known slot before the reorg
                    self.context
                        .blobscan_client()
                        .update_sync_state(BlockchainSyncState {
                            last_finalized_block: None,
                            last_lower_synced_slot: None,
                            last_upper_synced_slot: Some(cmp::max(parent_block_slot - 1, 0)),
                        })
                        .await
                        .map_err(HeadEventHandlerError::BlobscanSyncStateUpdateError)?;

                    return Err(err);
                }
            }
        }

        self.synchronizer
            .run(&initial_block_id, &BlockId::Slot(head_block_slot + 1))
            .await?;

        self.last_block_hash = Some(head_block_hash);

        Ok(())
    }

    async fn get_block_header(
        &self,
        block_id: &BlockId,
    ) -> Result<BlockHeader, HeadEventHandlerError> {
        match self
            .context
            .beacon_client()
            .get_block_header(block_id)
            .await
            .map_err(|err| {
                HeadEventHandlerError::BlockHeaderRetrievalError(block_id.clone(), err)
            })? {
            Some(block) => Ok(block),
            None => Err(HeadEventHandlerError::BlockHeaderNotFound(block_id.clone())),
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;
    use ethers::types::H256;
    use mockall::predicate::eq;

    use super::HeadEventHandler;
    use crate::{
        clients::{
            beacon::{
                types::{BlockHeader, BlockHeaderMessage, BlockId, InnerBlockHeader},
                MockCommonBeaconClient,
            },
            blobscan::{types::BlockchainSyncState, MockCommonBlobscanClient},
        },
        context::Context,
        synchronizer::MockCommonSynchronizer,
    };

    #[derive(Clone, Debug)]
    struct BlockData {
        slot: u32,
        hash: H256,
        parent_hash: Option<H256>,
    }

    impl BlockData {
        pub fn to_head_event(self) -> String {
            format!(
                r#"{{"slot": "{}", "block": "{}"}}"#,
                self.slot,
                format!("0x{:x}", self.hash)
            )
        }
    }

    #[tokio::test]
    async fn test_handler_on_initial_event() {
        let mut mock_synchronizer = Box::new(MockCommonSynchronizer::new());
        let mut mock_beacon_client = MockCommonBeaconClient::new();

        let initial_start_block_id = BlockId::Slot(1);

        let block_data = Box::new(BlockData {
            slot: 4,
            hash: _create_hash("4"),
            parent_hash: None,
        });

        _prepare_handler_calls(
            &mut mock_beacon_client,
            &mut mock_synchronizer,
            &block_data,
            Some(initial_start_block_id.clone()),
        );

        let mock_context = Context::new(Some(mock_beacon_client), None, None);

        let mut head_event_handler =
            HeadEventHandler::new(mock_context, mock_synchronizer, initial_start_block_id);

        let result = head_event_handler.handle(block_data.to_head_event()).await;

        assert!(result.is_ok())
    }

    #[tokio::test]
    async fn test_handler_after_first_event() {
        let mut mock_synchronizer = Box::new(MockCommonSynchronizer::new());
        let mut mock_beacon_client = MockCommonBeaconClient::new();

        let initial_start_block_id = BlockId::Slot(1);

        let first_head_block = BlockData {
            hash: _create_hash("5"),
            slot: 5,
            parent_hash: None,
        };
        let second_head_block = BlockData {
            hash: _create_hash("6"),
            slot: 6,
            parent_hash: Some(first_head_block.hash),
        };

        _prepare_handler_calls(
            &mut mock_beacon_client,
            &mut mock_synchronizer,
            &first_head_block,
            Some(initial_start_block_id.clone()),
        );

        _prepare_handler_calls(
            &mut mock_beacon_client,
            &mut mock_synchronizer,
            &second_head_block,
            None,
        );

        let mock_context = Context::new(Some(mock_beacon_client), None, None);

        let mut head_event_handler =
            HeadEventHandler::new(mock_context, mock_synchronizer, initial_start_block_id);

        let result = head_event_handler
            .handle(first_head_block.to_head_event())
            .await;

        assert!(
            result.is_ok(),
            "Expected first head event handler to succeed"
        );

        let result = head_event_handler
            .handle(second_head_block.to_head_event())
            .await;

        assert!(
            result.is_ok(),
            "Expected second head event handler to succeed"
        );
    }

    #[tokio::test]
    async fn test_handler_on_reorg() {
        let mut mock_synchronizer = Box::new(MockCommonSynchronizer::new());
        let mut mock_beacon_client = MockCommonBeaconClient::new();
        let mut mock_blobscan_client = MockCommonBlobscanClient::new();

        let initial_start_block_id = BlockId::Slot(1);

        let before_reorg_block = BlockData {
            slot: 2,
            hash: _create_hash("2"),
            parent_hash: Some(_create_hash("1")),
        };
        let reorged_block = BlockData {
            slot: 5,
            hash: _create_hash("5"),
            parent_hash: Some(_create_hash("4")),
        };
        let after_reorg_block = BlockData {
            slot: 6,
            hash: _create_hash("3b"),
            parent_hash: Some(before_reorg_block.hash),
        };

        _stub_get_block_header(&mut mock_beacon_client, &before_reorg_block);

        _prepare_handler_calls(
            &mut mock_beacon_client,
            &mut mock_synchronizer,
            &reorged_block,
            Some(initial_start_block_id.clone()),
        );

        _prepare_handler_calls(
            &mut mock_beacon_client,
            &mut mock_synchronizer,
            &after_reorg_block,
            None,
        );

        _stub_handle_reorged_slots(
            &mut mock_blobscan_client,
            (before_reorg_block.slot + 1..after_reorg_block.slot).collect::<Vec<u32>>(),
        );

        // We're expecting the synchronizer to re-sync the parent block of the reorged block
        _stub_synchronizer_run(
            &mut mock_synchronizer,
            BlockId::Slot(before_reorg_block.slot),
            BlockId::Slot(before_reorg_block.slot + 1),
        );

        let mock_context = Context::new(Some(mock_beacon_client), Some(mock_blobscan_client), None);
        let mut head_event_handler =
            HeadEventHandler::new(mock_context, mock_synchronizer, initial_start_block_id);

        let result = head_event_handler
            .handle(reorged_block.to_head_event())
            .await;

        assert!(
            result.is_ok(),
            "Expected first head event handling to succeed"
        );

        let result = head_event_handler
            .handle(after_reorg_block.to_head_event())
            .await;

        assert!(
            result.is_ok(),
            "Expected reorged head event handling to succeed"
        );
    }

    #[tokio::test]
    async fn test_handler_on_one_depth_reorg() {
        // 4 -> 5a
        //      5b -> 6 -> ...
        let mut mock_synchronizer = Box::new(MockCommonSynchronizer::new());
        let mut mock_beacon_client = MockCommonBeaconClient::new();
        let mut mock_blobscan_client = MockCommonBlobscanClient::new();

        let initial_start_block_id = BlockId::Slot(1);

        let block_before_reorg = BlockData {
            slot: 4,
            hash: _create_hash("4"),
            parent_hash: None,
        };
        let reorged_block = BlockData {
            slot: 5,
            hash: _create_hash("50"),
            parent_hash: Some(block_before_reorg.hash),
        };
        let block_after_reorg = BlockData {
            slot: 6,
            hash: _create_hash("5"),
            parent_hash: Some(block_before_reorg.hash),
        };

        _prepare_handler_calls(
            &mut mock_beacon_client,
            &mut mock_synchronizer,
            &reorged_block,
            Some(initial_start_block_id.clone()),
        );
        _prepare_handler_calls(
            &mut mock_beacon_client,
            &mut mock_synchronizer,
            &block_after_reorg,
            None,
        );

        _stub_get_block_header(&mut mock_beacon_client, &block_before_reorg);

        _stub_handle_reorged_slots(&mut mock_blobscan_client, vec![reorged_block.slot]);

        _stub_synchronizer_run(
            &mut mock_synchronizer,
            BlockId::Slot(block_before_reorg.slot),
            BlockId::Slot(block_before_reorg.slot + 1),
        );

        let mock_context = Context::new(Some(mock_beacon_client), Some(mock_blobscan_client), None);

        let mut head_event_handler =
            HeadEventHandler::new(mock_context, mock_synchronizer, initial_start_block_id);

        let result = head_event_handler
            .handle(reorged_block.to_head_event())
            .await;

        assert!(
            result.is_ok(),
            "Expected first head event handling to succeed"
        );

        let result = head_event_handler
            .handle(block_after_reorg.to_head_event())
            .await;

        assert!(
            result.is_ok(),
            "Expected reorged head event handling to succeed"
        );
    }

    #[tokio::test]
    async fn test_handler_on_one_depth_later_reorg() {
        // 4 -> 5a -> 6 -> ...
        //      5b
        let mut mock_synchronizer = Box::new(MockCommonSynchronizer::new());
        let mut mock_beacon_client = MockCommonBeaconClient::new();
        let mut mock_blobscan_client = MockCommonBlobscanClient::new();

        let initial_start_block_id = BlockId::Slot(1);

        let before_reorg_parent_block = BlockData {
            slot: 4,
            hash: _create_hash("4"),
            parent_hash: None,
        };
        let before_reorg_block = BlockData {
            slot: 5,
            hash: _create_hash("50"),
            parent_hash: Some(before_reorg_parent_block.hash),
        };
        let reorged_block = BlockData {
            slot: 6,
            hash: _create_hash("5"),
            parent_hash: Some(before_reorg_parent_block.hash),
        };
        let after_reorg_block = BlockData {
            slot: 7,
            hash: _create_hash("7"),
            parent_hash: Some(before_reorg_block.hash),
        };

        _prepare_handler_calls(
            &mut mock_beacon_client,
            &mut mock_synchronizer,
            &before_reorg_block,
            Some(initial_start_block_id.clone()),
        );
        _prepare_handler_calls(
            &mut mock_beacon_client,
            &mut mock_synchronizer,
            &reorged_block,
            None,
        );
        _prepare_handler_calls(
            &mut mock_beacon_client,
            &mut mock_synchronizer,
            &after_reorg_block,
            None,
        );

        _stub_get_block_header(&mut mock_beacon_client, &before_reorg_parent_block);

        _stub_handle_reorged_slots(&mut mock_blobscan_client, vec![before_reorg_block.slot]);

        _stub_synchronizer_run(
            &mut mock_synchronizer,
            BlockId::Slot(before_reorg_parent_block.slot),
            BlockId::Slot(before_reorg_parent_block.slot + 1),
        );

        _stub_handle_reorged_slots(&mut mock_blobscan_client, vec![reorged_block.slot]);

        _stub_synchronizer_run(
            &mut mock_synchronizer,
            BlockId::Slot(before_reorg_block.slot),
            BlockId::Slot(before_reorg_block.slot + 1),
        );

        let mock_context = Context::new(Some(mock_beacon_client), Some(mock_blobscan_client), None);

        let mut head_event_handler =
            HeadEventHandler::new(mock_context, mock_synchronizer, initial_start_block_id);

        let result = head_event_handler
            .handle(before_reorg_block.to_head_event())
            .await;

        assert!(
            result.is_ok(),
            "Expected first head event handling to succeed"
        );

        let result = head_event_handler
            .handle(reorged_block.to_head_event())
            .await;

        assert!(
            result.is_ok(),
            "Expected reorged head event handling to succeed"
        );

        let result = head_event_handler
            .handle(after_reorg_block.to_head_event())
            .await;

        assert!(
            result.is_ok(),
            "Expected after reorged head event handling to succeed"
        );
    }

    #[tokio::test]
    async fn test_handler_on_reorg_with_error() {
        let mut mock_synchronizer = Box::new(MockCommonSynchronizer::new());
        let mut mock_beacon_client = MockCommonBeaconClient::new();
        let mut mock_blobscan_client = MockCommonBlobscanClient::new();

        let initial_start_block_id = BlockId::Slot(1);

        let before_reorg_parent_block = BlockData {
            slot: 3,
            hash: _create_hash("3"),
            parent_hash: None,
        };
        let before_reorg_block = BlockData {
            slot: 4,
            hash: _create_hash("4"),
            parent_hash: Some(before_reorg_parent_block.hash),
        };
        let first_block = BlockData {
            slot: 5,
            hash: _create_hash("5"),
            parent_hash: Some(before_reorg_block.hash),
        };
        let reorged_block = BlockData {
            slot: 6,
            hash: _create_hash("999"),
            parent_hash: Some(before_reorg_block.hash),
        };

        _prepare_handler_calls(
            &mut mock_beacon_client,
            &mut mock_synchronizer,
            &first_block,
            Some(initial_start_block_id.clone()),
        );

        _stub_get_block_header(&mut mock_beacon_client, &reorged_block);

        _stub_get_block_header(&mut mock_beacon_client, &before_reorg_block);

        mock_blobscan_client
            .expect_handle_reorged_slots()
            .returning(|_x| {
                Box::pin(async move {
                    Err(crate::clients::common::ClientError::Other(anyhow!(
                        "Internal blobscan client error"
                    )))
                })
            });

        mock_blobscan_client
            .expect_update_sync_state()
            .times(1)
            .with(eq(BlockchainSyncState {
                last_finalized_block: None,
                last_lower_synced_slot: None,
                last_upper_synced_slot: Some(before_reorg_parent_block.slot),
            }))
            .returning(|_x| Box::pin(async move { Ok(()) }));

        let mock_context = Context::new(Some(mock_beacon_client), Some(mock_blobscan_client), None);

        let mut head_event_handler =
            HeadEventHandler::new(mock_context, mock_synchronizer, initial_start_block_id);

        let result = head_event_handler.handle(first_block.to_head_event()).await;

        assert!(
            result.is_ok(),
            "Expected first head event handling to succeed"
        );

        let result = head_event_handler
            .handle(reorged_block.to_head_event())
            .await;

        assert!(
            result.is_err(),
            "Expected reorged head event handling to fail"
        );
    }

    fn _prepare_handler_calls(
        mock_beacon_client: &mut MockCommonBeaconClient,
        mock_synchronizer: &mut MockCommonSynchronizer,
        head_block_data: &BlockData,
        initial_block_id: Option<BlockId>,
    ) {
        let slot = head_block_data.slot;

        _stub_get_block_header(mock_beacon_client, head_block_data);

        _stub_synchronizer_run(
            mock_synchronizer,
            initial_block_id.unwrap_or(BlockId::Slot(slot)),
            BlockId::Slot(slot + 1),
        )
    }

    fn _stub_get_block_header(
        mock_beacon_client: &mut MockCommonBeaconClient,
        block_data: &BlockData,
    ) {
        let root = block_data.hash;
        let slot = block_data.slot;
        let parent_root = block_data
            .parent_hash
            .unwrap_or(_create_hash((slot - 1).to_string().as_str()));

        mock_beacon_client
            .expect_get_block_header()
            .with(eq(BlockId::Slot(block_data.slot)))
            .returning(move |_x| {
                Box::pin(async move {
                    Ok(Some(BlockHeader {
                        root,
                        header: InnerBlockHeader {
                            message: BlockHeaderMessage { parent_root, slot },
                        },
                    }))
                })
            });
        mock_beacon_client
            .expect_get_block_header()
            .with(eq(BlockId::Hash(block_data.hash)))
            .returning(move |_x| {
                Box::pin(async move {
                    Ok(Some(BlockHeader {
                        root,
                        header: InnerBlockHeader {
                            message: BlockHeaderMessage { parent_root, slot },
                        },
                    }))
                })
            });
    }

    fn _stub_handle_reorged_slots(
        mock_blobscan_client: &mut MockCommonBlobscanClient,
        reorged_slots: Vec<u32>,
    ) {
        let reorged_slots_len = reorged_slots.len() as u32;

        mock_blobscan_client
            .expect_handle_reorged_slots()
            .with(eq(reorged_slots))
            .returning(move |_x| Box::pin(async move { Ok(reorged_slots_len) }));
    }

    fn _stub_synchronizer_run(
        mock_synchronizer: &mut MockCommonSynchronizer,
        initial_block_id: BlockId,
        final_block_id: BlockId,
    ) {
        mock_synchronizer
            .expect_run()
            .times(1)
            .with(eq(initial_block_id.clone()), eq(final_block_id))
            .returning(|_x, _y| Box::pin(async { Ok(()) }));
    }

    fn _create_hash(input: &str) -> H256 {
        // Ensure the input string is at most 64 characters
        let truncated_input = if input.len() > 64 {
            &input[0..64]
        } else {
            input
        };

        // Format the string to have a length of 64 characters by padding with zeros
        let hash = format!("0x{:0>64}", truncated_input);

        hash.parse().unwrap()
    }

    fn _create_head_event(slot: u32, block_hash: H256) -> String {
        let head_event = format!(
            r#"{{"slot": "{}", "block": "{}"}}"#,
            slot,
            format!("0x{:x}", block_hash)
        );

        head_event
    }

    // Additional tests for error handling, etc.
}
