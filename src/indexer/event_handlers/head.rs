use crate::{
    clients::beacon::types::{BlockId, HeadEventData},
    synchronizer::{error::SynchronizerError, CommonSynchronizer},
};

#[derive(Debug, thiserror::Error)]
pub enum HeadEventHandlerError {
    #[error(transparent)]
    EventDeserializationFailure(#[from] serde_json::Error),
    #[error("failed to index head block")]
    BlockSyncedError(#[from] SynchronizerError),
}

pub struct HeadEventHandler {
    synchronizer: Box<dyn CommonSynchronizer>,
    is_first_event: bool,
    custom_start_block_id: Option<BlockId>,
}

impl HeadEventHandler {
    pub fn new(
        synchronizer: Box<dyn CommonSynchronizer>,
        custom_start_block_id: Option<BlockId>,
    ) -> Self {
        HeadEventHandler {
            synchronizer,
            is_first_event: true,
            custom_start_block_id,
        }
    }

    pub async fn handle(&mut self, event_data: String) -> Result<(), HeadEventHandlerError> {
        let head_block_data = serde_json::from_str::<HeadEventData>(&event_data)?;
        let head_slot = head_block_data.slot;

        // If this is the first event being processed, ensure the synchronizer is fully up to date
        if self.is_first_event {
            self.is_first_event = false;

            let start_block_id = self.custom_start_block_id.clone().or(self
                .synchronizer
                .get_last_synced_block()
                .map(|block| (block.slot + 1).into()));

            if let Some(start_block_id) = start_block_id {
                if self.custom_start_block_id.is_some() {
                    self.synchronizer.clear_last_synced_block();
                }

                self.synchronizer
                    .sync_blocks(start_block_id, head_slot.into())
                    .await?;
            }
        }

        self.synchronizer.sync_block(head_slot.into()).await?;

        Ok(())
    }
}

// #[cfg(test)]
// mod tests {
//     use alloy::primitives::B256;
//     use anyhow::anyhow;
//     use mockall::predicate::eq;

//     use super::HeadEventHandler;
//     use crate::{
//         clients::{
//             beacon::{
//                 types::{BlockHeader, BlockHeaderMessage, BlockId, InnerBlockHeader},
//                 MockCommonBeaconClient,
//             },
//             blobscan::{types::BlockchainSyncState, MockCommonBlobscanClient},
//         },
//         context::Context,
//         synchronizer::MockCommonSynchronizer,
//     };

//     #[derive(Clone, Debug)]
//     struct BlockData {
//         slot: u32,
//         hash: B256,
//         parent_hash: Option<B256>,
//     }

//     impl BlockData {
//         pub fn to_head_event(self) -> String {
//             format!(
//                 r#"{{"slot": "{}", "block": "{}"}}"#,
//                 self.slot,
//                 format!("0x{:x}", self.hash)
//             )
//         }
//     }

//     #[tokio::test]
//     async fn test_handler_on_initial_event() {
//         let mut mock_synchronizer = Box::new(MockCommonSynchronizer::new());
//         let mut mock_beacon_client = MockCommonBeaconClient::new();

//         let initial_start_block_id = BlockId::Slot(1);

//         let block_data = Box::new(BlockData {
//             slot: 4,
//             hash: _create_hash("4"),
//             parent_hash: None,
//         });

//         _prepare_handler_calls(
//             &mut mock_beacon_client,
//             &mut mock_synchronizer,
//             &block_data,
//             Some(initial_start_block_id.clone()),
//         );

//         let mock_context = Context::new(Some(mock_beacon_client), None, None);

//         let mut head_event_handler =
//             HeadEventHandler::new(mock_context, mock_synchronizer, initial_start_block_id);

//         let result = head_event_handler.handle(block_data.to_head_event()).await;

//         assert!(result.is_ok())
//     }

//     #[tokio::test]
//     async fn test_handler_after_first_event() {
//         let mut mock_synchronizer = Box::new(MockCommonSynchronizer::new());
//         let mut mock_beacon_client = MockCommonBeaconClient::new();

//         let initial_start_block_id = BlockId::Slot(1);

//         let first_head_block = BlockData {
//             hash: _create_hash("5"),
//             slot: 5,
//             parent_hash: None,
//         };
//         let second_head_block = BlockData {
//             hash: _create_hash("6"),
//             slot: 6,
//             parent_hash: Some(first_head_block.hash),
//         };

//         _prepare_handler_calls(
//             &mut mock_beacon_client,
//             &mut mock_synchronizer,
//             &first_head_block,
//             Some(initial_start_block_id.clone()),
//         );

//         _prepare_handler_calls(
//             &mut mock_beacon_client,
//             &mut mock_synchronizer,
//             &second_head_block,
//             None,
//         );

//         let mock_context = Context::new(Some(mock_beacon_client), None, None);

//         let mut head_event_handler =
//             HeadEventHandler::new(mock_context, mock_synchronizer, initial_start_block_id);

//         let result = head_event_handler
//             .handle(first_head_block.to_head_event())
//             .await;

//         assert!(
//             result.is_ok(),
//             "Expected first head event handler to succeed"
//         );

//         let result = head_event_handler
//             .handle(second_head_block.to_head_event())
//             .await;

//         assert!(
//             result.is_ok(),
//             "Expected second head event handler to succeed"
//         );
//     }

//     #[tokio::test]
//     async fn test_handler_on_reorg() {
//         let mut mock_synchronizer = Box::new(MockCommonSynchronizer::new());
//         let mut mock_beacon_client = MockCommonBeaconClient::new();
//         let mut mock_blobscan_client = MockCommonBlobscanClient::new();

//         let initial_start_block_id = BlockId::Slot(1);

//         let before_reorg_block = BlockData {
//             slot: 2,
//             hash: _create_hash("2"),
//             parent_hash: Some(_create_hash("1")),
//         };
//         let reorged_block = BlockData {
//             slot: 5,
//             hash: _create_hash("5"),
//             parent_hash: Some(_create_hash("4")),
//         };
//         let after_reorg_block = BlockData {
//             slot: 6,
//             hash: _create_hash("3b"),
//             parent_hash: Some(before_reorg_block.hash),
//         };

//         _stub_get_block_header(&mut mock_beacon_client, &before_reorg_block);

//         _prepare_handler_calls(
//             &mut mock_beacon_client,
//             &mut mock_synchronizer,
//             &reorged_block,
//             Some(initial_start_block_id.clone()),
//         );

//         _prepare_handler_calls(
//             &mut mock_beacon_client,
//             &mut mock_synchronizer,
//             &after_reorg_block,
//             None,
//         );

//         _stub_handle_reorged_slots(
//             &mut mock_blobscan_client,
//             (before_reorg_block.slot + 1..after_reorg_block.slot).collect::<Vec<u32>>(),
//         );

//         // We're expecting the synchronizer to re-sync the parent block of the reorged block
//         _stub_synchronizer_run(
//             &mut mock_synchronizer,
//             BlockId::Slot(before_reorg_block.slot),
//             BlockId::Slot(before_reorg_block.slot + 1),
//         );

//         let mock_context = Context::new(Some(mock_beacon_client), Some(mock_blobscan_client), None);
//         let mut head_event_handler =
//             HeadEventHandler::new(mock_context, mock_synchronizer, initial_start_block_id);

//         let result = head_event_handler
//             .handle(reorged_block.to_head_event())
//             .await;

//         assert!(
//             result.is_ok(),
//             "Expected first head event handling to succeed"
//         );

//         let result = head_event_handler
//             .handle(after_reorg_block.to_head_event())
//             .await;

//         assert!(
//             result.is_ok(),
//             "Expected reorged head event handling to succeed"
//         );
//     }

//     #[tokio::test]
//     async fn test_handler_on_one_depth_reorg() {
//         // Slots:
//         // 4 -> 5
//         //      6 -> 7 -> ...
//         let mut mock_synchronizer = Box::new(MockCommonSynchronizer::new());
//         let mut mock_beacon_client = MockCommonBeaconClient::new();
//         let mut mock_blobscan_client = MockCommonBlobscanClient::new();

//         let initial_start_block_id = BlockId::Slot(1);

//         let block_before_reorg = BlockData {
//             slot: 4,
//             hash: _create_hash("4"),
//             parent_hash: None,
//         };
//         let reorged_block = BlockData {
//             slot: 5,
//             hash: _create_hash("50"),
//             parent_hash: Some(block_before_reorg.hash),
//         };
//         let block_after_reorg = BlockData {
//             slot: 6,
//             hash: _create_hash("5"),
//             parent_hash: Some(block_before_reorg.hash),
//         };

//         _prepare_handler_calls(
//             &mut mock_beacon_client,
//             &mut mock_synchronizer,
//             &reorged_block,
//             Some(initial_start_block_id.clone()),
//         );
//         _prepare_handler_calls(
//             &mut mock_beacon_client,
//             &mut mock_synchronizer,
//             &block_after_reorg,
//             None,
//         );

//         _stub_get_block_header(&mut mock_beacon_client, &block_before_reorg);

//         _stub_handle_reorged_slots(&mut mock_blobscan_client, vec![reorged_block.slot]);

//         _stub_synchronizer_run(
//             &mut mock_synchronizer,
//             BlockId::Slot(block_before_reorg.slot),
//             BlockId::Slot(block_before_reorg.slot + 1),
//         );

//         let mock_context = Context::new(Some(mock_beacon_client), Some(mock_blobscan_client), None);

//         let mut head_event_handler =
//             HeadEventHandler::new(mock_context, mock_synchronizer, initial_start_block_id);

//         let result = head_event_handler
//             .handle(reorged_block.to_head_event())
//             .await;

//         assert!(
//             result.is_ok(),
//             "Expected first head event handling to succeed"
//         );

//         let result = head_event_handler
//             .handle(block_after_reorg.to_head_event())
//             .await;

//         assert!(
//             result.is_ok(),
//             "Expected reorged head event handling to succeed"
//         );
//     }

//     #[tokio::test]
//     async fn test_handler_on_one_depth_former_reorg() {
//         // Reorged block is reorged back to its former parent
//         // Slots:
//         // 4 -> 5 -> 7 -> ...
//         //      6
//         let mut mock_synchronizer = Box::new(MockCommonSynchronizer::new());
//         let mut mock_beacon_client = MockCommonBeaconClient::new();
//         let mut mock_blobscan_client = MockCommonBlobscanClient::new();

//         let initial_start_block_id = BlockId::Slot(1);

//         let before_reorg_parent_block = BlockData {
//             slot: 4,
//             hash: _create_hash("4"),
//             parent_hash: None,
//         };
//         let before_reorg_block = BlockData {
//             slot: 5,
//             hash: _create_hash("50"),
//             parent_hash: Some(before_reorg_parent_block.hash),
//         };
//         let reorged_block = BlockData {
//             slot: 6,
//             hash: _create_hash("5"),
//             parent_hash: Some(before_reorg_parent_block.hash),
//         };
//         let after_reorg_block = BlockData {
//             slot: 7,
//             hash: _create_hash("7"),
//             parent_hash: Some(before_reorg_block.hash),
//         };

//         _prepare_handler_calls(
//             &mut mock_beacon_client,
//             &mut mock_synchronizer,
//             &before_reorg_block,
//             Some(initial_start_block_id.clone()),
//         );
//         _prepare_handler_calls(
//             &mut mock_beacon_client,
//             &mut mock_synchronizer,
//             &reorged_block,
//             None,
//         );
//         _prepare_handler_calls(
//             &mut mock_beacon_client,
//             &mut mock_synchronizer,
//             &after_reorg_block,
//             None,
//         );

//         _stub_get_block_header(&mut mock_beacon_client, &before_reorg_parent_block);

//         _stub_handle_reorged_slots(&mut mock_blobscan_client, vec![before_reorg_block.slot]);

//         _stub_synchronizer_run(
//             &mut mock_synchronizer,
//             BlockId::Slot(before_reorg_parent_block.slot),
//             BlockId::Slot(before_reorg_parent_block.slot + 1),
//         );

//         _stub_handle_reorged_slots(&mut mock_blobscan_client, vec![reorged_block.slot]);

//         _stub_synchronizer_run(
//             &mut mock_synchronizer,
//             BlockId::Slot(before_reorg_block.slot),
//             BlockId::Slot(before_reorg_block.slot + 1),
//         );

//         let mock_context = Context::new(Some(mock_beacon_client), Some(mock_blobscan_client), None);

//         let mut head_event_handler =
//             HeadEventHandler::new(mock_context, mock_synchronizer, initial_start_block_id);

//         let result = head_event_handler
//             .handle(before_reorg_block.to_head_event())
//             .await;

//         assert!(
//             result.is_ok(),
//             "Expected first head event handling to succeed"
//         );

//         let result = head_event_handler
//             .handle(reorged_block.to_head_event())
//             .await;

//         assert!(
//             result.is_ok(),
//             "Expected reorged head event handling to succeed"
//         );

//         let result = head_event_handler
//             .handle(after_reorg_block.to_head_event())
//             .await;

//         assert!(
//             result.is_ok(),
//             "Expected after reorged head event handling to succeed"
//         );
//     }

//     #[tokio::test]
//     async fn test_handler_on_reorg_with_error() {
//         let mut mock_synchronizer = Box::new(MockCommonSynchronizer::new());
//         let mut mock_beacon_client = MockCommonBeaconClient::new();
//         let mut mock_blobscan_client = MockCommonBlobscanClient::new();

//         let initial_start_block_id = BlockId::Slot(1);

//         let before_reorg_parent_block = BlockData {
//             slot: 3,
//             hash: _create_hash("3"),
//             parent_hash: None,
//         };
//         let before_reorg_block = BlockData {
//             slot: 4,
//             hash: _create_hash("4"),
//             parent_hash: Some(before_reorg_parent_block.hash),
//         };
//         let first_block = BlockData {
//             slot: 5,
//             hash: _create_hash("5"),
//             parent_hash: Some(before_reorg_block.hash),
//         };
//         let reorged_block = BlockData {
//             slot: 6,
//             hash: _create_hash("999"),
//             parent_hash: Some(before_reorg_block.hash),
//         };

//         _prepare_handler_calls(
//             &mut mock_beacon_client,
//             &mut mock_synchronizer,
//             &first_block,
//             Some(initial_start_block_id.clone()),
//         );

//         _stub_get_block_header(&mut mock_beacon_client, &reorged_block);

//         _stub_get_block_header(&mut mock_beacon_client, &before_reorg_block);

//         mock_blobscan_client
//             .expect_handle_reorged_slots()
//             .returning(|_x| {
//                 Box::pin(async move {
//                     Err(crate::clients::common::ClientError::Other(anyhow!(
//                         "Internal blobscan client error"
//                     )))
//                 })
//             });

//         mock_blobscan_client
//             .expect_update_sync_state()
//             .times(1)
//             .with(eq(BlockchainSyncState {
//                 last_finalized_block: None,
//                 last_lower_synced_slot: None,
//                 last_upper_synced_slot: Some(before_reorg_parent_block.slot),
//             }))
//             .returning(|_x| Box::pin(async move { Ok(()) }));

//         let mock_context = Context::new(Some(mock_beacon_client), Some(mock_blobscan_client), None);

//         let mut head_event_handler =
//             HeadEventHandler::new(mock_context, mock_synchronizer, initial_start_block_id);

//         let result = head_event_handler.handle(first_block.to_head_event()).await;

//         assert!(
//             result.is_ok(),
//             "Expected first head event handling to succeed"
//         );

//         let result = head_event_handler
//             .handle(reorged_block.to_head_event())
//             .await;

//         assert!(
//             result.is_err(),
//             "Expected reorged head event handling to fail"
//         );
//     }

//     fn _prepare_handler_calls(
//         mock_beacon_client: &mut MockCommonBeaconClient,
//         mock_synchronizer: &mut MockCommonSynchronizer,
//         head_block_data: &BlockData,
//         initial_block_id: Option<BlockId>,
//     ) {
//         let slot = head_block_data.slot;

//         _stub_get_block_header(mock_beacon_client, head_block_data);

//         _stub_synchronizer_run(
//             mock_synchronizer,
//             initial_block_id.unwrap_or(BlockId::Slot(slot)),
//             BlockId::Slot(slot + 1),
//         )
//     }

//     fn _stub_get_block_header(
//         mock_beacon_client: &mut MockCommonBeaconClient,
//         block_data: &BlockData,
//     ) {
//         let root = block_data.hash;
//         let slot = block_data.slot;
//         let parent_root = block_data
//             .parent_hash
//             .unwrap_or(_create_hash((slot - 1).to_string().as_str()));

//         mock_beacon_client
//             .expect_get_block_header()
//             .with(eq(BlockId::Slot(block_data.slot)))
//             .returning(move |_x| {
//                 Box::pin(async move {
//                     Ok(Some(BlockHeader {
//                         root,
//                         header: InnerBlockHeader {
//                             message: BlockHeaderMessage { parent_root, slot },
//                         },
//                     }))
//                 })
//             });
//         mock_beacon_client
//             .expect_get_block_header()
//             .with(eq(BlockId::Hash(block_data.hash)))
//             .returning(move |_x| {
//                 Box::pin(async move {
//                     Ok(Some(BlockHeader {
//                         root,
//                         header: InnerBlockHeader {
//                             message: BlockHeaderMessage { parent_root, slot },
//                         },
//                     }))
//                 })
//             });
//     }

//     fn _stub_handle_reorged_slots(
//         mock_blobscan_client: &mut MockCommonBlobscanClient,
//         reorged_slots: Vec<u32>,
//     ) {
//         let reorged_slots_len = reorged_slots.len() as u32;

//         mock_blobscan_client
//             .expect_handle_reorged_slots()
//             .with(eq(reorged_slots))
//             .returning(move |_x| Box::pin(async move { Ok(reorged_slots_len) }));
//     }

//     fn _stub_synchronizer_run(
//         mock_synchronizer: &mut MockCommonSynchronizer,
//         initial_block_id: BlockId,
//         final_block_id: BlockId,
//     ) {
//         mock_synchronizer
//             .expect_run()
//             .times(1)
//             .with(eq(initial_block_id.clone()), eq(final_block_id))
//             .returning(|_x, _y| Box::pin(async { Ok(()) }));
//     }

//     fn _create_hash(input: &str) -> B256 {
//         // Ensure the input string is at most 64 characters
//         let truncated_input = if input.len() > 64 {
//             &input[0..64]
//         } else {
//             input
//         };

//         // Format the string to have a length of 64 characters by padding with zeros
//         let hash = format!("0x{:0>64}", truncated_input);

//         hash.parse().unwrap()
//     }

//     fn _create_head_event(slot: u32, block_hash: B256) -> String {
//         let head_event = format!(
//             r#"{{"slot": "{}", "block": "{}"}}"#,
//             slot,
//             format!("0x{:x}", block_hash)
//         );

//         head_event
//     }
// }
