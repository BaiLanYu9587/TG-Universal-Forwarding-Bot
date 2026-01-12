use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::paths::{project_root, resolve_path};
use super::validate::validate_config;
use std::env;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub api_id: i32,
    pub api_hash: String,
    pub source_file: PathBuf,
    pub target: String,
    pub dedup_file: PathBuf,

    #[serde(default = "default_mode")]
    pub mode: String,

    #[serde(default = "default_past_limit")]
    pub past_limit: usize,

    #[serde(default = "default_album_delay")]
    pub album_delay: f64,
    #[serde(default = "default_album_max_items")]
    pub album_max_items: usize,
    #[serde(default = "default_album_backfill_max_range")]
    pub album_backfill_max_range: usize,

    #[serde(default = "default_forward_delay")]
    pub forward_delay: f64,

    #[serde(default = "default_flood_wait_max_retries")]
    pub flood_wait_max_retries: usize,

    #[serde(default = "default_flood_wait_max_total_wait")]
    pub flood_wait_max_total_wait: usize,

    #[serde(default = "default_request_timeout")]
    pub request_timeout: u64,

    #[serde(default = "default_dispatcher_idle_timeout")]
    pub dispatcher_idle_timeout: u64,

    #[serde(default = "default_updates_idle_timeout")]
    pub updates_idle_timeout: u64,

    #[serde(default = "default_worker_count")]
    pub worker_count: usize,

    #[serde(default = "default_keyword_case_sensitive")]
    pub keyword_case_sensitive: bool,

    #[serde(default)]
    pub keyword_file: Option<PathBuf>,

    #[serde(default = "default_keyword_reload_interval")]
    pub keyword_reload_interval: f64,

    #[serde(default)]
    pub replacement_file: Option<PathBuf>,

    #[serde(default)]
    pub content_addition_file: Option<PathBuf>,

    #[serde(default = "default_session_name")]
    pub session_name: String,

    #[serde(default = "default_log_level")]
    pub log_level: String,

    #[serde(default = "default_log_max_lines")]
    pub log_max_lines: usize,
    #[serde(default = "default_state_flush_interval")]
    pub state_flush_interval: usize,
    #[serde(default = "default_shutdown_drain_timeout")]
    pub shutdown_drain_timeout: u64,
    #[serde(default = "default_state_gap_timeout")]
    pub state_gap_timeout: u64,
    #[serde(default = "default_state_pending_limit")]
    pub state_pending_limit: usize,
    #[serde(default = "default_hotreload_interval")]
    pub hotreload_interval: usize,

    #[serde(default = "default_media_size_limit")]
    pub media_size_limit: usize,

    #[serde(default)]
    pub media_ext_allowlist: Vec<String>,

    #[serde(default = "default_enable_file_forward")]
    pub enable_file_forward: bool,

    #[serde(default = "default_dedup_limit")]
    pub dedup_limit: usize,

    #[serde(default = "default_media_dedup_limit")]
    pub media_dedup_limit: usize,
    #[serde(default = "default_dedup_simhash_threshold")]
    pub dedup_simhash_threshold: usize,
    #[serde(default = "default_dedup_jaccard_short_threshold")]
    pub dedup_jaccard_short_threshold: f64,
    #[serde(default = "default_dedup_jaccard_long_threshold")]
    pub dedup_jaccard_long_threshold: f64,
    #[serde(default = "default_dedup_recent_text_limit")]
    pub dedup_recent_text_limit: usize,
    #[serde(default = "default_thumb_dir")]
    pub thumb_dir: PathBuf,
    #[serde(default = "default_dedup_thumb_phash_threshold")]
    pub dedup_thumb_phash_threshold: usize,
    #[serde(default = "default_dedup_thumb_dhash_threshold")]
    pub dedup_thumb_dhash_threshold: usize,

    #[serde(default = "default_dedup_album_thumb_ratio")]
    pub dedup_album_thumb_ratio: f64,

    #[serde(default = "default_dedup_short_text_threshold")]
    pub dedup_short_text_threshold: usize,

    #[serde(default = "default_text_merge_window")]
    pub text_merge_window: f64,

    #[serde(default = "default_text_merge_min_len")]
    pub text_merge_min_len: usize,

    #[serde(default = "default_text_merge_max_id_gap")]
    pub text_merge_max_id_gap: usize,

    #[serde(default = "default_append_limit_with_media")]
    pub append_limit_with_media: usize,

    #[serde(default = "default_append_limit_text")]
    pub append_limit_text: usize,

    #[serde(default = "default_catch_up_interval")]
    pub catch_up_interval: usize,

    #[serde(default)]
    pub tdlib_db_dir: Option<String>,
    #[serde(default)]
    pub tdlib_files_dir: Option<String>,
    #[serde(default)]
    pub device_model: Option<String>,
    #[serde(default)]
    pub system_version: Option<String>,
    #[serde(default)]
    pub app_version: Option<String>,
    #[serde(default)]
    pub system_lang: Option<String>,
    #[serde(default)]
    pub use_test_dc: Option<bool>,
}

fn default_mode() -> String {
    "live".to_string()
}
fn default_past_limit() -> usize {
    100
}
fn default_album_delay() -> f64 {
    5.0
}
fn default_album_max_items() -> usize {
    10
}
fn default_album_backfill_max_range() -> usize {
    20
}
fn default_forward_delay() -> f64 {
    0.0
}
fn default_flood_wait_max_retries() -> usize {
    5
}
fn default_flood_wait_max_total_wait() -> usize {
    3600
}
fn default_request_timeout() -> u64 {
    30
}
fn default_dispatcher_idle_timeout() -> u64 {
    300
}
fn default_updates_idle_timeout() -> u64 {
    0
}
fn default_worker_count() -> usize {
    3
}
fn default_keyword_case_sensitive() -> bool {
    false
}
fn default_keyword_reload_interval() -> f64 {
    2.0
}
fn default_session_name() -> String {
    "user_session".to_string()
}
fn default_log_level() -> String {
    "INFO".to_string()
}
fn default_log_max_lines() -> usize {
    100
}
fn default_state_flush_interval() -> usize {
    5
}
fn default_shutdown_drain_timeout() -> u64 {
    30
}
fn default_state_gap_timeout() -> u64 {
    300
}
fn default_state_pending_limit() -> usize {
    2000
}
fn default_hotreload_interval() -> usize {
    2
}
fn default_media_size_limit() -> usize {
    100 * 1024 * 1024
}
fn default_enable_file_forward() -> bool {
    true
}
fn default_dedup_limit() -> usize {
    5000
}
fn default_media_dedup_limit() -> usize {
    15000
}
fn default_dedup_simhash_threshold() -> usize {
    3
}
fn default_dedup_jaccard_short_threshold() -> f64 {
    0.7
}
fn default_dedup_jaccard_long_threshold() -> f64 {
    0.5
}
fn default_dedup_recent_text_limit() -> usize {
    100
}
fn default_thumb_dir() -> PathBuf {
    PathBuf::from("data")
}
fn default_dedup_thumb_phash_threshold() -> usize {
    10
}
fn default_dedup_thumb_dhash_threshold() -> usize {
    10
}
fn default_dedup_album_thumb_ratio() -> f64 {
    0.34
}
fn default_dedup_short_text_threshold() -> usize {
    50
}
fn default_text_merge_window() -> f64 {
    0.0
}
fn default_text_merge_min_len() -> usize {
    0
}
fn default_text_merge_max_id_gap() -> usize {
    5
}
fn default_append_limit_with_media() -> usize {
    1024
}
fn default_append_limit_text() -> usize {
    4096
}
fn default_catch_up_interval() -> usize {
    300
}

pub fn load_config() -> Result<AppConfig> {
    let project_root = project_root();
    let env_path = project_root.join(".env");

    if env_path.exists() {
        dotenv::from_path(&env_path)?;
    }

    let api_id_str = env::var("TG_API_ID").unwrap_or_default().trim().to_string();
    let api_id = api_id_str.parse::<i32>().context("TG_API_ID 必须为整数")?;

    let api_hash = env::var("TG_API_HASH")
        .context("请设置 TG_API_HASH")?
        .trim()
        .to_string();

    let target = env::var("TG_TARGET")
        .context("请设置 TG_TARGET")?
        .trim()
        .to_string();

    let dedup_file_raw =
        env::var("TG_DEDUP_FILE").unwrap_or_else(|_| "deduplication.json".to_string());
    let dedup_file = resolve_path(&dedup_file_raw, "deduplication.json");
    let thumb_dir_raw = env::var("TG_THUMB_DIR").unwrap_or_else(|_| "data".to_string());
    let thumb_dir = resolve_path(&thumb_dir_raw, "data");

    let source_file_raw = env::var("TG_SOURCE_FILE").unwrap_or_else(|_| "sources.txt".to_string());
    let source_file = resolve_path(&source_file_raw, "sources.txt");

    let keyword_file_raw = env::var("TG_KEYWORD_FILE").unwrap_or_default();
    let keyword_file = if keyword_file_raw.is_empty() {
        None
    } else {
        Some(resolve_path(&keyword_file_raw, "keywords.txt"))
    };

    let replacement_file_raw = env::var("TG_REPLACEMENT_FILE").unwrap_or_default();
    let replacement_file = if replacement_file_raw.is_empty() {
        None
    } else {
        Some(resolve_path(&replacement_file_raw, "replacements.json"))
    };

    let content_addition_file_raw = env::var("TG_CONTENT_ADDITION_FILE").unwrap_or_default();
    let content_addition_file = if content_addition_file_raw.is_empty() {
        None
    } else {
        Some(resolve_path(
            &content_addition_file_raw,
            "content_addition.json",
        ))
    };

    let session_name = env::var("TG_SESSION_NAME").unwrap_or_else(|_| "user_session".to_string());

    let mode = env::var("TG_MODE")
        .unwrap_or_else(|_| "live".to_string())
        .to_lowercase();

    let past_limit = env::var("TG_PAST_LIMIT")
        .unwrap_or_else(|_| "2000".to_string())
        .parse::<usize>()
        .unwrap_or(2000);

    let album_delay = env::var("TG_ALBUM_DELAY")
        .unwrap_or_else(|_| "5.0".to_string())
        .parse::<f64>()
        .unwrap_or(5.0);
    tracing::info!("已加载 TG_ALBUM_DELAY: {} 秒", album_delay);
    let album_max_items = env::var("TG_ALBUM_MAX_ITEMS")
        .unwrap_or_else(|_| "10".to_string())
        .parse::<usize>()
        .unwrap_or(10);
    let album_backfill_max_range = env::var("TG_ALBUM_BACKFILL_MAX_RANGE")
        .unwrap_or_else(|_| "20".to_string())
        .parse::<usize>()
        .unwrap_or(20);

    let forward_delay = env::var("TG_FORWARD_DELAY")
        .unwrap_or_else(|_| "0".to_string())
        .parse::<f64>()
        .unwrap_or(0.0);

    let flood_wait_max_retries = env::var("TG_FLOOD_WAIT_MAX_RETRIES")
        .unwrap_or_else(|_| default_flood_wait_max_retries().to_string())
        .parse::<usize>()
        .unwrap_or(default_flood_wait_max_retries());
    let flood_wait_max_total_wait = env::var("TG_FLOOD_WAIT_MAX_TOTAL_WAIT")
        .unwrap_or_else(|_| default_flood_wait_max_total_wait().to_string())
        .parse::<usize>()
        .unwrap_or(default_flood_wait_max_total_wait());

    let request_timeout = env::var("TG_REQUEST_TIMEOUT")
        .unwrap_or_else(|_| default_request_timeout().to_string())
        .parse::<u64>()
        .unwrap_or(default_request_timeout());
    let dispatcher_idle_timeout = env::var("TG_DISPATCHER_IDLE_TIMEOUT")
        .unwrap_or_else(|_| default_dispatcher_idle_timeout().to_string())
        .parse::<u64>()
        .unwrap_or(default_dispatcher_idle_timeout());
    let updates_idle_timeout = env::var("TG_UPDATES_IDLE_TIMEOUT")
        .unwrap_or_else(|_| default_updates_idle_timeout().to_string())
        .parse::<u64>()
        .unwrap_or(default_updates_idle_timeout());

    let worker_count = env::var("TG_WORKER_COUNT")
        .unwrap_or_else(|_| "3".to_string())
        .parse::<usize>()
        .unwrap_or(3);

    let keyword_case_sensitive = matches!(
        env::var("TG_KEYWORD_CASE_SENSITIVE")
            .unwrap_or_else(|_| "false".to_string())
            .to_lowercase()
            .as_str(),
        "true" | "1" | "yes" | "on"
    );

    let keyword_reload_interval = env::var("TG_KEYWORD_RELOAD_INTERVAL")
        .unwrap_or_else(|_| "2".to_string())
        .parse::<f64>()
        .unwrap_or(2.0);

    let log_level = env::var("TG_LOG_LEVEL").unwrap_or_else(|_| "INFO".to_string());

    let log_max_lines = env::var("TG_LOG_MAX_LINES")
        .unwrap_or_else(|_| "100".to_string())
        .parse::<usize>()
        .unwrap_or(100);
    let state_flush_interval = env::var("TG_STATE_FLUSH_INTERVAL")
        .unwrap_or_else(|_| "5".to_string())
        .parse::<usize>()
        .unwrap_or(5);
    let shutdown_drain_timeout = env::var("TG_SHUTDOWN_DRAIN_TIMEOUT")
        .unwrap_or_else(|_| default_shutdown_drain_timeout().to_string())
        .parse::<u64>()
        .unwrap_or(default_shutdown_drain_timeout());
    let state_gap_timeout = env::var("TG_STATE_GAP_TIMEOUT")
        .unwrap_or_else(|_| "300".to_string())
        .parse::<u64>()
        .unwrap_or(300);
    let state_pending_limit = env::var("TG_STATE_PENDING_LIMIT")
        .unwrap_or_else(|_| "2000".to_string())
        .parse::<usize>()
        .unwrap_or(2000);
    let hotreload_interval = env::var("TG_HOTRELOAD_INTERVAL")
        .unwrap_or_else(|_| "2".to_string())
        .parse::<usize>()
        .unwrap_or(2);

    let media_size_limit_mb = env::var("TG_MEDIA_SIZE_LIMIT")
        .unwrap_or_else(|_| "100".to_string())
        .parse::<usize>()
        .unwrap_or(100);
    let media_size_limit = media_size_limit_mb * 1024 * 1024;

    let media_ext_allowlist_raw = env::var("TG_MEDIA_EXT_ALLOWLIST").unwrap_or_default();
    let media_ext_allowlist: Vec<String> = media_ext_allowlist_raw
        .split(&[',', ';', '|', ' '][..])
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    let enable_file_forward = matches!(
        env::var("TG_ENABLE_FILE_FORWARD")
            .unwrap_or_else(|_| "true".to_string())
            .to_lowercase()
            .as_str(),
        "true" | "1" | "yes" | "on"
    );

    let dedup_limit = env::var("TG_DEDUP_LIMIT")
        .unwrap_or_else(|_| "5000".to_string())
        .parse::<usize>()
        .unwrap_or(5000);

    let media_dedup_limit = env::var("TG_MEDIA_DEDUP_LIMIT")
        .unwrap_or_else(|_| "15000".to_string())
        .parse::<usize>()
        .unwrap_or(15000);
    let dedup_simhash_threshold = env::var("TG_DEDUP_SIMHASH_THRESHOLD")
        .unwrap_or_else(|_| "3".to_string())
        .parse::<usize>()
        .unwrap_or(3);
    let dedup_jaccard_short_threshold = env::var("TG_DEDUP_JACCARD_SHORT_THRESHOLD")
        .unwrap_or_else(|_| "0.7".to_string())
        .parse::<f64>()
        .unwrap_or(0.7);
    let dedup_jaccard_long_threshold = env::var("TG_DEDUP_JACCARD_LONG_THRESHOLD")
        .unwrap_or_else(|_| "0.5".to_string())
        .parse::<f64>()
        .unwrap_or(0.5);
    let dedup_recent_text_limit = env::var("TG_DEDUP_RECENT_TEXT_LIMIT")
        .unwrap_or_else(|_| "100".to_string())
        .parse::<usize>()
        .unwrap_or(100);
    let dedup_thumb_phash_threshold = env::var("TG_DEDUP_PHASH_THRESHOLD")
        .unwrap_or_else(|_| "10".to_string())
        .parse::<usize>()
        .unwrap_or(10);
    let dedup_thumb_dhash_threshold = env::var("TG_DEDUP_DHASH_THRESHOLD")
        .or_else(|_| env::var("TG_DEDUP_THUMB_DHASH_THRESHOLD"))
        .unwrap_or_else(|_| "10".to_string())
        .parse::<usize>()
        .unwrap_or(10);

    let dedup_album_thumb_ratio = env::var("TG_DEDUP_ALBUM_THUMB_RATIO")
        .unwrap_or_else(|_| "0.34".to_string())
        .parse::<f64>()
        .unwrap_or(0.34);

    let dedup_short_text_threshold = env::var("TG_DEDUP_SHORT_TEXT_THRESHOLD")
        .unwrap_or_else(|_| "50".to_string())
        .parse::<usize>()
        .unwrap_or(50);

    let text_merge_window = env::var("TG_TEXT_MERGE_WINDOW")
        .unwrap_or_else(|_| "0".to_string())
        .parse::<f64>()
        .unwrap_or(0.0);

    let text_merge_min_len = env::var("TG_TEXT_MERGE_MIN_LEN")
        .unwrap_or_else(|_| "0".to_string())
        .parse::<usize>()
        .unwrap_or(0);

    let text_merge_max_id_gap = env::var("TG_TEXT_MERGE_MAX_ID_GAP")
        .unwrap_or_else(|_| "5".to_string())
        .parse::<usize>()
        .unwrap_or(5);

    let append_limit_with_media = env::var("TG_APPEND_LIMIT_WITH_MEDIA")
        .unwrap_or_else(|_| "1024".to_string())
        .parse::<usize>()
        .unwrap_or(1024);

    let append_limit_text = env::var("TG_APPEND_LIMIT_TEXT")
        .unwrap_or_else(|_| "4096".to_string())
        .parse::<usize>()
        .unwrap_or(4096);

    let catch_up_interval = env::var("TG_CATCH_UP_INTERVAL")
        .unwrap_or_else(|_| "300".to_string())
        .parse::<usize>()
        .unwrap_or(300);

    let tdlib_db_dir = env::var("TG_TDLIB_DB_DIR").ok();
    let tdlib_files_dir = env::var("TG_TDLIB_FILES_DIR").ok();
    let device_model = env::var("TG_DEVICE_MODEL").ok();
    let system_version = env::var("TG_SYSTEM_VERSION").ok();
    let app_version = env::var("TG_APP_VERSION").ok();
    let system_lang = env::var("TG_SYSTEM_LANG").ok();
    let use_test_dc = env::var("TG_USE_TEST_DC")
        .ok()
        .map(|s| matches!(s.to_lowercase().as_str(), "true" | "1" | "yes" | "on"));

    let config = AppConfig {
        api_id,
        api_hash,
        source_file,
        target,
        dedup_file,
        mode,
        past_limit,
        album_delay,
        album_max_items,
        album_backfill_max_range,
        forward_delay,
        flood_wait_max_retries,
        flood_wait_max_total_wait,
        request_timeout,
        dispatcher_idle_timeout,
        updates_idle_timeout,
        worker_count,
        keyword_case_sensitive,
        keyword_file,
        keyword_reload_interval,
        replacement_file,
        content_addition_file,
        session_name,
        log_level,
        log_max_lines,
        state_flush_interval,
        shutdown_drain_timeout,
        state_gap_timeout,
        state_pending_limit,
        hotreload_interval,
        media_size_limit,
        media_ext_allowlist,
        enable_file_forward,
        dedup_limit,
        media_dedup_limit,
        dedup_simhash_threshold,
        dedup_jaccard_short_threshold,
        dedup_jaccard_long_threshold,
        dedup_recent_text_limit,
        thumb_dir,
        dedup_thumb_phash_threshold,
        dedup_thumb_dhash_threshold,
        dedup_album_thumb_ratio,
        dedup_short_text_threshold,
        text_merge_window,
        text_merge_min_len,
        text_merge_max_id_gap,
        append_limit_with_media,
        append_limit_text,
        catch_up_interval,
        tdlib_db_dir,
        tdlib_files_dir,
        device_model,
        system_version,
        app_version,
        system_lang,
        use_test_dc,
    };

    validate_config(&config)?;

    Ok(config)
}
