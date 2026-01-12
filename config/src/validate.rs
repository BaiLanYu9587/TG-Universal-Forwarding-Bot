use super::AppConfig;
use anyhow::Result;

pub fn validate_config(config: &AppConfig) -> Result<()> {
    const TG_ALBUM_MAX_ITEMS_UPPER: usize = 10;

    if config.api_id <= 0 {
        anyhow::bail!("TG_API_ID 必须为正整数");
    }

    if config.api_hash.is_empty() {
        anyhow::bail!("TG_API_HASH 不能为空");
    }

    if config.target.is_empty() {
        anyhow::bail!("TG_TARGET 不能为空");
    }

    if !matches!(config.mode.as_str(), "live" | "past") {
        anyhow::bail!("TG_MODE 必须为 live 或 past");
    }

    if config.past_limit == 0 {
        anyhow::bail!("TG_PAST_LIMIT 必须大于 0");
    }

    if config.worker_count == 0 {
        anyhow::bail!("TG_WORKER_COUNT 必须大于 0");
    }
    if config.album_max_items == 0 {
        anyhow::bail!("TG_ALBUM_MAX_ITEMS 必须大于 0");
    }
    if config.album_max_items > TG_ALBUM_MAX_ITEMS_UPPER {
        anyhow::bail!(
            "TG_ALBUM_MAX_ITEMS 不能超过 {}（Telegram 相册上限）",
            TG_ALBUM_MAX_ITEMS_UPPER
        );
    }
    if config.state_flush_interval == 0 {
        anyhow::bail!("TG_STATE_FLUSH_INTERVAL 必须大于 0");
    }
    if config.shutdown_drain_timeout == 0 {
        anyhow::bail!("TG_SHUTDOWN_DRAIN_TIMEOUT 必须大于 0");
    }
    if config.state_gap_timeout == 0 {
        anyhow::bail!("TG_STATE_GAP_TIMEOUT 必须大于 0");
    }
    if config.state_pending_limit == 0 {
        anyhow::bail!("TG_STATE_PENDING_LIMIT 必须大于 0");
    }
    if config.hotreload_interval == 0 {
        anyhow::bail!("TG_HOTRELOAD_INTERVAL 必须大于 0");
    }
    if config.dedup_simhash_threshold == 0 {
        anyhow::bail!("TG_DEDUP_SIMHASH_THRESHOLD 必须大于 0");
    }
    if !(0.0..=1.0).contains(&config.dedup_jaccard_short_threshold) {
        anyhow::bail!("TG_DEDUP_JACCARD_SHORT_THRESHOLD 必须在 0-1 之间");
    }
    if !(0.0..=1.0).contains(&config.dedup_jaccard_long_threshold) {
        anyhow::bail!("TG_DEDUP_JACCARD_LONG_THRESHOLD 必须在 0-1 之间");
    }
    if config.dedup_recent_text_limit == 0 {
        anyhow::bail!("TG_DEDUP_RECENT_TEXT_LIMIT 必须大于 0");
    }
    if config.dedup_thumb_phash_threshold == 0 {
        anyhow::bail!("TG_DEDUP_PHASH_THRESHOLD 必须大于 0");
    }
    if config.dedup_thumb_dhash_threshold == 0 {
        anyhow::bail!("TG_DEDUP_DHASH_THRESHOLD 必须大于 0");
    }
    if !(0.0..=1.0).contains(&config.dedup_album_thumb_ratio) {
        anyhow::bail!("TG_DEDUP_ALBUM_THUMB_RATIO 必须在 0-1 之间");
    }
    if config.dedup_short_text_threshold == 0 {
        anyhow::bail!("TG_DEDUP_SHORT_TEXT_THRESHOLD 必须大于 0");
    }
    if config.thumb_dir.as_os_str().is_empty() {
        anyhow::bail!("TG_THUMB_DIR 不能为空");
    }
    if config.append_limit_with_media == 0 || config.append_limit_text == 0 {
        anyhow::bail!("TG_APPEND_LIMIT_* 必须大于 0");
    }

    if config.album_delay < 0.0 {
        anyhow::bail!("TG_ALBUM_DELAY 不能为负数");
    }

    if config.forward_delay < 0.0 {
        anyhow::bail!("TG_FORWARD_DELAY 不能为负数");
    }

    if config.flood_wait_max_retries == 0 {
        anyhow::bail!("TG_FLOOD_WAIT_MAX_RETRIES 必须大于 0");
    }
    if config.flood_wait_max_total_wait == 0 {
        anyhow::bail!("TG_FLOOD_WAIT_MAX_TOTAL_WAIT 必须大于 0");
    }
    if config.request_timeout == 0 {
        anyhow::bail!("TG_REQUEST_TIMEOUT 必须大于 0");
    }
    if config.dispatcher_idle_timeout == 0 {
        anyhow::bail!("TG_DISPATCHER_IDLE_TIMEOUT 必须大于 0");
    }

    Ok(())
}
