use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SourceState {
    last_committed: i64,
    pending: BTreeSet<i64>,
}

impl SourceState {
    fn new() -> Self {
        Self {
            last_committed: 0,
            pending: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct GapRuntime {
    gap_since: Option<Instant>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateStore {
    last_committed_by_source: HashMap<String, SourceState>,
    last_committed_default: SourceState,
    #[serde(skip)]
    gap_runtime: HashMap<String, GapRuntime>,
    #[serde(skip)]
    pending_limit: usize,
    #[serde(skip)]
    gap_timeout: Duration,
    #[serde(skip)]
    dirty: bool,
}

impl StateStore {
    pub fn new() -> Self {
        Self {
            last_committed_by_source: HashMap::new(),
            last_committed_default: SourceState::new(),
            gap_runtime: HashMap::new(),
            pending_limit: 2000,
            gap_timeout: Duration::from_secs(300),
            dirty: false,
        }
    }

    pub fn set_gap_config(&mut self, pending_limit: usize, gap_timeout: u64) {
        if pending_limit > 0 {
            self.pending_limit = pending_limit;
        }
        if gap_timeout > 0 {
            self.gap_timeout = Duration::from_secs(gap_timeout);
        }
    }

    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::new());
        }

        let content = std::fs::read_to_string(path)
            .with_context(|| format!("无法读取状态文件: {:?}", path))?;
        if content.trim().is_empty() {
            tracing::warn!("状态文件为空，已重置: {:?}", path);
            return Ok(Self::new());
        }

        let data: serde_json::Value = serde_json::from_str(&content)?;
        let mut store = Self::new();

        if let Some(last_committed) = data.get("last_committed_id") {
            store.load_last_committed(last_committed)?;
            if let Some(pending) = data.get("pending_ids") {
                store.load_pending(pending)?;
            }
            if let Some(pending_default) = data.get("pending_ids_default") {
                store.load_pending_default(pending_default)?;
            }
            return Ok(store);
        }

        if let Some(last_message_id) = data.get("last_message_id") {
            store.load_last_committed(last_message_id)?;
        }

        Ok(store)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let path_ref = path.as_ref();
        let last_committed_id = if self.last_committed_by_source.is_empty() {
            serde_json::Value::Number(serde_json::Number::from(
                self.last_committed_default.last_committed,
            ))
        } else {
            serde_json::to_value(
                self.last_committed_by_source
                    .iter()
                    .map(|(k, v)| (k.clone(), v.last_committed))
                    .collect::<HashMap<_, _>>(),
            )?
        };

        let pending_ids = serde_json::to_value(
            self.last_committed_by_source
                .iter()
                .map(|(k, v)| (k.clone(), v.pending.iter().copied().collect::<Vec<_>>()))
                .collect::<HashMap<_, _>>(),
        )?;
        let pending_ids_default = self
            .last_committed_default
            .pending
            .iter()
            .copied()
            .collect::<Vec<_>>();

        let data = serde_json::json!({
            "last_committed_id": last_committed_id,
            "pending_ids": pending_ids,
            "pending_ids_default": pending_ids_default,
        });

        let content = serde_json::to_string_pretty(&data)?;
        std::fs::write(path_ref, content)
            .with_context(|| format!("无法写入状态文件: {:?}", path_ref))?;

        Ok(())
    }

    pub fn get_min_id(&self, source_key: &str) -> i64 {
        let key = normalize_source_key(source_key);
        self.last_committed_by_source
            .get(&key)
            .map(|s| s.last_committed)
            .unwrap_or(self.last_committed_default.last_committed)
    }

    pub fn migrate_source_key(&mut self, from: &str, to: &str) {
        let from_key = normalize_source_key(from);
        let to_key = normalize_source_key(to);
        if from_key == to_key {
            return;
        }

        let Some(from_state) = self.last_committed_by_source.remove(&from_key) else {
            return;
        };

        let target = self
            .last_committed_by_source
            .entry(to_key.clone())
            .or_insert_with(SourceState::new);

        if from_state.last_committed > target.last_committed {
            target.last_committed = from_state.last_committed;
        }

        for id in from_state.pending {
            if id > target.last_committed {
                target.pending.insert(id);
            }
        }

        self.gap_runtime.remove(&from_key);
        self.dirty = true;
    }

    pub fn is_processed(&self, source_key: Option<&str>, msg_id: i64) -> bool {
        let key = source_key.map(normalize_source_key);
        let state = match key {
            Some(ref k) => self
                .last_committed_by_source
                .get(k)
                .unwrap_or(&self.last_committed_default),
            None => &self.last_committed_default,
        };

        if msg_id <= state.last_committed {
            return true;
        }

        state.pending.contains(&msg_id)
    }

    pub fn mark_processed(&mut self, source_key: Option<&str>, msg_id: i64) {
        let gap_timeout = self.gap_timeout;
        self.mark_processed_with_timeout(source_key, msg_id, gap_timeout);
    }

    pub fn mark_processed_with_timeout(
        &mut self,
        source_key: Option<&str>,
        msg_id: i64,
        gap_timeout: Duration,
    ) {
        let key = source_key.map(normalize_source_key);
        let runtime_key = key.clone().unwrap_or_else(|| "__default__".to_string());
        let StateStore {
            last_committed_by_source,
            last_committed_default,
            gap_runtime,
            pending_limit,
            dirty,
            ..
        } = self;

        let state = match key {
            Some(ref k) => last_committed_by_source
                .entry(k.clone())
                .or_insert_with(SourceState::new),
            None => last_committed_default,
        };

        if msg_id <= state.last_committed {
            return;
        }

        state.pending.insert(msg_id);
        *dirty = true;

        if state.pending.len() > *pending_limit {
            if let Some(min_pending) = state.pending.iter().next().copied() {
                let old_committed = state.last_committed;
                state.last_committed = min_pending.saturating_sub(1);
                if state.last_committed > old_committed {
                    tracing::warn!(
                        "pending 集合溢出，跳过 {} 条消息: {}..{} (limit={})",
                        state.last_committed - old_committed,
                        old_committed + 1,
                        state.last_committed,
                        pending_limit
                    );
                }
                gap_runtime.remove(&runtime_key);
                *dirty = true;
            }
        }

        let runtime = gap_runtime.entry(runtime_key).or_default();
        Self::advance_commit(runtime, state, gap_timeout, dirty);
    }

    pub fn update(&mut self, source_key: Option<&str>, msg_id: i64) {
        self.mark_processed(source_key, msg_id);
    }

    pub fn flush<P: AsRef<Path>>(&mut self, path: P) -> Result<bool> {
        if self.dirty {
            self.save(path)?;
            self.dirty = false;
            return Ok(true);
        }
        Ok(false)
    }

    fn advance_commit(
        runtime: &mut GapRuntime,
        state: &mut SourceState,
        gap_timeout: Duration,
        dirty: &mut bool,
    ) {
        loop {
            let next = state.last_committed + 1;
            if state.pending.remove(&next) {
                state.last_committed = next;
                runtime.gap_since = None;
                *dirty = true;
                continue;
            }

            if state.pending.is_empty() {
                runtime.gap_since = None;
                break;
            }

            let min_pending = match state.pending.iter().next().copied() {
                Some(v) => v,
                None => break,
            };

            if min_pending <= state.last_committed {
                state.pending.remove(&min_pending);
                *dirty = true;
                continue;
            }

            let now = Instant::now();
            if runtime.gap_since.is_none() {
                runtime.gap_since = Some(now);
            }
            let gap_since = runtime.gap_since.unwrap_or(now);
            if now.duration_since(gap_since) >= gap_timeout {
                state.last_committed = min_pending.saturating_sub(1);
                runtime.gap_since = None;
                *dirty = true;
                continue;
            }
            break;
        }
    }

    fn load_last_committed(&mut self, value: &serde_json::Value) -> Result<()> {
        if value.is_object() {
            let raw_map: HashMap<String, i64> = serde_json::from_value(value.clone())?;
            for (raw_key, id) in raw_map {
                let key = normalize_source_key(&raw_key);
                let entry = self
                    .last_committed_by_source
                    .entry(key)
                    .or_insert_with(SourceState::new);
                if id > entry.last_committed {
                    entry.last_committed = id;
                }
            }
        } else {
            let default_id = value.as_i64().unwrap_or(0);
            self.last_committed_default.last_committed = default_id;
        }

        Ok(())
    }

    fn load_pending(&mut self, value: &serde_json::Value) -> Result<()> {
        if !value.is_object() {
            return Ok(());
        }

        let raw_map: HashMap<String, Vec<i64>> = serde_json::from_value(value.clone())?;
        for (raw_key, ids) in raw_map {
            let key = normalize_source_key(&raw_key);
            let entry = self
                .last_committed_by_source
                .entry(key)
                .or_insert_with(SourceState::new);

            for id in ids {
                if id > entry.last_committed {
                    entry.pending.insert(id);
                }
            }
        }

        Ok(())
    }

    fn load_pending_default(&mut self, value: &serde_json::Value) -> Result<()> {
        if !value.is_array() {
            return Ok(());
        }
        let ids: Vec<i64> = serde_json::from_value(value.clone())?;
        for id in ids {
            if id > self.last_committed_default.last_committed {
                self.last_committed_default.pending.insert(id);
            }
        }
        Ok(())
    }
}

impl Default for StateStore {
    fn default() -> Self {
        Self::new()
    }
}

fn normalize_source_key(raw: &str) -> String {
    let trimmed = raw.trim();
    let without_scheme = trimmed
        .strip_prefix("https://t.me/")
        .or_else(|| trimmed.strip_prefix("http://t.me/"))
        .or_else(|| trimmed.strip_prefix("t.me/"))
        .unwrap_or(trimmed);
    without_scheme.trim_start_matches('@').to_string()
}
