use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::Path;

pub fn parse_sources<P: AsRef<Path>>(path: P) -> Result<Vec<String>> {
    let path = path.as_ref();

    if !path.exists() {
        anyhow::bail!("sources.txt 不存在: {:?}", path);
    }

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("无法读取 sources.txt: {:?}", path))?;

    let mut sources = Vec::new();
    let mut seen = HashSet::new();

    for raw_line in content.lines() {
        let line = raw_line.trim();

        // 跳过空行
        if line.is_empty() {
            continue;
        }

        // 处理行内注释
        let source = if let Some(pos) = line.find('#') {
            &line[..pos]
        } else {
            line
        }
        .trim();

        if source.is_empty() {
            continue;
        }

        // 规范化 URL 格式
        let normalized = if source.starts_with("https://t.me/") {
            source.strip_prefix("https://t.me/").unwrap().to_string()
        } else if source.starts_with("t.me/") {
            source.strip_prefix("t.me/").unwrap().to_string()
        } else {
            source.to_string()
        };

        // 去重
        if !seen.insert(normalized.clone()) {
            continue;
        }

        sources.push(normalized);
    }

    if sources.is_empty() {
        anyhow::bail!("sources.txt 中没有有效的来源");
    }

    tracing::info!("已解析 sources.txt: {} 个来源", sources.len());

    Ok(sources)
}
