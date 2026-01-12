use anyhow::Result;
use std::path::Path;
use std::process::Command;
use tracing::{debug, warn};

/// SQLite WAL 文件清理器
///
/// `grammers-session` 库使用 SQLite 的 WAL (Write-Ahead Log) 模式，
/// 会生成 `-wal` 和 `-shm` 文件。
///
/// 此模块提供定期 checkpoint 功能，将 WAL 内容写回主数据库以控制 WAL 增长。
pub struct SessionCleaner {
    session_path: std::path::PathBuf,
}

impl SessionCleaner {
    /// 创建新的 session 清理器
    ///
    /// # 参数
    /// * `session_path`: session 文件路径（与 grammers-session 使用的路径一致）
    ///
    /// # 示例
    /// ```
    /// use anyhow::Result;
    /// use storage::SessionCleaner;
    ///
    /// # fn main() -> Result<()> {
    /// let cleaner = SessionCleaner::new("data/user_session");
    /// cleaner.checkpoint()?;  // 执行一次 checkpoint
    /// # Ok(())
    /// # }
    /// ```
    pub fn new<P: AsRef<Path>>(session_path: P) -> Self {
        Self {
            session_path: session_path.as_ref().to_path_buf(),
        }
    }

    /// 执行 WAL checkpoint，将 WAL 内容写回主数据库
    ///
    /// 使用 TRUNCATE 模式，尝试收缩 WAL 文件
    ///
    /// # 返回
    /// * `Ok(true)`: checkpoint 成功执行
    /// * `Ok(false)`: session 文件不存在或不是数据库
    /// * `Err(_)`: checkpoint 执行失败
    pub fn checkpoint(&self) -> Result<bool> {
        let db_path = self.session_path.clone();
        if !db_path.exists() {
            return Ok(false);
        }

        // 检查是否是有效的 SQLite 数据库
        if !self.is_sqlite_database(&db_path)? {
            return Ok(false);
        }

        debug!("执行 SQLite WAL checkpoint: {:?}", db_path);

        // 尝试使用 sqlite3 命令行工具执行 checkpoint
        // 这避免了 rusqlite 与 grammers-session 的依赖冲突
        let result = Command::new("sqlite3")
            .arg(&db_path)
            .arg("PRAGMA wal_checkpoint(TRUNCATE);")
            .output();

        match result {
            Ok(output) => {
                if output.status.success() {
                    debug!("SQLite WAL checkpoint 成功");
                    Ok(true)
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let detail = if !stderr.trim().is_empty() {
                        stderr.trim()
                    } else if !stdout.trim().is_empty() {
                        stdout.trim()
                    } else {
                        "unknown"
                    };
                    warn!(
                        "SQLite WAL checkpoint 失败: status={} detail={}",
                        output.status, detail
                    );
                    Ok(false)
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                warn!("sqlite3 命令不可用，跳过 WAL checkpoint");
                Ok(false)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// 定期执行 checkpoint 的异步任务
    ///
    /// # 参数
    /// * `interval_sec`: 检查间隔（秒）
    /// * `shutdown`: 关闭信号
    ///
    /// # 使用方法
    /// ```no_run
    /// use storage::SessionCleaner;
    /// use std::sync::{Arc, atomic::AtomicBool};
    ///
    /// let cleaner = SessionCleaner::new("data/user_session");
    /// let shutdown = Arc::new(AtomicBool::new(false));
    /// tokio::spawn(cleaner.periodic_checkpoint(300, shutdown));
    /// ```
    pub fn periodic_checkpoint(
        &self,
        interval_sec: u64,
        shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> impl std::future::Future<Output = ()> + Send + 'static {
        let cleaner = self.clone();
        let interval = tokio::time::Duration::from_secs(interval_sec);

        async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.tick().await; // 跳过第一次立即执行

            loop {
                ticker.tick().await;

                if shutdown.load(std::sync::atomic::Ordering::Relaxed) {
                    debug!("Session 清理器收到关闭信号");
                    break;
                }

                match cleaner.checkpoint() {
                    Ok(true) => {
                        debug!("定期 WAL checkpoint 成功");
                    }
                    Ok(false) => {
                        // session 文件不存在或不是数据库，忽略
                    }
                    Err(e) => {
                        warn!("定期 WAL checkpoint 失败: {}", e);
                    }
                }
            }

            // 退出前执行最后一次 checkpoint
            if let Err(e) = cleaner.checkpoint() {
                warn!("退出时 WAL checkpoint 失败: {}", e);
            }
        }
    }

    /// 检查文件是否是有效的 SQLite 数据库
    fn is_sqlite_database(&self, path: &Path) -> Result<bool> {
        // SQLite 数据库文件头是 "SQLite format 3\0"
        let mut file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.into()),
        };
        let mut header = [0u8; 16];
        use std::io::Read;
        if let Err(e) = file.read_exact(&mut header) {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                return Ok(false);
            }
            return Err(e.into());
        }

        Ok(&header == b"SQLite format 3\0")
    }
}

impl Clone for SessionCleaner {
    fn clone(&self) -> Self {
        Self::new(&self.session_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_sqlite_database() {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temp_path = std::env::temp_dir().join(format!(
            "session_cleaner_test_{}_{}",
            std::process::id(),
            suffix
        ));
        let cleaner = SessionCleaner::new(&temp_path);
        // 测试不存在的文件
        assert!(!cleaner.is_sqlite_database(&temp_path).unwrap());
    }
}
