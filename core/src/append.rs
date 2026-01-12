use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentAppender {
    pub header: String,
    pub footer: String,
}

impl ContentAppender {
    pub fn new() -> Self {
        Self {
            header: String::new(),
            footer: String::new(),
        }
    }

    pub fn load_from_file<P: AsRef<Path>>(&mut self, path: P) -> anyhow::Result<()> {
        let path = path.as_ref();
        if !path.exists() {
            self.header = String::new();
            self.footer = String::new();
            return Ok(());
        }

        let content = std::fs::read_to_string(path)?;
        let data: serde_json::Value = serde_json::from_str(&content)?;

        self.header = data
            .get("header")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        self.footer = data
            .get("footer")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        tracing::info!(
            "内容追加配置已加载: header_len={} footer_len={}",
            self.header.len(),
            self.footer.len()
        );

        Ok(())
    }
}

impl Default for ContentAppender {
    fn default() -> Self {
        Self::new()
    }
}
