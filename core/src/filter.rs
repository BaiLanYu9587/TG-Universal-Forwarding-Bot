use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeywordFilter {
    whitelist: Vec<String>,
    blacklist: Vec<String>,
    normalized_whitelist: Vec<String>,
    normalized_blacklist: Vec<String>,
    case_sensitive: bool,
}

impl KeywordFilter {
    pub fn new(case_sensitive: bool) -> Self {
        Self {
            whitelist: Vec::new(),
            blacklist: Vec::new(),
            normalized_whitelist: Vec::new(),
            normalized_blacklist: Vec::new(),
            case_sensitive,
        }
    }

    pub fn load_from_file<P: AsRef<Path>>(&mut self, path: P) -> anyhow::Result<()> {
        let path = path.as_ref();
        if !path.exists() {
            tracing::warn!("关键词文件不存在: {:?}", path);
            self.whitelist.clear();
            self.blacklist.clear();
            self.normalized_whitelist.clear();
            self.normalized_blacklist.clear();
            return Ok(());
        }

        let content = std::fs::read_to_string(path)?;

        let mut whitelist = Vec::new();
        let mut blacklist = Vec::new();
        let mut mode = Mode::Whitelist;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let lower = line.to_lowercase();
            if lower == "[whitelist]" || lower == "[white]" {
                mode = Mode::Whitelist;
                continue;
            }
            if lower == "[blacklist]" || lower == "[black]" {
                mode = Mode::Blacklist;
                continue;
            }
            if lower.starts_with("whitelist=") {
                whitelist.extend(Self::parse_keywords(&line[11..]));
                continue;
            }
            if lower.starts_with("blacklist=") {
                blacklist.extend(Self::parse_keywords(&line[10..]));
                continue;
            }

            let keywords = Self::parse_keywords(line);
            match mode {
                Mode::Whitelist => whitelist.extend(keywords),
                Mode::Blacklist => blacklist.extend(keywords),
            }
        }

        self.whitelist = whitelist;
        self.blacklist = blacklist;
        self.normalized_whitelist = if self.case_sensitive {
            self.whitelist.clone()
        } else {
            self.whitelist.iter().map(|s| s.to_lowercase()).collect()
        };
        self.normalized_blacklist = if self.case_sensitive {
            self.blacklist.clone()
        } else {
            self.blacklist.iter().map(|s| s.to_lowercase()).collect()
        };

        tracing::info!(
            "关键词规则已加载: whitelist={} blacklist={}",
            self.whitelist.len(),
            self.blacklist.len()
        );

        Ok(())
    }

    fn parse_keywords(raw: &str) -> Vec<String> {
        raw.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    pub fn should_forward(&self, text: &str) -> (bool, Option<String>) {
        let normalized = if self.case_sensitive {
            text.to_string()
        } else {
            text.to_lowercase()
        };

        if !self.normalized_whitelist.is_empty() {
            let matched = self
                .normalized_whitelist
                .iter()
                .any(|kw| normalized.contains(kw));

            if !matched {
                return (false, Some("未匹配白名单".to_string()));
            }
        }

        for kw in &self.normalized_blacklist {
            if normalized.contains(kw) {
                return (false, Some(format!("匹配黑名单: {}", kw)));
            }
        }

        (true, None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Whitelist,
    Blacklist,
}
