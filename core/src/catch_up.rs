use super::model::MessageView;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub type CatchUpFuture = Pin<Box<dyn Future<Output = anyhow::Result<Vec<MessageView>>> + Send>>;

pub trait CatchUpFetcher: Send + Sync {
    fn fetch(&self, source: &str, last_id: i64) -> CatchUpFuture;
}

#[derive(Clone)]
pub struct CatchUpManager {
    fetcher: Arc<dyn CatchUpFetcher>,
}

impl CatchUpManager {
    pub fn new(fetcher: Arc<dyn CatchUpFetcher>) -> Self {
        Self { fetcher }
    }

    pub async fn fetch_missed_messages(
        &self,
        source: &str,
        last_id: i64,
    ) -> anyhow::Result<Vec<MessageView>> {
        self.fetcher.fetch(source, last_id).await
    }
}
