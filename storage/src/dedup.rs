use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupStore {
    version: i32,
    fingerprints: VecDeque<u64>,
    media_id_signatures: VecDeque<String>,
    media_meta_signatures: VecDeque<String>,
    media_thumb_hashes: VecDeque<(u64, u64)>,
    recent_text_features: VecDeque<Vec<u64>>,
    #[serde(skip)]
    media_id_signature_set: HashSet<String>,
    #[serde(skip)]
    media_meta_signature_set: HashSet<String>,
    #[serde(skip)]
    dirty: bool,
}

impl DedupStore {
    pub fn new(limit: usize, media_limit: usize) -> Self {
        Self {
            version: 6,
            fingerprints: VecDeque::with_capacity(limit),
            media_id_signatures: VecDeque::with_capacity(media_limit),
            media_meta_signatures: VecDeque::with_capacity(media_limit),
            media_thumb_hashes: VecDeque::with_capacity(media_limit),
            recent_text_features: VecDeque::with_capacity(limit),
            media_id_signature_set: HashSet::new(),
            media_meta_signature_set: HashSet::new(),
            dirty: false,
        }
    }

    pub fn load<P: AsRef<Path>>(path: P, limit: usize, media_limit: usize) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::new(limit, media_limit));
        }

        let content = std::fs::read_to_string(path)
            .with_context(|| format!("无法读取去重文件: {:?}", path))?;
        if content.trim().is_empty() {
            tracing::warn!("去重文件为空，已重置: {:?}", path);
            return Ok(Self::new(limit, media_limit));
        }

        let data: serde_json::Value = serde_json::from_str(&content)?;

        let version = data.get("version").and_then(|v| v.as_i64()).unwrap_or(0);
        if version != 5 && version != 6 {
            anyhow::bail!(
                "去重记录版本不匹配: 当前版本={}, 支持版本=5或6\n\
                这可能导致大量重复消息被转发。\n\
                请备份现有去重文件后删除它，或手动迁移数据格式。\n\
                去重文件路径: {:?}",
                version,
                path
            );
        }

        let fingerprints: VecDeque<u64> = data
            .get("fingerprints")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let media_id_signatures: VecDeque<String> = data
            .get("media_id_signatures")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let media_meta_signatures: VecDeque<String> = data
            .get("media_meta_signatures")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let media_thumb_hashes: VecDeque<(u64, u64)> = data
            .get("media_thumb_hashes")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let recent_text_features: VecDeque<Vec<u64>> = data
            .get("recent_text_features")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let mut store = Self::new(limit, media_limit);
        store.fingerprints = fingerprints;
        store.recent_text_features = recent_text_features;
        store.media_thumb_hashes = media_thumb_hashes;

        for sig in media_id_signatures.iter().rev().take(media_limit) {
            if store.media_id_signature_set.contains(sig) {
                continue;
            }
            store.media_id_signatures.push_front(sig.clone());
            store.media_id_signature_set.insert(sig.clone());
        }

        for sig in media_meta_signatures.iter().rev().take(media_limit) {
            if store.media_meta_signature_set.contains(sig) {
                continue;
            }
            store.media_meta_signatures.push_front(sig.clone());
            store.media_meta_signature_set.insert(sig.clone());
        }

        Ok(store)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let path_ref = path.as_ref();
        let data = serde_json::json!({
            "version": self.version,
            "fingerprints": Vec::<u64>::from(self.fingerprints.clone()),
            "media_id_signatures": Vec::<String>::from(self.media_id_signatures.clone()),
            "media_meta_signatures": Vec::<String>::from(self.media_meta_signatures.clone()),
            "media_thumb_hashes": Vec::<(u64, u64)>::from(self.media_thumb_hashes.clone()),
            "recent_text_features": Vec::<Vec<u64>>::from(self.recent_text_features.clone()),
        });

        let content = serde_json::to_string_pretty(&data)?;
        std::fs::write(path_ref, content)
            .with_context(|| format!("无法写入去重文件: {:?}", path_ref))?;

        Ok(())
    }

    pub fn add_fingerprint(&mut self, fp: u64) {
        self.fingerprints.push_back(fp);
        self.dirty = true;
    }

    pub fn contains_fingerprint(&self, fp: u64) -> bool {
        self.fingerprints.contains(&fp)
    }

    pub fn hamming_distance(&self, a: u64, b: u64, threshold: usize) -> bool {
        let x = a ^ b;
        let mut count = 0;
        let mut n = x;
        while n > 0 && count <= threshold {
            count += 1;
            n &= n - 1;
        }
        count <= threshold
    }

    fn record_signature(
        storage: &mut VecDeque<String>,
        set: &mut HashSet<String>,
        sig: String,
        limit: usize,
    ) -> bool {
        if set.contains(&sig) {
            return false;
        }
        if storage.len() >= limit {
            if let Some(removed) = storage.pop_front() {
                set.remove(&removed);
            }
        }
        storage.push_back(sig.clone());
        set.insert(sig);
        true
    }

    pub fn contains_media_id(&self, sig: &str) -> bool {
        self.media_id_signature_set.contains(sig)
    }

    pub fn contains_media_meta(&self, sig: &str) -> bool {
        self.media_meta_signature_set.contains(sig)
    }

    pub fn add_media_id(&mut self, sig: String, limit: usize) {
        if Self::record_signature(
            &mut self.media_id_signatures,
            &mut self.media_id_signature_set,
            sig,
            limit,
        ) {
            self.dirty = true;
        }
    }

    pub fn add_media_meta(&mut self, sig: String, limit: usize) {
        if Self::record_signature(
            &mut self.media_meta_signatures,
            &mut self.media_meta_signature_set,
            sig,
            limit,
        ) {
            self.dirty = true;
        }
    }

    pub fn flush<P: AsRef<Path>>(&mut self, path: P) -> Result<bool> {
        if self.dirty {
            self.save(path)?;
            self.dirty = false;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn fingerprints(&self) -> VecDeque<u64> {
        self.fingerprints.clone()
    }

    pub fn media_id_signatures(&self) -> VecDeque<String> {
        self.media_id_signatures.clone()
    }

    pub fn media_meta_signatures(&self) -> VecDeque<String> {
        self.media_meta_signatures.clone()
    }

    pub fn media_thumb_hashes(&self) -> VecDeque<(u64, u64)> {
        self.media_thumb_hashes.clone()
    }

    pub fn recent_text_features(&self) -> VecDeque<Vec<u64>> {
        self.recent_text_features.clone()
    }

    pub fn set_fingerprints(&mut self, data: VecDeque<u64>) {
        if self.fingerprints != data {
            self.fingerprints = data;
            self.dirty = true;
        }
    }

    pub fn set_media_id_signatures(&mut self, data: VecDeque<String>) {
        if self.media_id_signatures != data {
            self.media_id_signatures = data.clone();
            self.media_id_signature_set = data.iter().cloned().collect();
            self.dirty = true;
        }
    }

    pub fn set_media_meta_signatures(&mut self, data: VecDeque<String>) {
        if self.media_meta_signatures != data {
            self.media_meta_signatures = data.clone();
            self.media_meta_signature_set = data.iter().cloned().collect();
            self.dirty = true;
        }
    }

    pub fn set_media_thumb_hashes(&mut self, data: VecDeque<(u64, u64)>) {
        if self.media_thumb_hashes != data {
            self.media_thumb_hashes = data;
            self.dirty = true;
        }
    }

    pub fn set_recent_text_features(&mut self, data: VecDeque<Vec<u64>>) {
        if self.recent_text_features != data {
            self.recent_text_features = data;
            self.dirty = true;
        }
    }
}
