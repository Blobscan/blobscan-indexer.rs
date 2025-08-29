use tokio::{
    sync::{
        mpsc::{Receiver as MPSCReceiver, Sender as MPSCSender},
        oneshot::{Receiver as OneshotReceiver, Sender as OneshotSender},
    },
    task::JoinHandle,
};

use crate::indexer::error::IndexerTaskError;

pub struct ErrorResport {
    pub task_name: String,
    pub error: IndexerTaskError,
}

pub type TaskResult = ();
pub type TaskResultChannelSender = OneshotSender<TaskResult>;
pub type TaskResultChannelReceiver = OneshotReceiver<TaskResult>;
pub type TaskErrorChannelSender = MPSCSender<ErrorResport>;
pub type TaskErrorChannelReceiver = MPSCReceiver<ErrorResport>;

pub type IndexingTaskJoinHandle = JoinHandle<()>;
