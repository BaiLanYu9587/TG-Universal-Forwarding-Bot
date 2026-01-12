use super::model::MessageView;
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::debug;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AlbumKey {
    pub source_key: String,
    pub album_id: i64,
}

pub struct AlbumAggregator {
    cache: RwLock<HashMap<AlbumKey, AlbumCache>>,
    max_items: usize,
}

#[derive(Debug, Clone)]
struct AlbumCache {
    messages: BTreeMap<i64, MessageView>,
    last_update: Instant,
}

impl AlbumAggregator {
    pub fn new(max_items: usize) -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            max_items,
        }
    }

    pub async fn update_if_cached(&self, message: MessageView, source_key: &str) -> bool {
        let album_id = match message.album_id {
            Some(id) => id,
            None => return false,
        };

        let key = AlbumKey {
            source_key: source_key.to_string(),
            album_id,
        };
        let mut cache = self.cache.write().await;
        let entry = match cache.get_mut(&key) {
            Some(entry) => entry,
            None => return false,
        };

        entry.messages.insert(message.id, message);
        entry.last_update = Instant::now();
        true
    }

    pub async fn add_message(&self, message: MessageView, source_key: &str) -> AlbumAction {
        debug!(
            "AlbumAggregator::add_message entry: source={} msg_id={}",
            source_key, message.id
        );
        let album_id = match message.album_id {
            Some(id) => id,
            None => return AlbumAction::NotAlbum,
        };

        let key = AlbumKey {
            source_key: source_key.to_string(),
            album_id,
        };
        let mut cache = self.cache.write().await;

        let entry = cache.entry(key.clone()).or_insert_with(|| AlbumCache {
            messages: BTreeMap::new(),
            last_update: Instant::now(),
        });

        // 如果消息ID已存在，需要决定是保留现有版本还是更新为新版本
        if let Some(existing) = entry.messages.get(&message.id) {
            let existing_text = existing.text.trim();
            let incoming_text = message.text.trim();

            // 决策逻辑：优先保留更完整/更新的版本
            // 1. 如果现有是编辑版本，新来的不是编辑版本 → 保留现有
            //    （可能是乱序接收：先收到编辑事件，后收到原始消息）
            // 2. 如果现有不是编辑版本，新来的是编辑版本 → 更新为新版本
            //    （正常流程：用编辑后的内容替换原始内容）
            // 3. 如果现有文本为空，新文本非空 → 更新为新版本
            //    （补充缺失的说明文字）
            // 4. 如果现有文本非空，新文本为空 → 保留现有
            //    （避免丢失已有的说明文字）
            // 5. 其他情况 → 保留文本更长的版本
            //    （认为更长的文本包含更多信息）
            let keep_existing = if existing.is_edit && !message.is_edit {
                true
            } else if (!existing.is_edit && message.is_edit)
                || (existing_text.is_empty() && !incoming_text.is_empty())
            {
                false
            } else if !existing_text.is_empty() && incoming_text.is_empty() {
                true
            } else {
                existing_text.chars().count() >= incoming_text.chars().count()
            };

            if !keep_existing {
                entry.messages.insert(message.id, message);
            }
            entry.last_update = Instant::now();
            return AlbumAction::Cached;
        }

        entry.messages.insert(message.id, message);
        entry.last_update = Instant::now();
        if entry.messages.len() >= self.max_items {
            let messages = entry.messages.values().cloned().collect();
            cache.remove(&key);
            AlbumAction::Flush(messages)
        } else {
            AlbumAction::Cached
        }
    }

    pub async fn expired_keys(&self, delay: Duration) -> Vec<AlbumKey> {
        let cache = self.cache.read().await;
        cache
            .iter()
            .filter(|(_, entry)| entry.last_update.elapsed() >= delay)
            .map(|(key, _)| key.clone())
            .collect()
    }

    pub async fn flush(&self, key: &AlbumKey) -> Option<Vec<MessageView>> {
        let mut cache = self.cache.write().await;
        cache
            .remove(key)
            .map(|entry| entry.messages.values().cloned().collect())
    }

    pub async fn get(&self, key: &AlbumKey) -> Option<Vec<MessageView>> {
        let cache = self.cache.read().await;
        cache
            .get(key)
            .map(|entry| entry.messages.values().cloned().collect())
    }

    pub async fn has_pending_album(&self, source_key: &str) -> bool {
        let cache = self.cache.read().await;
        cache.keys().any(|k| k.source_key == source_key)
    }

    pub async fn drain_all(&self) -> Vec<(AlbumKey, Vec<MessageView>)> {
        let mut cache = self.cache.write().await;
        let mut result = Vec::new();
        for (key, entry) in cache.drain() {
            result.push((key, entry.messages.values().cloned().collect()));
        }
        result
    }
}

#[derive(Debug, Clone)]
pub enum AlbumAction {
    NotAlbum,
    Cached,
    Duplicate,
    Flush(Vec<MessageView>),
}
