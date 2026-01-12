use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

type HotReloadCallback = Arc<dyn Fn() -> anyhow::Result<()> + Send + Sync>;
type HotReloadEntry = (SystemTime, HotReloadCallback);
type HotReloadFiles = HashMap<String, HotReloadEntry>;

#[derive(Clone)]
pub struct HotReloadManager {
    files: Arc<RwLock<HotReloadFiles>>,
}

impl HotReloadManager {
    pub fn new() -> Self {
        Self {
            files: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn register<F>(&self, path: &str, callback: F)
    where
        F: Fn() -> anyhow::Result<()> + Send + Sync + 'static,
    {
        let mtime = Self::get_mtime(path).unwrap_or(UNIX_EPOCH);
        let mut files = self.files.write().await;
        files.insert(path.to_string(), (mtime, Arc::new(callback)));
    }

    pub async fn check_and_reload(&self) {
        let mut files = self.files.write().await;

        let mut to_reload = Vec::new();
        for (path, (last_mtime, _callback)) in files.iter() {
            if let Ok(current_mtime) = Self::get_mtime(path) {
                if current_mtime != *last_mtime {
                    to_reload.push(path.clone());
                }
            }
        }

        for path in to_reload {
            if let Some((_, callback)) = files.get(&path).cloned() {
                match callback() {
                    Ok(_) => {
                        if let Ok(new_mtime) = Self::get_mtime(&path) {
                            files.insert(path.clone(), (new_mtime, callback.clone()));
                            tracing::info!("检测到文件变更，已重新加载: {}", path);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("重新加载文件失败: {} error: {}", path, e);
                    }
                }
            }
        }
    }

    fn get_mtime(path: &str) -> anyhow::Result<SystemTime> {
        let metadata = std::fs::metadata(path)?;
        Ok(metadata.modified()?)
    }
}

impl Default for HotReloadManager {
    fn default() -> Self {
        Self::new()
    }
}
