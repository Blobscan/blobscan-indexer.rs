use std::{fmt::Debug, time::Duration};

use anyhow::{anyhow, Context as AnyhowContext};
use async_trait::async_trait;
use backon::ExponentialBuilder;

use reqwest::{Client, Url};
use reqwest_eventsource::EventSource;

#[cfg(test)]
use mockall::automock;
use types::BlockHeader;

use crate::{
    clients::{
        beacon::types::{BlockHeaderResponse, Spec, SpecResponse},
        common::ClientResult,
    },
    json_get,
};

use self::types::{Blob, BlobsResponse, Block, BlockId, BlockResponse, Topic};

pub mod types;

#[derive(Debug, Clone)]
pub struct BeaconClient {
    base_url: Url,
    client: Client,
    sse_client: Client,
    exp_backoff: Option<ExponentialBuilder>,
}

pub struct Config {
    pub base_url: String,
    pub exp_backoff: Option<ExponentialBuilder>,
}

#[async_trait]
#[cfg_attr(test, automock)]
pub trait CommonBeaconClient: Send + Sync + Debug {
    async fn get_block(&self, block_id: BlockId) -> ClientResult<Option<Block>>;
    async fn get_block_header(&self, block_id: BlockId) -> ClientResult<Option<BlockHeader>>;
    async fn get_blobs(&self, block_id: BlockId) -> ClientResult<Option<Vec<Blob>>>;
    async fn get_spec(&self) -> ClientResult<Option<Spec>>;
    fn subscribe_to_events(&self, topics: &[Topic]) -> ClientResult<EventSource>;
}

impl BeaconClient {
    pub fn try_with_client(client: Client, config: Config) -> ClientResult<Self> {
        let base_url = Url::parse(&format!("{}/eth/", config.base_url))
            .with_context(|| "Failed to parse base URL")?;
        let exp_backoff = config.exp_backoff;

        let sse_client = Client::builder()
            .connect_timeout(Duration::from_secs(32))
            .build()
            .with_context(|| "Failed to build SSE client")?;

        Ok(Self {
            base_url,
            client,
            sse_client,
            exp_backoff,
        })
    }
}

#[async_trait]
impl CommonBeaconClient for BeaconClient {
    async fn get_block(&self, block_id: BlockId) -> ClientResult<Option<Block>> {
        let path = format!("v2/beacon/blocks/{}", { block_id.to_detailed_string() });
        let url = self.base_url.join(path.as_str())?;

        json_get!(&self.client, url, BlockResponse, self.exp_backoff.clone())
            .map(|res| res.map(|r| r.into()))
    }

    async fn get_block_header(&self, block_id: BlockId) -> ClientResult<Option<BlockHeader>> {
        let path = format!("v1/beacon/headers/{}", { block_id.to_detailed_string() });
        let url = self.base_url.join(path.as_str())?;

        json_get!(
            &self.client,
            url,
            BlockHeaderResponse,
            self.exp_backoff.clone()
        )
        .map(|res| res.map(|r| r.into()))
    }

    async fn get_blobs(&self, block_id: BlockId) -> ClientResult<Option<Vec<Blob>>> {
        let path = format!("v1/beacon/blob_sidecars/{}", {
            block_id.to_detailed_string()
        });
        let url = self.base_url.join(path.as_str())?;

        json_get!(&self.client, url, BlobsResponse, self.exp_backoff.clone()).map(|res| match res {
            Some(r) => Some(r.data),
            None => None,
        })
    }

    async fn get_spec(&self) -> ClientResult<Option<Spec>> {
        let url = self.base_url.join("v1/config/spec")?;

        json_get!(&self.client, url, SpecResponse, self.exp_backoff.clone()).map(|res| match res {
            Some(r) => Some(r.data),
            None => None,
        })
    }

    fn subscribe_to_events(&self, topics: &[Topic]) -> ClientResult<EventSource> {
        let topics = topics
            .iter()
            .map(|topic| topic.into())
            .collect::<Vec<String>>()
            .join(",");
        let path = format!("v1/events?topics={topics}");
        let url = self.base_url.join(&path)?;

        EventSource::new(self.sse_client.get(url))
            .map_err(|e| anyhow!("Failed to create SSE event source: {e}").into())
    }
}

#[cfg(test)]
mod retry_tests {
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        time::Duration,
    };

    use backon::ExponentialBuilder;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;

    const HEADER_JSON: &str = r#"{"data":{"root":"0x0000000000000000000000000000000000000000000000000000000000000001","header":{"message":{"parent_root":"0x0000000000000000000000000000000000000000000000000000000000000002","slot":"7"}}}}"#;

    /// Serves `statuses` in order (the last one repeats), counting the requests.
    async fn serve(statuses: Vec<(u16, &'static str)>) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();

        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let n = counter.fetch_add(1, Ordering::SeqCst);
                let (status, body) = statuses[n.min(statuses.len() - 1)];
                let mut buf = [0u8; 2048];
                let _ = socket.read(&mut buf).await;
                let response = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });

        (url, hits)
    }

    fn client(url: String, max_times: usize) -> BeaconClient {
        BeaconClient::try_with_client(
            Client::new(),
            Config {
                base_url: url,
                exp_backoff: Some(
                    ExponentialBuilder::default()
                        .with_min_delay(Duration::from_millis(5))
                        .with_max_times(max_times),
                ),
            },
        )
        .unwrap()
    }

    #[tokio::test]
    async fn retries_429_and_5xx_until_success() {
        let (url, hits) = serve(vec![
            (429, r#"{"message":"slow down"}"#),
            (503, "<html>unavailable</html>"),
            (200, HEADER_JSON),
        ])
        .await;

        let header = client(url, 4)
            .get_block_header(BlockId::Slot(7))
            .await
            .unwrap()
            .unwrap();

        assert_eq!(header.slot, 7);
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn gives_up_after_the_configured_retries_and_reports_the_status() {
        let (url, hits) = serve(vec![(502, "bad gateway")]).await;

        let error = client(url, 2)
            .get_block_header(BlockId::Slot(7))
            .await
            .unwrap_err();

        assert_eq!(hits.load(Ordering::SeqCst), 3);
        assert!(error.to_string().contains("502"), "{error}");
    }

    #[tokio::test]
    async fn does_not_retry_a_404() {
        let (url, hits) = serve(vec![(404, r#"{"code":404,"message":"not found"}"#)]).await;

        let header = client(url, 4)
            .get_block_header(BlockId::Slot(7))
            .await
            .unwrap();

        assert!(header.is_none());
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }
}
