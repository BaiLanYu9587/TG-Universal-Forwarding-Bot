use anyhow::Result;
use std::path::Path;

pub fn rotate_log<P: AsRef<Path>>(path: P, max_lines: usize) -> Result<()> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(path)?;
    let lines: Vec<&str> = content.lines().collect();

    if lines.len() <= max_lines {
        return Ok(());
    }

    let start_idx = lines.len().saturating_sub(max_lines);
    let new_content = lines[start_idx..].join("\n");
    std::fs::write(path, new_content)?;

    Ok(())
}
