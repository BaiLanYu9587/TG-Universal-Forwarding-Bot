use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static PROJECT_ROOT: OnceLock<PathBuf> = OnceLock::new();

fn detect_project_root() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

pub fn project_root() -> PathBuf {
    PROJECT_ROOT.get_or_init(detect_project_root).clone()
}

pub fn resolve_path(raw: &str, default: &str) -> PathBuf {
    let base = project_root();

    if raw.is_empty() {
        return base.join(default);
    }

    let path = PathBuf::from(raw);
    if path.is_absolute() {
        return path;
    }

    base.join(path)
}

pub fn ensure_parent_exists(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}
