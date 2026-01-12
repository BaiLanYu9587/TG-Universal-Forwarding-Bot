use super::model::{MessageView, TextEntity};

#[derive(Debug, Clone)]
pub struct TextMergeCache {
    pub message: MessageView,
    pub expires_at: std::time::Instant,
}

pub struct TextMergeManager {
    cache: std::collections::HashMap<String, TextMergeCache>,
    merge_window: f64,
    min_len: usize,
    max_id_gap: usize,
}

impl TextMergeManager {
    pub fn new(merge_window: f64, min_len: usize, max_id_gap: usize) -> Self {
        Self {
            cache: std::collections::HashMap::new(),
            merge_window,
            min_len,
            max_id_gap,
        }
    }

    pub fn try_merge_for_album(
        &mut self,
        source_key: &str,
        min_album_id: i64,
    ) -> Option<(String, Vec<TextEntity>)> {
        if self.merge_window <= 0.0 {
            return None;
        }

        let cached = self.cache.get(source_key)?.clone();

        if std::time::Instant::now() > cached.expires_at {
            self.cache.remove(source_key);
            return None;
        }

        let gap = min_album_id - cached.message.id;
        if gap <= 0 || gap > self.max_id_gap as i64 {
            return None;
        }

        let text = cached.message.text.trim().to_string();
        if text.len() < self.min_len {
            return None;
        }

        let result = (text.clone(), cached.message.entities.clone());
        self.cache.remove(source_key);

        tracing::info!(
            "已合并文本到相册: source={} text_id={} album_min_id={} gap={} len={}",
            source_key,
            cached.message.id,
            min_album_id,
            gap,
            text.len()
        );

        Some(result)
    }

    pub fn cache_text(&mut self, source_key: &str, message: &MessageView) -> bool {
        if self.merge_window <= 0.0 {
            return false;
        }

        let text = message.text.trim();
        if text.is_empty() || text.len() < self.min_len {
            return false;
        }

        let mut cached_message = message.clone();
        cached_message.is_edit = false;
        let cache = TextMergeCache {
            message: cached_message,
            expires_at: std::time::Instant::now()
                + std::time::Duration::from_secs_f64(self.merge_window),
        };

        self.cache.insert(source_key.to_string(), cache);

        tracing::info!(
            "已缓存待合并文本: source={} text_id={} len={} window={}",
            source_key,
            message.id,
            text.len(),
            self.merge_window
        );

        true
    }

    pub fn update_cached_text(&mut self, source_key: &str, message: &MessageView) -> bool {
        if self.merge_window <= 0.0 {
            return false;
        }

        let cache = match self.cache.get_mut(source_key) {
            Some(cache) => cache,
            None => return false,
        };

        if cache.message.id != message.id {
            return false;
        }

        let text = message.text.trim();
        if text.is_empty() || text.len() < self.min_len {
            return false;
        }

        let mut updated = message.clone();
        updated.is_edit = false;
        cache.message = updated;
        cache.expires_at =
            std::time::Instant::now() + std::time::Duration::from_secs_f64(self.merge_window);

        tracing::info!(
            "已更新待合并文本: source={} text_id={} len={} window={}",
            source_key,
            message.id,
            text.len(),
            self.merge_window
        );

        true
    }

    pub fn get_expired_keys(&self) -> Vec<String> {
        if self.merge_window <= 0.0 {
            return Vec::new();
        }

        let now = std::time::Instant::now();
        self.cache
            .iter()
            .filter(|(_, cache)| cache.expires_at < now)
            .map(|(key, _)| key.clone())
            .collect()
    }

    pub fn remove_message(&mut self, source_key: &str) -> Option<MessageView> {
        self.cache.remove(source_key).map(|c| c.message)
    }

    pub fn remove_message_if_expired(&mut self, source_key: &str) -> Option<MessageView> {
        if self.merge_window <= 0.0 {
            return None;
        }

        let now = std::time::Instant::now();
        let expired = match self.cache.get(source_key) {
            Some(cache) => cache.expires_at < now,
            None => false,
        };
        if !expired {
            return None;
        }

        self.cache.remove(source_key).map(|c| c.message)
    }

    pub fn extend_expiration(&mut self, source_key: &str, duration: std::time::Duration) {
        if let Some(entry) = self.cache.get_mut(source_key) {
            entry.expires_at = std::time::Instant::now() + duration;
            tracing::debug!(
                "文本缓存延长: source={} text_id={} extend={:?}",
                source_key,
                entry.message.id,
                duration
            );
        }
    }

    pub fn extend_expiration_if_expired(
        &mut self,
        source_key: &str,
        duration: std::time::Duration,
    ) -> bool {
        if self.merge_window <= 0.0 {
            return false;
        }

        let now = std::time::Instant::now();
        let entry = match self.cache.get_mut(source_key) {
            Some(entry) => entry,
            None => return false,
        };
        if entry.expires_at >= now {
            return false;
        }

        entry.expires_at = now + duration;
        tracing::debug!(
            "文本缓存延长(过期后): source={} text_id={} extend={:?}",
            source_key,
            entry.message.id,
            duration
        );
        true
    }

    pub fn take_expired(&mut self) -> Vec<(String, MessageView)> {
        if self.merge_window <= 0.0 {
            return Vec::new();
        }

        let now = std::time::Instant::now();
        let expired: Vec<String> = self
            .cache
            .iter()
            .filter(|(_, cache)| cache.expires_at < now)
            .map(|(key, _)| key.clone())
            .collect();

        let mut result = Vec::new();
        for key in &expired {
            if let Some(cache) = self.cache.remove(key) {
                result.push((key.clone(), cache.message));
            }
        }

        if !expired.is_empty() {
            tracing::info!("文本缓存过期待发送: {} 条", expired.len());
        }

        result
    }

    pub fn take_all(&mut self) -> Vec<(String, MessageView)> {
        if self.merge_window <= 0.0 || self.cache.is_empty() {
            return Vec::new();
        }

        let mut result = Vec::with_capacity(self.cache.len());
        for (key, cache) in self.cache.drain() {
            result.push((key, cache.message));
        }

        if !result.is_empty() {
            tracing::info!("文本缓存强制发送: {} 条", result.len());
        }

        result
    }

    pub fn flush_expired(&mut self) {
        let _ = self.take_expired();
    }
}

impl Default for TextMergeManager {
    fn default() -> Self {
        Self::new(0.0, 0, 5)
    }
}
