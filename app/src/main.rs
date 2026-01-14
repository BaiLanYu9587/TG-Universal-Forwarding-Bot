use anyhow::Result;
use config::paths::project_root;
use dotenv::dotenv;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::signal;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use common::text::truncate_text;
use config::{load_config, resolve_path, validate_config, AppConfig};
use logging::LogGuard;
use storage::{rotate_log, DedupStore, SessionCleaner, StateStore};
use tdlib::{FloodWaitPolicy, MessageHandler, PeerRef, TdlibClient};
use tg_core::{
    album::{AlbumAction, AlbumAggregator},
    append::ContentAppender,
    catch_up::{CatchUpFetcher, CatchUpFuture, CatchUpManager},
    dedup::{DedupConfig, MessageDeduplicator},
    dispatch::Dispatcher,
    filter::KeywordFilter,
    hotreload::HotReloadManager,
    media_filter::MediaFilter,
    pipeline::{Pipeline, PipelineConfig},
    replace::TextReplacer,
    text_merge::TextMergeManager,
    throttle::SendThrottle,
    ForwardTask, MessageView, TextEntity,
};
mod thumb_dedup;
use thumb_dedup::ThumbDedupManager;

struct SkipLogBatch {
    ranges: Vec<(i64, i64)>,
    count: usize,
    last_update: Instant,
}

impl SkipLogBatch {
    fn new(now: Instant) -> Self {
        Self {
            ranges: Vec::new(),
            count: 0,
            last_update: now,
        }
    }

    fn clear(&mut self, now: Instant) {
        self.ranges.clear();
        self.count = 0;
        self.last_update = now;
    }

    fn push(&mut self, msg_id: i64, now: Instant) {
        if let Some((start, end)) = self.ranges.last_mut() {
            if msg_id >= *start && msg_id <= *end {
                self.count += 1;
                self.last_update = now;
                return;
            }
            if msg_id == *end + 1 {
                *end = msg_id;
                self.count += 1;
                self.last_update = now;
                return;
            }
        }

        self.ranges.push((msg_id, msg_id));
        self.count += 1;
        self.last_update = now;
    }
}

struct SkipLogAggregator {
    per_source: std::collections::HashMap<String, SkipLogBatch>,
    max_ranges: usize,
    max_count: usize,
    max_age: Duration,
}

impl SkipLogAggregator {
    fn new(max_ranges: usize, max_count: usize, max_age: Duration) -> Self {
        Self {
            per_source: std::collections::HashMap::new(),
            max_ranges,
            max_count,
            max_age,
        }
    }

    fn record(&mut self, source: &str, msg_id: i64) {
        let now = Instant::now();
        let entry = self
            .per_source
            .entry(source.to_string())
            .or_insert_with(|| SkipLogBatch::new(now));

        if !entry.ranges.is_empty() && now.duration_since(entry.last_update) > self.max_age {
            Self::log_batch(source, entry);
            entry.clear(now);
        }

        entry.push(msg_id, now);

        if entry.ranges.len() >= self.max_ranges || entry.count >= self.max_count {
            Self::log_batch(source, entry);
            entry.clear(now);
        }
    }

    fn flush_expired(&mut self) {
        let now = Instant::now();
        let mut expired = Vec::new();

        for (source, batch) in &self.per_source {
            if !batch.ranges.is_empty() && now.duration_since(batch.last_update) > self.max_age {
                expired.push(source.clone());
            }
        }

        for source in expired {
            if let Some(batch) = self.per_source.get_mut(&source) {
                Self::log_batch(&source, batch);
                batch.clear(now);
            }
        }
    }

    fn flush_all(&mut self) {
        let now = Instant::now();
        let sources: Vec<String> = self.per_source.keys().cloned().collect();
        for source in sources {
            if let Some(batch) = self.per_source.get_mut(&source) {
                Self::log_batch(&source, batch);
                batch.clear(now);
            }
        }
    }

    fn log_batch(source: &str, batch: &SkipLogBatch) {
        if batch.ranges.is_empty() {
            return;
        }
        let ids = batch
            .ranges
            .iter()
            .map(|(start, end)| {
                if start == end {
                    start.to_string()
                } else {
                    format!("{}-{}", start, end)
                }
            })
            .collect::<Vec<_>>()
            .join(",");

        debug!(
            "消息已处理，批量跳过: source={} count={} ids={}",
            source, batch.count, ids
        );
    }
}

struct TimeoutWindow {
    window: Duration,
    attempts: std::collections::VecDeque<Instant>,
    timeouts: std::collections::VecDeque<Instant>,
}

impl TimeoutWindow {
    fn new(window: Duration) -> Self {
        Self {
            window,
            attempts: std::collections::VecDeque::new(),
            timeouts: std::collections::VecDeque::new(),
        }
    }

    fn record_attempt(&mut self, now: Instant) {
        self.attempts.push_back(now);
        self.prune(now);
    }

    fn record_timeout(&mut self, now: Instant) {
        self.timeouts.push_back(now);
        self.prune(now);
    }

    fn should_reload(&mut self, now: Instant) -> Option<f64> {
        self.prune(now);
        let attempts = self.attempts.len();
        if attempts == 0 {
            return None;
        }
        let rate = self.timeouts.len() as f64 / attempts as f64;
        if rate > 0.5 {
            Some(rate)
        } else {
            None
        }
    }

    fn attempts_len(&self) -> usize {
        self.attempts.len()
    }

    fn timeouts_len(&self) -> usize {
        self.timeouts.len()
    }

    fn prune(&mut self, now: Instant) {
        while let Some(front) = self.attempts.front() {
            if now.duration_since(*front) > self.window {
                self.attempts.pop_front();
            } else {
                break;
            }
        }
        while let Some(front) = self.timeouts.front() {
            if now.duration_since(*front) > self.window {
                self.timeouts.pop_front();
            } else {
                break;
            }
        }
    }
}

pub struct TelegramForwarder {
    config: AppConfig,
    client: Arc<TdlibClient>,
    state: Arc<tokio::sync::Mutex<StateStore>>,
    dedup: Arc<tokio::sync::Mutex<DedupStore>>,
    pipeline: Arc<tokio::sync::Mutex<Pipeline>>,
    album_aggregator: Arc<tokio::sync::Mutex<AlbumAggregator>>,
    text_merge: Arc<tokio::sync::Mutex<TextMergeManager>>,
    dispatcher: Dispatcher,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
    draining: Arc<std::sync::atomic::AtomicBool>,
    force_exit: Arc<std::sync::atomic::AtomicBool>,
    send_throttle: Arc<tokio::sync::Mutex<SendThrottle>>,
    sources: Vec<String>,
    hotreload: HotReloadManager,
    #[allow(dead_code)]
    hotreload_tx: mpsc::UnboundedSender<()>,
    hotreload_rx: Arc<tokio::sync::Mutex<Option<mpsc::UnboundedReceiver<()>>>>,
    catch_up_manager: CatchUpManager,
    peer_cache: Arc<tokio::sync::Mutex<std::collections::HashMap<String, PeerRef>>>,
    source_alias: Arc<tokio::sync::Mutex<std::collections::HashMap<String, String>>>,
    source_reverse: Arc<tokio::sync::Mutex<std::collections::HashMap<String, String>>>,
    inflight: Arc<tokio::sync::Mutex<HashMap<String, HashSet<i64>>>>,
    skip_log: Arc<tokio::sync::Mutex<SkipLogAggregator>>,
    _log_guard: &'static LogGuard,
}

pub enum RunOutcome {
    Exit,
    Reload,
}

struct AppCatchUpFetcher {
    client: Arc<TdlibClient>,
    page_limit: usize,
    request_timeout: Duration,
}

impl AppCatchUpFetcher {
    fn new(client: Arc<TdlibClient>, page_limit: usize, request_timeout: Duration) -> Self {
        Self {
            client,
            page_limit: page_limit.max(1),
            request_timeout,
        }
    }
}

impl CatchUpFetcher for AppCatchUpFetcher {
    fn fetch(&self, source: &str, last_id: i64) -> CatchUpFuture {
        let client = self.client.clone();
        let source = source.to_string();
        let page_limit = self.page_limit;
        let request_timeout = self.request_timeout;

        Box::pin(async move {
            let peer =
                match tokio::time::timeout(request_timeout, client.resolve_chat(&source)).await {
                    Ok(Ok(chat)) => chat,
                    Ok(Err(e)) => {
                        warn!("无法解析来源: {} error: {}", source, e);
                        return Err(e);
                    }
                    Err(_) => {
                        warn!(
                            "解析来源超时: {} timeout={}s",
                            source,
                            request_timeout.as_secs()
                        );
                        return Err(anyhow::anyhow!("解析来源超时"));
                    }
                };

            match tokio::time::timeout(
                request_timeout,
                client.get_messages_since((&peer).into(), last_id, page_limit),
            )
            .await
            {
                Ok(Ok(messages)) => Ok(messages),
                Ok(Err(e)) => {
                    warn!("获取补漏消息失败: {} error: {}", source, e);
                    Err(e)
                }
                Err(_) => {
                    warn!(
                        "获取补漏消息超时: {} timeout={}s",
                        source,
                        request_timeout.as_secs()
                    );
                    Err(anyhow::anyhow!("获取补漏消息超时"))
                }
            }
        })
    }
}

async fn register_hotreload_watchers(
    hotreload: &HotReloadManager,
    tx: &mpsc::UnboundedSender<()>,
    config: &AppConfig,
) {
    let project_root = project_root();
    let paths: Vec<PathBuf> = vec![project_root.join(".env"), config.source_file.clone()];

    for path in paths {
        if path.exists() {
            let sender = tx.clone();
            hotreload
                .register(path.to_string_lossy().as_ref(), move || {
                    let _ = sender.send(());
                    Ok(())
                })
                .await;
        }
    }

    if let Some(ref p) = config.keyword_file {
        if p.exists() {
            let sender = tx.clone();
            hotreload
                .register(p.to_string_lossy().as_ref(), move || {
                    let _ = sender.send(());
                    Ok(())
                })
                .await;
        }
    }
    if let Some(ref p) = config.replacement_file {
        if p.exists() {
            let sender = tx.clone();
            hotreload
                .register(p.to_string_lossy().as_ref(), move || {
                    let _ = sender.send(());
                    Ok(())
                })
                .await;
        }
    }
    if let Some(ref p) = config.content_addition_file {
        if p.exists() {
            let sender = tx.clone();
            hotreload
                .register(p.to_string_lossy().as_ref(), move || {
                    let _ = sender.send(());
                    Ok(())
                })
                .await;
        }
    }
}

async fn recv_update_with_timeout(
    receiver: &mut mpsc::UnboundedReceiver<MessageView>,
    timeout: Option<Duration>,
) -> Result<Option<MessageView>, tokio::time::error::Elapsed> {
    if let Some(timeout) = timeout {
        tokio::time::timeout(timeout, receiver.recv()).await
    } else {
        Ok(receiver.recv().await)
    }
}

fn is_network_send_error(err: &anyhow::Error) -> bool {
    let message = err.to_string();
    let lower = message.to_lowercase();
    lower.contains("read error")
        || lower.contains("io failed")
        || lower.contains("connection")
        || lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("network")
        || lower.contains("reset")
        || lower.contains("broken pipe")
        || lower.contains("transport")
        || message.contains("超时")
        || message.contains("网络")
        || message.contains("断开")
        || message.contains("连接重置")
}

impl TelegramForwarder {
    fn sanitize_identifier(raw: &str) -> String {
        let trimmed = raw.trim();
        let without_scheme = trimmed
            .trim_start_matches("https://t.me/")
            .trim_start_matches("http://t.me/")
            .trim_start_matches("t.me/");
        without_scheme.trim_start_matches('@').to_string()
    }

    pub async fn new() -> Result<Self> {
        let config = load_config()?;
        validate_config(&config)?;

        let log_file = resolve_path("", "logs.txt");
        if log_file.exists() {
            rotate_log(&log_file, config.log_max_lines)?;
        }

        let log_guard = logging::init(&log_file, &config.log_level);

        let sources = tg_core::parse_sources(&config.source_file)?;
        if sources.is_empty() {
            error!("源聊天文件为空，启动失败");
            anyhow::bail!("sources.txt 无有效来源");
        }
        info!("已加载 {} 个来源", sources.len());

        info!("正在初始化 TelegramForwarder...");

        let client =
            TdlibClient::connect(config.api_id, &config.api_hash, &config.session_name).await?;

        if !client.is_authorized().await? {
            info!("需要授权，请按提示完成登录");
            client.authorize().await?;
        }

        let client = Arc::new(client);

        let state_file = resolve_path("", "state.json");
        let mut state = StateStore::load(&state_file)?;
        state.set_gap_config(config.state_pending_limit, config.state_gap_timeout);
        let state = Arc::new(tokio::sync::Mutex::new(state));

        let dedup_file = config.dedup_file.clone();
        let dedup_store =
            DedupStore::load(&dedup_file, config.dedup_limit, config.media_dedup_limit)?;
        let snapshot = tg_core::dedup::DedupState {
            fingerprints: dedup_store.fingerprints(),
            media_id_signatures: dedup_store.media_id_signatures(),
            media_meta_signatures: dedup_store.media_meta_signatures(),
            media_thumb_hashes: dedup_store.media_thumb_hashes(),
            recent_text_features: dedup_store.recent_text_features(),
        };
        let dedup = Arc::new(tokio::sync::Mutex::new(dedup_store));

        let mut filter = KeywordFilter::new(config.keyword_case_sensitive);
        if let Some(ref kw_file) = config.keyword_file {
            if kw_file.exists() {
                filter.load_from_file(kw_file)?;
            }
        }

        let mut replacer = TextReplacer::new();
        if let Some(ref rep_file) = config.replacement_file {
            if rep_file.exists() {
                replacer.load_from_file(rep_file)?;
            }
        }

        let mut appender = ContentAppender::new();
        if let Some(ref add_file) = config.content_addition_file {
            if add_file.exists() {
                appender.load_from_file(add_file)?;
            }
        }

        let dedup_config = DedupConfig {
            limit: config.dedup_limit,
            media_limit: config.media_dedup_limit,
            hamming_threshold: config.dedup_simhash_threshold,
            jaccard_short_threshold: config.dedup_jaccard_short_threshold,
            jaccard_long_threshold: config.dedup_jaccard_long_threshold,
            recent_text_limit: config.dedup_recent_text_limit,
            thumb_phash_threshold: config.dedup_thumb_phash_threshold,
            thumb_dhash_threshold: config.dedup_thumb_dhash_threshold,
            album_thumb_ratio: config.dedup_album_thumb_ratio,
            short_text_threshold: config.dedup_short_text_threshold,
        };
        let deduplicator = MessageDeduplicator::from_snapshot(dedup_config, Some(snapshot));
        let request_timeout = Duration::from_secs(config.request_timeout);
        let thumb_hasher: Arc<dyn tg_core::thumb::ThumbHasher> = Arc::new(ThumbDedupManager::new(
            (*client).clone(),
            config.thumb_dir.clone(),
            request_timeout,
        )?);
        let media_filter = MediaFilter::new(
            config.media_size_limit,
            config.media_ext_allowlist.clone(),
            config.enable_file_forward,
        );
        let pipeline = Pipeline::new(PipelineConfig {
            filter,
            replacer,
            deduplicator,
            appender,
            media_filter,
            thumb_hasher: Some(thumb_hasher),
            append_limit_with_media: config.append_limit_with_media,
            append_limit_text: config.append_limit_text,
        });
        let pipeline = Arc::new(tokio::sync::Mutex::new(pipeline));

        let album_aggregator = AlbumAggregator::new(config.album_max_items);
        let text_merge = TextMergeManager::new(
            config.text_merge_window,
            config.text_merge_min_len,
            config.text_merge_max_id_gap,
        );
        let send_throttle = Arc::new(tokio::sync::Mutex::new(SendThrottle::new(
            config.forward_delay,
        )));
        let dispatcher = Dispatcher::new(config.worker_count);
        let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let draining = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let force_exit = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let hotreload = HotReloadManager::new();
        let (hotreload_tx, hotreload_rx) = mpsc::unbounded_channel();
        register_hotreload_watchers(&hotreload, &hotreload_tx, &config).await;
        let hotreload_rx = Arc::new(tokio::sync::Mutex::new(Some(hotreload_rx)));
        let catch_up_fetcher = Arc::new(AppCatchUpFetcher::new(
            client.clone(),
            config.past_limit,
            request_timeout,
        ));
        let catch_up_manager = CatchUpManager::new(catch_up_fetcher);
        let peer_cache = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let source_alias = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let source_reverse = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let inflight = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let skip_log = Arc::new(tokio::sync::Mutex::new(SkipLogAggregator::new(
            32,
            200,
            Duration::from_secs(2),
        )));

        Ok(Self {
            config,
            client,
            state,
            dedup,
            pipeline,
            album_aggregator: Arc::new(tokio::sync::Mutex::new(album_aggregator)),
            text_merge: Arc::new(tokio::sync::Mutex::new(text_merge)),
            dispatcher,
            shutdown,
            draining,
            force_exit,
            send_throttle,
            sources,
            hotreload,
            hotreload_tx,
            hotreload_rx,
            catch_up_manager,
            peer_cache,
            source_alias,
            source_reverse,
            inflight,
            skip_log,
            _log_guard: log_guard,
        })
    }

    pub async fn run(&mut self) -> Result<RunOutcome> {
        info!("启动转发器...");

        if self.config.mode == "past" {
            self.run_past_mode().await
        } else {
            self.run_live_mode().await
        }
    }

    async fn resolve_target_chat(&self) -> Result<PeerRef> {
        let target = Self::sanitize_identifier(&self.config.target);
        let request_timeout = Duration::from_secs(self.config.request_timeout);
        let peer =
            match tokio::time::timeout(request_timeout, self.client.resolve_chat(&target)).await {
                Ok(Ok(peer)) => peer,
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    anyhow::bail!(
                        "解析目标聊天超时: target={} timeout={}s",
                        target,
                        request_timeout.as_secs()
                    );
                }
            };
        Ok((&peer).into())
    }

    async fn get_source_peer(&self, source_key: &str) -> Option<PeerRef> {
        {
            let cache = self.peer_cache.lock().await;
            if let Some(peer) = cache.get(source_key) {
                return Some(*peer);
            }
        }

        let clean = Self::sanitize_identifier(source_key);
        let request_timeout = Duration::from_secs(self.config.request_timeout);
        let chat =
            match tokio::time::timeout(request_timeout, self.client.resolve_chat(&clean)).await {
                Ok(Ok(chat)) => chat,
                Ok(Err(e)) => {
                    warn!("无法解析来源: {} error: {}", source_key, e);
                    return None;
                }
                Err(_) => {
                    warn!(
                        "解析来源超时: source={} timeout={}s",
                        source_key,
                        request_timeout.as_secs()
                    );
                    return None;
                }
            };

        let peer_ref: PeerRef = (&chat).into();
        let mut cache = self.peer_cache.lock().await;
        cache.insert(source_key.to_string(), peer_ref);
        Some(peer_ref)
    }

    async fn backfill_album_messages(
        &self,
        source_key: &str,
        album_id: i64,
        min_id: i64,
        max_id: i64,
    ) -> Vec<MessageView> {
        // 只有有效的相册ID才需要补漏
        if album_id <= 0 {
            return Vec::new();
        }

        let Some((start_id, end_id)) =
            compute_backfill_range(min_id, max_id, self.config.album_backfill_max_range)
        else {
            return Vec::new();
        };
        debug!(
            "相册补漏范围: source={} album_id={} min_id={} max_id={} start_id={} end_id={}",
            source_key, album_id, min_id, max_id, start_id, end_id
        );

        let peer = match self.get_source_peer(source_key).await {
            Some(peer) => peer,
            None => return Vec::new(),
        };
        let ids: Vec<i64> = (start_id..=end_id).collect();
        let request_timeout = Duration::from_secs(self.config.request_timeout);

        for attempt in 1..=3 {
            match tokio::time::timeout(request_timeout, self.client.get_messages_by_id(peer, &ids))
                .await
            {
                Ok(Ok(messages)) => {
                    let filtered: Vec<MessageView> = messages
                        .into_iter()
                        .filter(|m| m.album_id == Some(album_id))
                        .collect();
                    debug!(
                        "相册补漏结果: source={} album_id={} count={} range=[{},{}]",
                        source_key,
                        album_id,
                        filtered.len(),
                        start_id,
                        end_id
                    );
                    return filtered;
                }
                Ok(Err(e)) => {
                    warn!(
                        "相册补漏失败 (第 {} 次): source={} err={}",
                        attempt, source_key, e
                    );
                }
                Err(_) => {
                    warn!(
                        "相册补漏超时 (第 {} 次): source={} timeout={}s",
                        attempt,
                        source_key,
                        request_timeout.as_secs()
                    );
                }
            }
            if attempt < 3 {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }

        warn!(
            "相册补漏最终失败: source={} album_id={}",
            source_key, album_id
        );
        Vec::new()
    }

    fn start_dispatcher(
        &mut self,
        handler: Arc<MessageHandler>,
        reload_tx: Option<mpsc::UnboundedSender<()>>,
    ) {
        let state = self.state.clone();
        let pipeline = self.pipeline.clone();
        let inflight = self.inflight.clone();
        let reload_tx = reload_tx.clone();
        let effective_gap_timeout = std::time::Duration::from_secs(
            self.config
                .state_gap_timeout
                .max(self.config.album_delay.max(0.0).ceil() as u64),
        );

        self.dispatcher.start(move |task| {
            let handler = handler.clone();
            let state = state.clone();
            let pipeline = pipeline.clone();
            let inflight = inflight.clone();
            let reload_tx = reload_tx.clone();
            let effective_gap_timeout = effective_gap_timeout;

            async move {
                let mut message_ids: Vec<i64> = task.messages.iter().map(|m| m.id).collect();
                message_ids.sort_unstable();
                message_ids.dedup();
                let source = task.source_key.clone();

                let processed = apply_pipeline_to_task(&pipeline, task).await;
                let mut send_failed = false;
                if let Some(processed_task) = processed {
                    let dedup_infos = processed_task.dedup_infos.clone();
                    match handler.process_task(processed_task).await {
                        Ok(()) => {
                            let mut pipeline = pipeline.lock().await;
                            for info in dedup_infos {
                                pipeline.commit_dedup(info);
                            }
                        }
                        Err(e) => {
                            let mut pipeline = pipeline.lock().await;
                            for info in dedup_infos {
                                pipeline.rollback_dedup(info);
                            }
                            error!("处理任务失败: {}", e);
                            if let Some(reload_tx) = reload_tx.as_ref() {
                                if is_network_send_error(&e) {
                                    warn!("发送任务疑似网络错误，触发重连: {}", e);
                                    let _ = reload_tx.send(());
                                }
                            }
                            send_failed = true;
                        }
                    }
                }

                if send_failed {
                    if !message_ids.is_empty() {
                        let mut inflight = inflight.lock().await;
                        if let Some(entry) = inflight.get_mut(&source) {
                            for msg_id in &message_ids {
                                entry.remove(msg_id);
                            }
                            if entry.is_empty() {
                                inflight.remove(&source);
                            }
                        }
                    }
                    return;
                }

                if !message_ids.is_empty() {
                    let mut s = state.lock().await;
                    for msg_id in &message_ids {
                        s.mark_processed_with_timeout(
                            Some(&source),
                            *msg_id,
                            effective_gap_timeout,
                        );
                    }
                }

                if !message_ids.is_empty() {
                    let mut inflight = inflight.lock().await;
                    if let Some(entry) = inflight.get_mut(&source) {
                        for msg_id in &message_ids {
                            entry.remove(msg_id);
                        }
                        if entry.is_empty() {
                            inflight.remove(&source);
                        }
                    }
                }
            }
        });
    }

    async fn wait_dispatcher_drain(&self, timeout: Duration, skip_on_timeout: bool) -> bool {
        info!("等待队列任务发送完成...");
        let drained = self.dispatcher.shutdown_graceful(timeout).await;
        if !drained {
            if self.force_exit.load(std::sync::atomic::Ordering::Relaxed) {
                warn!("强制退出：不再等待未完成的任务");
                return false;
            }
            if skip_on_timeout {
                warn!("等待超时，仍有发送任务未完成，不再等待以便重连");
                return false;
            }
            warn!("等待超时，仍有发送任务未完成，继续等待直到完成");
            self.dispatcher.wait_idle().await;
        }
        true
    }

    async fn mark_inflight_as_pending(&self, reason: &str) {
        let inflight = {
            let inflight = self.inflight.lock().await;
            inflight.clone()
        };

        if inflight.is_empty() {
            return;
        }

        let gap_timeout = Duration::from_secs(self.config.state_gap_timeout);
        let mut marked = 0usize;
        let mut state = self.state.lock().await;

        for (source, ids) in inflight {
            for msg_id in ids {
                state.mark_processed_with_timeout(Some(&source), msg_id, gap_timeout);
                marked += 1;
            }
        }

        if marked > 0 {
            warn!(
                "未完成任务被标记为已处理以避免重复发送: reason={} count={}",
                reason, marked
            );
        }
    }

    async fn run_live_mode(&mut self) -> Result<RunOutcome> {
        info!("启动实时模式...");

        let target_peer = self.resolve_target_chat().await?;

        let flood_wait_policy = FloodWaitPolicy {
            max_retries: self.config.flood_wait_max_retries,
            max_total_wait: Duration::from_secs(self.config.flood_wait_max_total_wait as u64),
            request_timeout: Duration::from_secs(self.config.request_timeout),
        };
        let handler = Arc::new(MessageHandler::new(
            self.client.clone(),
            target_peer,
            self.send_throttle.clone(),
            flood_wait_policy,
        ));

        self.start_dispatcher(handler.clone(), None);

        let state_file = resolve_path("", "state.json");
        let dedup_file = self.config.dedup_file.clone();
        let state_file_bg = state_file.clone();
        let dedup_file_bg = dedup_file.clone();
        let log_file = resolve_path("", "logs.txt");
        let state_flush_interval =
            tokio::time::Duration::from_secs(self.config.state_flush_interval as u64);
        let dispatcher_drain_timeout =
            tokio::time::Duration::from_secs(self.config.shutdown_drain_timeout);
        let shutdown = self.shutdown.clone();
        let draining = self.draining.clone();
        let album_delay = tokio::time::Duration::from_secs_f64(self.config.album_delay);
        let request_timeout = tokio::time::Duration::from_secs(self.config.request_timeout);
        let catch_up_interval =
            tokio::time::Duration::from_secs(self.config.catch_up_interval as u64);
        let mut hotreload_rx = {
            let mut guard = self.hotreload_rx.lock().await;
            guard.take().expect("hotreload 通道已被消费")
        };

        let state_clone = self.state.clone();
        let dedup_clone = self.dedup.clone();
        let pipeline_clone = self.pipeline.clone();
        let shutdown_clone = shutdown.clone();
        let hotreload_interval =
            tokio::time::Duration::from_secs(self.config.hotreload_interval as u64);
        let log_max_lines = self.config.log_max_lines;

        let state_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(state_flush_interval);
            loop {
                interval.tick().await;
                if shutdown_clone.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                let mut saved = false;
                {
                    let mut s = state_clone.lock().await;
                    match s.flush(&state_file_bg) {
                        Ok(true) => saved = true,
                        Ok(false) => {} // No changes
                        Err(e) => {
                            error!("保存状态失败: {}", e);
                        }
                    }
                }
                let snap = {
                    let p = pipeline_clone.lock().await;
                    p.dedup_snapshot()
                };
                {
                    let mut d = dedup_clone.lock().await;
                    d.set_fingerprints(snap.fingerprints);
                    d.set_media_id_signatures(snap.media_id_signatures);
                    d.set_media_meta_signatures(snap.media_meta_signatures);
                    d.set_media_thumb_hashes(snap.media_thumb_hashes);
                    d.set_recent_text_features(snap.recent_text_features);
                    match d.flush(&dedup_file_bg) {
                        Ok(true) => saved = true,
                        Ok(false) => {} // No changes
                        Err(e) => {
                            error!("保存去重记录失败: {}", e);
                        }
                    }
                }
                if saved {
                    debug!("已保存状态和去重记录");
                }
                if let Err(e) = rotate_log(&log_file, log_max_lines) {
                    error!("日志截断失败: {}", e);
                }
            }
        });

        // Session WAL 文件清理任务
        let session_cleaner = SessionCleaner::new(&self.config.session_name);
        let session_clean_task = {
            let shutdown = shutdown.clone();
            tokio::spawn(async move {
                // 每 5 分钟执行一次 WAL checkpoint
                session_cleaner.periodic_checkpoint(300, shutdown).await;
            })
        };

        let hotreload_task = {
            let hotreload = Arc::new(self.hotreload.clone());
            let shutdown = shutdown.clone();

            tokio::spawn(async move {
                let mut interval = tokio::time::interval(hotreload_interval);
                loop {
                    interval.tick().await;
                    if shutdown.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    hotreload.check_and_reload().await;
                }
            })
        };

        let mut sources_map = std::collections::HashMap::new();
        let mut source_peers = std::collections::HashMap::new();
        let mut source_alias = std::collections::HashMap::new();
        let mut source_reverse = std::collections::HashMap::new();
        let mut stable_sources = Vec::new();
        let mut seen_chat_ids = std::collections::HashSet::new();
        let sources = self.sources.clone();
        for source in &sources {
            let clean = Self::sanitize_identifier(source);
            let chat = match self.client.resolve_chat(&clean).await {
                Ok(chat) => chat,
                Err(e) => {
                    warn!("无法解析来源: {} error: {}", source, e);
                    continue;
                }
            };

            let chat_id = chat.id().bot_api_dialog_id();
            if chat_id == 0 {
                continue;
            }

            let stable_key = chat_id.to_string();
            if !seen_chat_ids.insert(chat_id) {
                source_reverse.insert(source.clone(), stable_key.clone());
                source_alias
                    .entry(stable_key.clone())
                    .or_insert_with(|| source.clone());
                warn!(
                    "来源已重复解析，跳过重复配置: source={} chat_id={}",
                    source, chat_id
                );
                continue;
            }

            info!("来源解析成功: source={} chat_id={}", source, chat_id);
            sources_map.insert(chat_id, stable_key.clone());
            source_peers.insert(stable_key.clone(), (&chat).into());
            source_alias.insert(stable_key.clone(), source.clone());
            source_reverse.insert(source.clone(), stable_key.clone());
            stable_sources.push(stable_key);
        }

        let sources_set: std::collections::HashSet<i64> = sources_map.keys().cloned().collect();
        info!("监听 {} 个来源: {:?}", sources_set.len(), sources_set);

        {
            let mut cache = self.peer_cache.lock().await;
            *cache = source_peers.clone();
        }
        {
            let mut cache = self.source_alias.lock().await;
            *cache = source_alias.clone();
        }
        {
            let mut cache = self.source_reverse.lock().await;
            *cache = source_reverse.clone();
        }
        {
            let mut state = self.state.lock().await;
            for (raw, stable) in &source_reverse {
                state.migrate_source_key(raw, stable);
            }
        }

        let source_peers = Arc::new(source_peers);
        let album_backfill_max_range = self.config.album_backfill_max_range;
        let caption_limit = self.config.append_limit_with_media;

        let album_task = {
            let aggregator = self.album_aggregator.clone();
            let text_merge = self.text_merge.clone();
            let dispatcher = self.dispatcher.clone();
            let shutdown = shutdown.clone();
            let client = self.client.clone();
            let source_peers = source_peers.clone();

            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(1));
                loop {
                    interval.tick().await;
                    if shutdown.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }

                    let keys = {
                        let aggregator = aggregator.lock().await;
                        aggregator.expired_keys(album_delay).await
                    };

                    for key in keys {
                        let messages = {
                            let aggregator = aggregator.lock().await;
                            aggregator.flush(&key).await
                        };
                        let Some(mut messages) = messages else {
                            continue;
                        };
                        if messages.is_empty() {
                            continue;
                        }

                        let min_id = messages.iter().map(|m| m.id).min().unwrap_or(0);
                        let max_id = messages.iter().map(|m| m.id).max().unwrap_or(0);
                        let album_id = messages.first().and_then(|m| m.album_id).unwrap_or(0);
                        info!(
                            "相册超时触发发送: source={} album_id={} count={} min_id={} max_id={}",
                            key.source_key,
                            album_id,
                            messages.len(),
                            min_id,
                            max_id
                        );

                        // 只有真实相册（album_id > 0）才需要补漏
                        if album_id > 0 {
                            if let Some((start_id, end_id)) =
                                compute_backfill_range(min_id, max_id, album_backfill_max_range)
                            {
                                if let Some(peer) = source_peers.get(&key.source_key) {
                                    let ids: Vec<i64> = (start_id..=end_id).collect();
                                    match tokio::time::timeout(
                                        request_timeout,
                                        client.get_messages_by_id(*peer, &ids),
                                    )
                                    .await
                                    {
                                        Ok(Ok(backfilled)) => {
                                            let backfilled = backfilled
                                                .into_iter()
                                                .filter(|m| m.album_id == Some(album_id))
                                                .collect();
                                            extend_unique_messages(&mut messages, backfilled);
                                        }
                                        Ok(Err(e)) => {
                                            warn!(
                                                "相册补漏失败: source={} err={}",
                                                key.source_key, e
                                            );
                                        }
                                        Err(_) => {
                                            warn!(
                                                "相册补漏超时: source={} timeout={}s",
                                                key.source_key,
                                                request_timeout.as_secs()
                                            );
                                        }
                                    }
                                }
                            }
                        }

                        messages.sort_by_key(|m| m.id);
                        let min_id = messages.first().map(|m| m.id).unwrap_or(0);
                        let merged = {
                            let mut text_merge = text_merge.lock().await;
                            // 只有真实相册才合并前置文本
                            text_merge.try_merge_for_album(&key.source_key, min_id)
                        };
                        normalize_album_messages(&mut messages, merged, caption_limit);

                        // 真实相册：作为相册发送
                        let task = ForwardTask {
                            source_key: key.source_key.clone(),
                            messages,
                            is_album: true,
                            dedup_infos: Vec::new(),
                        };
                        dispatcher.dispatch(task);
                    }
                }
            })
        };

        let text_merge_task = if self.config.text_merge_window > 0.0 {
            let text_merge = self.text_merge.clone();
            let album_aggregator = self.album_aggregator.clone();
            let dispatcher = self.dispatcher.clone();
            let shutdown = shutdown.clone();

            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
                loop {
                    interval.tick().await;
                    if shutdown.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }

                    let potential_expired = {
                        let text_merge = text_merge.lock().await;
                        text_merge.get_expired_keys()
                    };

                    for key in potential_expired {
                        let has_pending_album = {
                            let aggregator = album_aggregator.lock().await;
                            aggregator.has_pending_album(&key).await
                        };

                        let mut text_merge = text_merge.lock().await;
                        if has_pending_album {
                            // 如果还有未发送的相册，延长文本缓存时间，等待相册发送时合并
                            text_merge.extend_expiration_if_expired(
                                &key,
                                std::time::Duration::from_secs(5),
                            );
                        } else if let Some(msg) = text_merge.remove_message_if_expired(&key) {
                            let task = ForwardTask {
                                source_key: key,
                                messages: vec![msg],
                                is_album: false,
                                dedup_infos: Vec::new(),
                            };
                            dispatcher.dispatch(task);
                        }
                    }
                }
            })
        } else {
            tokio::spawn(async move { {} })
        };

        let catch_up_task = {
            let catch_up = Arc::new(self.catch_up_manager.clone());
            let state = self.state.clone();
            let dispatcher = self.dispatcher.clone();
            let shutdown = shutdown.clone();
            let sources = stable_sources.clone();

            tokio::spawn(async move {
                let mut interval = tokio::time::interval(catch_up_interval);
                loop {
                    interval.tick().await;
                    if shutdown.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }

                    for source in &sources {
                        let last_id = {
                            let s = state.lock().await;
                            s.get_min_id(source)
                        };

                        let mut messages =
                            match catch_up.fetch_missed_messages(source, last_id).await {
                                Ok(msgs) => msgs,
                                Err(e) => {
                                    error!(
                                        "补漏失败: source={} last_id={} error={}",
                                        source, last_id, e
                                    );
                                    continue;
                                }
                            };

                        if messages.is_empty() {
                            continue;
                        }

                        // 补漏也按相册分组，避免相册被拆成多条
                        messages.sort_by_key(|m| m.id);
                        let mut current_group_id: Option<i64> = None;
                        let mut current_group: Vec<MessageView> = Vec::new();

                        for msg in messages {
                            if let Some(group_id) = msg.album_id {
                                if current_group_id.is_none() || current_group_id == Some(group_id)
                                {
                                    current_group_id = Some(group_id);
                                } else {
                                    if !current_group.is_empty() {
                                        let mut group = std::mem::take(&mut current_group);
                                        normalize_album_messages(&mut group, None, caption_limit);
                                        let task = ForwardTask {
                                            source_key: source.clone(),
                                            messages: group,
                                            is_album: true,
                                            dedup_infos: Vec::new(),
                                        };
                                        dispatcher.dispatch(task);
                                    }
                                    current_group_id = Some(group_id);
                                }
                                current_group.push(msg);
                            } else {
                                if !current_group.is_empty() {
                                    let mut group = std::mem::take(&mut current_group);
                                    normalize_album_messages(&mut group, None, caption_limit);
                                    let task = ForwardTask {
                                        source_key: source.clone(),
                                        messages: group,
                                        is_album: true,
                                        dedup_infos: Vec::new(),
                                    };
                                    dispatcher.dispatch(task);
                                }
                                current_group_id = None;

                                let task = ForwardTask {
                                    source_key: source.clone(),
                                    messages: vec![msg],
                                    is_album: false,
                                    dedup_infos: Vec::new(),
                                };
                                dispatcher.dispatch(task);
                            }
                        }

                        if !current_group.is_empty() {
                            let mut group = std::mem::take(&mut current_group);
                            normalize_album_messages(&mut group, None, caption_limit);
                            let task = ForwardTask {
                                source_key: source.clone(),
                                messages: group,
                                is_album: true,
                                dedup_infos: Vec::new(),
                            };
                            dispatcher.dispatch(task);
                        }
                    }
                }
            })
        };

        info!("开始监听消息更新...");
        let mut update_receiver = self.client.subscribe_updates().await;
        let updates_idle_timeout = if self.config.updates_idle_timeout > 0 {
            Some(Duration::from_secs(self.config.updates_idle_timeout))
        } else {
            None
        };
        let updates_idle_timeout_secs = self.config.updates_idle_timeout;

        let mut message_count = 0usize;

        let outcome = loop {
            tokio::select! {
                result = recv_update_with_timeout(&mut update_receiver, updates_idle_timeout) => {
                    if self.draining.load(std::sync::atomic::Ordering::Relaxed) {
                        break RunOutcome::Exit;
                    }

                    match result {
                        Ok(None) => {
                            // 通道已关闭，可能是 runner 崩溃
                            error!("更新通道已关闭，可能是网络断开或 runner 崩溃");
                            // 检查 runner 状态
                            if !self.client.is_runner_alive() {
                                error!("确认 runner 已崩溃，触发重连...");
                                break RunOutcome::Reload;
                            } else {
                                error!("Runner 存活但通道关闭，触发重连...");
                                break RunOutcome::Reload;
                            }
                        }
                        Ok(Some(message)) => {
                            // 检查是否是特殊的 runner 崩溃标记消息
                            if message.id == 0 && message.chat_id == 0 && message.is_edit {
                                error!("收到 runner 崩溃通知，触发重连...");
                                break RunOutcome::Reload;
                            }

                            if let Some(source_key) = sources_map.get(&message.chat_id) {
                                message_count += 1;
                                if let Err(e) = self.handle_message(&message, source_key).await {
                                    error!("处理消息失败: {}", e);
                                }
                            }
                        }
                        Err(_) => {
                            warn!(
                                "更新流空闲超时: idle_timeout={}s，触发重连...",
                                updates_idle_timeout_secs
                            );
                            break RunOutcome::Reload;
                        }
                    }
                }
                change = hotreload_rx.recv() => {
                    if change.is_some() {
                        info!("检测到配置或来源文件变更，准备热重启...");
                        self.draining.store(true, std::sync::atomic::Ordering::Relaxed);
                        break RunOutcome::Reload;
                    }
                }
                _ = signal::ctrl_c() => {
                    info!("收到退出信号，已处理 {} 条消息", message_count);
                    self.draining.store(true, std::sync::atomic::Ordering::Relaxed);
                    self.force_exit
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    break RunOutcome::Exit;
                }
            }
        };

        shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
        draining.store(true, std::sync::atomic::Ordering::Relaxed);
        state_task.abort();
        session_clean_task.abort();
        hotreload_task.abort();
        album_task.abort();
        text_merge_task.abort();
        catch_up_task.abort();

        let pending_albums = {
            let aggregator = self.album_aggregator.lock().await;
            aggregator.drain_all().await
        };
        for (key, mut messages) in pending_albums {
            if messages.is_empty() {
                continue;
            }
            messages.sort_by_key(|m| m.id);
            let min_id = messages.first().map(|m| m.id).unwrap_or(0);
            let merged = {
                let mut text_merge = self.text_merge.lock().await;
                if min_id > 0 {
                    text_merge.try_merge_for_album(&key.source_key, min_id)
                } else {
                    None
                }
            };
            normalize_album_messages(&mut messages, merged, self.config.append_limit_with_media);

            let task = ForwardTask {
                source_key: key.source_key,
                messages,
                is_album: true,
                dedup_infos: Vec::new(),
            };
            self.dispatcher.dispatch(task);
        }

        let pending_texts = {
            let mut text_merge = self.text_merge.lock().await;
            text_merge.take_all()
        };
        for (source_key, msg) in pending_texts {
            let task = ForwardTask {
                source_key,
                messages: vec![msg],
                is_album: false,
                dedup_infos: Vec::new(),
            };
            self.dispatcher.dispatch(task);
        }

        let skip_on_timeout = matches!(outcome, RunOutcome::Reload);
        let drained = self
            .wait_dispatcher_drain(dispatcher_drain_timeout, skip_on_timeout)
            .await;
        if skip_on_timeout && !drained && self.config.mode == "past" {
            self.mark_inflight_as_pending("dispatcher_timeout").await;
        }

        {
            let mut p = self.pipeline.lock().await;
            let pending = p.take_all_pending();
            p.commit_all_pending(&pending);
        }

        {
            let p = self.pipeline.lock().await;
            let mut d = self.dedup.lock().await;
            let snap = p.dedup_snapshot();
            d.set_fingerprints(snap.fingerprints);
            d.set_media_id_signatures(snap.media_id_signatures);
            d.set_media_meta_signatures(snap.media_meta_signatures);
            d.set_media_thumb_hashes(snap.media_thumb_hashes);
            d.set_recent_text_features(snap.recent_text_features);
        }

        {
            let mut s = self.state.lock().await;
            if let Err(e) = s.flush(&state_file) {
                error!("退出时保存状态失败: {}", e);
            } else {
                info!("状态已保存");
            }
        }
        {
            let mut d = self.dedup.lock().await;
            if let Err(e) = d.flush(&dedup_file) {
                error!("退出时保存去重记录失败: {}", e);
            } else {
                info!("去重记录已保存");
            }
        }

        {
            let mut skip_log = self.skip_log.lock().await;
            skip_log.flush_all();
        }

        Ok(outcome)
    }

    async fn run_past_mode(&mut self) -> Result<RunOutcome> {
        info!("启动历史轮询模式，单页限制: {}", self.config.past_limit);

        let target_peer = self.resolve_target_chat().await?;

        let flood_wait_policy = FloodWaitPolicy {
            max_retries: self.config.flood_wait_max_retries,
            max_total_wait: Duration::from_secs(self.config.flood_wait_max_total_wait as u64),
            request_timeout: Duration::from_secs(self.config.request_timeout),
        };
        let handler = Arc::new(MessageHandler::new(
            self.client.clone(),
            target_peer,
            self.send_throttle.clone(),
            flood_wait_policy,
        ));

        let (reload_tx, mut reload_rx) = mpsc::unbounded_channel();
        self.start_dispatcher(handler.clone(), Some(reload_tx));

        let state_file = resolve_path("", "state.json");
        let dedup_file = self.config.dedup_file.clone();
        let state_file_bg = state_file.clone();
        let dedup_file_bg = dedup_file.clone();
        let log_file = resolve_path("", "logs.txt");
        let state_flush_interval =
            tokio::time::Duration::from_secs(self.config.state_flush_interval as u64);
        let dispatcher_drain_timeout =
            tokio::time::Duration::from_secs(self.config.shutdown_drain_timeout);
        let shutdown = self.shutdown.clone();
        let draining = self.draining.clone();
        let album_delay = tokio::time::Duration::from_secs_f64(self.config.album_delay);
        let poll_interval = tokio::time::Duration::from_secs(self.config.catch_up_interval as u64);
        let request_timeout = tokio::time::Duration::from_secs(self.config.request_timeout);
        let dispatcher_idle_timeout =
            tokio::time::Duration::from_secs(self.config.dispatcher_idle_timeout);
        let mut hotreload_rx = {
            let mut guard = self.hotreload_rx.lock().await;
            guard.take().expect("hotreload 通道已被消费")
        };

        let state_clone = self.state.clone();
        let dedup_clone = self.dedup.clone();
        let pipeline_clone = self.pipeline.clone();
        let shutdown_clone = shutdown.clone();
        let hotreload_interval =
            tokio::time::Duration::from_secs(self.config.hotreload_interval as u64);
        let log_max_lines = self.config.log_max_lines;

        let state_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(state_flush_interval);
            loop {
                interval.tick().await;
                if shutdown_clone.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                let mut saved = false;
                {
                    let mut s = state_clone.lock().await;
                    match s.flush(&state_file_bg) {
                        Ok(true) => saved = true,
                        Ok(false) => {} // No changes
                        Err(e) => {
                            error!("保存状态失败: {}", e);
                        }
                    }
                }
                let snap = {
                    let p = pipeline_clone.lock().await;
                    p.dedup_snapshot()
                };
                {
                    let mut d = dedup_clone.lock().await;
                    d.set_fingerprints(snap.fingerprints);
                    d.set_media_id_signatures(snap.media_id_signatures);
                    d.set_media_meta_signatures(snap.media_meta_signatures);
                    d.set_media_thumb_hashes(snap.media_thumb_hashes);
                    d.set_recent_text_features(snap.recent_text_features);
                    match d.flush(&dedup_file_bg) {
                        Ok(true) => saved = true,
                        Ok(false) => {} // No changes
                        Err(e) => {
                            error!("保存去重记录失败: {}", e);
                        }
                    }
                }
                if saved {
                    debug!("已保存状态和去重记录");
                }
                if let Err(e) = rotate_log(&log_file, log_max_lines) {
                    error!("日志截断失败: {}", e);
                }
            }
        });

        // Session WAL 文件清理任务
        let session_cleaner = SessionCleaner::new(&self.config.session_name);
        let session_clean_task = {
            let shutdown = shutdown.clone();
            tokio::spawn(async move {
                // 每 5 分钟执行一次 WAL checkpoint
                session_cleaner.periodic_checkpoint(300, shutdown).await;
            })
        };

        let thumb_dir = self.config.thumb_dir.clone();
        let thumb_cleanup_max_age = Duration::from_secs(600);
        let thumb_cleanup_interval = poll_interval.max(Duration::from_secs(60));
        let thumb_cleanup_running = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thumb_cleanup_task = {
            let shutdown = shutdown.clone();
            let thumb_dir = thumb_dir.clone();
            let thumb_cleanup_running = thumb_cleanup_running.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(thumb_cleanup_interval);
                loop {
                    interval.tick().await;
                    if shutdown.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    if thumb_cleanup_running
                        .compare_exchange(
                            false,
                            true,
                            std::sync::atomic::Ordering::SeqCst,
                            std::sync::atomic::Ordering::SeqCst,
                        )
                        .is_err()
                    {
                        continue;
                    }
                    if let Err(e) =
                        thumb_dedup::cleanup_orphan_thumbs(&thumb_dir, thumb_cleanup_max_age)
                    {
                        warn!("清理缩略图缓存失败: {:?}", e);
                    }
                    thumb_cleanup_running.store(false, std::sync::atomic::Ordering::SeqCst);
                }
            })
        };
        let hotreload_task = {
            let hotreload = Arc::new(self.hotreload.clone());
            let shutdown = shutdown.clone();

            tokio::spawn(async move {
                let mut interval = tokio::time::interval(hotreload_interval);
                loop {
                    interval.tick().await;
                    if shutdown.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    hotreload.check_and_reload().await;
                }
            })
        };

        let mut source_peers = std::collections::HashMap::new();
        let mut source_alias = std::collections::HashMap::new();
        let mut source_reverse = std::collections::HashMap::new();
        let mut stable_sources = Vec::new();
        let mut seen_chat_ids = std::collections::HashSet::new();
        for source in &self.sources {
            if let Some(peer) = self.get_source_peer(source).await {
                let chat_id = peer.id.bot_api_dialog_id();
                if chat_id == 0 {
                    continue;
                }
                let stable_key = chat_id.to_string();
                if !seen_chat_ids.insert(chat_id) {
                    source_reverse.insert(source.clone(), stable_key.clone());
                    source_alias
                        .entry(stable_key.clone())
                        .or_insert_with(|| source.clone());
                    warn!(
                        "来源已重复解析，跳过重复配置: source={} chat_id={}",
                        source, chat_id
                    );
                    continue;
                }
                info!("来源解析成功: source={} chat_id={}", source, chat_id);
                source_peers.insert(stable_key.clone(), peer);
                source_alias.insert(stable_key.clone(), source.clone());
                source_reverse.insert(source.clone(), stable_key.clone());
                stable_sources.push(stable_key);
            }
        }

        {
            let mut cache = self.peer_cache.lock().await;
            *cache = source_peers.clone();
        }
        {
            let mut cache = self.source_alias.lock().await;
            *cache = source_alias.clone();
        }
        {
            let mut cache = self.source_reverse.lock().await;
            *cache = source_reverse.clone();
        }
        {
            let mut state = self.state.lock().await;
            for (raw, stable) in &source_reverse {
                state.migrate_source_key(raw, stable);
            }
        }

        let source_peers = Arc::new(source_peers);
        let album_backfill_max_range = self.config.album_backfill_max_range;
        let caption_limit = self.config.append_limit_with_media;

        let album_task = {
            let aggregator = self.album_aggregator.clone();
            let text_merge = self.text_merge.clone();
            let dispatcher = self.dispatcher.clone();
            let shutdown = shutdown.clone();
            let client = self.client.clone();
            let source_peers = source_peers.clone();

            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(1));
                loop {
                    interval.tick().await;
                    if shutdown.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }

                    let keys = {
                        let aggregator = aggregator.lock().await;
                        aggregator.expired_keys(album_delay).await
                    };

                    for key in keys {
                        let messages = {
                            let aggregator = aggregator.lock().await;
                            aggregator.flush(&key).await
                        };
                        let Some(mut messages) = messages else {
                            continue;
                        };
                        if messages.is_empty() {
                            continue;
                        }

                        let min_id = messages.iter().map(|m| m.id).min().unwrap_or(0);
                        let max_id = messages.iter().map(|m| m.id).max().unwrap_or(0);
                        let album_id = messages.first().and_then(|m| m.album_id).unwrap_or(0);
                        info!(
                            "相册超时触发发送: source={} album_id={} count={} min_id={} max_id={}",
                            key.source_key,
                            album_id,
                            messages.len(),
                            min_id,
                            max_id
                        );

                        // 只有真实相册（album_id > 0）才需要补漏
                        if album_id > 0 {
                            if let Some((start_id, end_id)) =
                                compute_backfill_range(min_id, max_id, album_backfill_max_range)
                            {
                                if let Some(peer) = source_peers.get(&key.source_key) {
                                    let ids: Vec<i64> = (start_id..=end_id).collect();
                                    match tokio::time::timeout(
                                        request_timeout,
                                        client.get_messages_by_id(*peer, &ids),
                                    )
                                    .await
                                    {
                                        Ok(Ok(backfilled)) => {
                                            let backfilled = backfilled
                                                .into_iter()
                                                .filter(|m| m.album_id == Some(album_id))
                                                .collect();
                                            extend_unique_messages(&mut messages, backfilled);
                                        }
                                        Ok(Err(e)) => {
                                            warn!(
                                                "相册补漏失败: source={} err={}",
                                                key.source_key, e
                                            );
                                        }
                                        Err(_) => {
                                            warn!(
                                                "相册补漏超时: source={} timeout={}s",
                                                key.source_key,
                                                request_timeout.as_secs()
                                            );
                                        }
                                    }
                                }
                            }
                        }

                        messages.sort_by_key(|m| m.id);
                        let min_id = messages.first().map(|m| m.id).unwrap_or(0);
                        let merged = {
                            let mut text_merge = text_merge.lock().await;
                            // 只有真实相册才合并前置文本
                            text_merge.try_merge_for_album(&key.source_key, min_id)
                        };
                        normalize_album_messages(&mut messages, merged, caption_limit);

                        let task = ForwardTask {
                            source_key: key.source_key.clone(),
                            messages,
                            is_album: true,
                            dedup_infos: Vec::new(),
                        };
                        dispatcher.dispatch(task);
                    }
                }
            })
        };

        let text_merge_task = if self.config.text_merge_window > 0.0 {
            let text_merge = self.text_merge.clone();
            let album_aggregator = self.album_aggregator.clone();
            let dispatcher = self.dispatcher.clone();
            let shutdown = shutdown.clone();

            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
                loop {
                    interval.tick().await;
                    if shutdown.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }

                    let potential_expired = {
                        let text_merge = text_merge.lock().await;
                        text_merge.get_expired_keys()
                    };

                    for key in potential_expired {
                        let has_pending_album = {
                            let aggregator = album_aggregator.lock().await;
                            aggregator.has_pending_album(&key).await
                        };

                        let mut text_merge = text_merge.lock().await;
                        if has_pending_album {
                            // 如果还有未发送的相册，延长文本缓存时间，等待相册发送时合并
                            text_merge.extend_expiration(&key, std::time::Duration::from_secs(5));
                        } else if let Some(msg) = text_merge.remove_message(&key) {
                            let task = ForwardTask {
                                source_key: key,
                                messages: vec![msg],
                                is_album: false,
                                dedup_infos: Vec::new(),
                            };
                            dispatcher.dispatch(task);
                        }
                    }
                }
            })
        } else {
            tokio::spawn(async move { {} })
        };

        let sources = stable_sources.clone();
        let mut latest_start_ids = HashMap::new();
        for (source_key, peer) in source_peers.iter() {
            let latest =
                match tokio::time::timeout(request_timeout, self.client.get_messages(*peer, 1, 0))
                    .await
                {
                    Ok(Ok(mut msgs)) => msgs.pop(),
                    Ok(Err(e)) => {
                        warn!("初始化拉取最新消息失败: source={} err={}", source_key, e);
                        None
                    }
                    Err(_) => {
                        warn!(
                            "初始化拉取最新消息超时: source={} timeout={}s",
                            source_key,
                            request_timeout.as_secs()
                        );
                        None
                    }
                };

            let Some(message) = latest else {
                continue;
            };

            let start_id = message
                .id
                .saturating_sub(self.config.past_limit.saturating_sub(1) as i64);
            info!(
                "历史起点初始化: source={} latest_id={} start_id={} past_limit={}",
                source_key, message.id, start_id, self.config.past_limit
            );
            latest_start_ids.insert(source_key.clone(), start_id);
        }

        let mut timeout_window = TimeoutWindow::new(Duration::from_secs(120));
        let outcome = loop {
            if !self.client.is_runner_alive() {
                warn!("检测到 sender runner 已停止，准备重连");
                break RunOutcome::Reload;
            }

            if tokio::time::timeout(dispatcher_idle_timeout, self.dispatcher.wait_idle())
                .await
                .is_err()
            {
                warn!(
                    "等待发送任务完成超时: timeout={}s，准备重连",
                    dispatcher_idle_timeout.as_secs()
                );
                break RunOutcome::Reload;
            }
            info!("开始新一轮历史轮询...");

            let mut should_reload = false;
            for source in sources.clone() {
                if self.draining.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }

                let Some(peer) = self.get_source_peer(&source).await else {
                    continue;
                };

                let mut min_id = {
                    let state = self.state.lock().await;
                    state.get_min_id(&source)
                };
                if let Some(start_id) = latest_start_ids.get(&source) {
                    if min_id < *start_id {
                        min_id = *start_id;
                    }
                }

                let attempt_at = Instant::now();
                timeout_window.record_attempt(attempt_at);

                let mut messages = if min_id <= 0 {
                    match tokio::time::timeout(
                        request_timeout,
                        self.client.get_messages(peer, self.config.past_limit, 0),
                    )
                    .await
                    {
                        Ok(Ok(msgs)) => msgs,
                        Ok(Err(e)) => {
                            error!("拉取历史消息失败: {}", e);
                            continue;
                        }
                        Err(_) => {
                            warn!(
                                "拉取历史消息超时: source={} timeout={}s",
                                source,
                                request_timeout.as_secs()
                            );
                            timeout_window.record_timeout(Instant::now());
                            if let Some(rate) = timeout_window.should_reload(Instant::now()) {
                                warn!(
                                    "2分钟内历史拉取超时率过高: rate={:.2} timeouts={} attempts={}，准备热重启",
                                    rate,
                                    timeout_window.timeouts_len(),
                                    timeout_window.attempts_len()
                                );
                                should_reload = true;
                                break;
                            }
                            continue;
                        }
                    }
                } else {
                    match tokio::time::timeout(
                        request_timeout,
                        self.client
                            .get_messages_since(peer, min_id, self.config.past_limit),
                    )
                    .await
                    {
                        Ok(Ok(msgs)) => msgs,
                        Ok(Err(e)) => {
                            error!("拉取历史消息失败: {}", e);
                            continue;
                        }
                        Err(_) => {
                            warn!(
                                "拉取历史消息超时: source={} timeout={}s",
                                source,
                                request_timeout.as_secs()
                            );
                            timeout_window.record_timeout(Instant::now());
                            if let Some(rate) = timeout_window.should_reload(Instant::now()) {
                                warn!(
                                    "2分钟内历史拉取超时率过高: rate={:.2} timeouts={} attempts={}，准备热重启",
                                    rate,
                                    timeout_window.timeouts_len(),
                                    timeout_window.attempts_len()
                                );
                                should_reload = true;
                                break;
                            }
                            continue;
                        }
                    }
                };

                if should_reload {
                    break;
                }

                if !messages.is_empty() {
                    let inflight_ids = {
                        let inflight = self.inflight.lock().await;
                        inflight.get(&source).cloned()
                    };
                    if let Some(ids) = inflight_ids {
                        messages.retain(|msg| !ids.contains(&msg.id));
                    }
                }

                if messages.is_empty() {
                    continue;
                }

                let mut messages = messages;
                messages.reverse();

                for msg in messages {
                    if let Err(e) = self.handle_message_past(&msg, &source).await {
                        error!("处理消息失败: {}", e);
                    }
                }
            }
            if should_reload {
                break RunOutcome::Reload;
            }

            {
                let mut s = self.state.lock().await;
                if let Err(e) = s.flush(&state_file) {
                    error!("轮询后保存状态失败: {}", e);
                }
            }
            let snap = {
                let p = self.pipeline.lock().await;
                p.dedup_snapshot()
            };
            {
                let mut d = self.dedup.lock().await;
                d.set_fingerprints(snap.fingerprints);
                d.set_media_id_signatures(snap.media_id_signatures);
                d.set_media_meta_signatures(snap.media_meta_signatures);
                d.set_media_thumb_hashes(snap.media_thumb_hashes);
                d.set_recent_text_features(snap.recent_text_features);
                if let Err(e) = d.flush(&dedup_file) {
                    error!("轮询后保存去重记录失败: {}", e);
                }
            }

            tokio::select! {
                _ = tokio::time::sleep(poll_interval) => {}
                change = hotreload_rx.recv() => {
                    if change.is_some() {
                        info!("检测到配置或来源文件变更，停止历史模式以便重启...");
                        self.draining.store(true, std::sync::atomic::Ordering::Relaxed);
                        break RunOutcome::Reload;
                    }
                }
                reload = reload_rx.recv() => {
                    if reload.is_some() {
                        warn!("检测到发送网络错误，触发重连...");
                        self.draining.store(true, std::sync::atomic::Ordering::Relaxed);
                        break RunOutcome::Reload;
                    }
                    warn!("发送重连通道已关闭，触发重连...");
                    self.draining.store(true, std::sync::atomic::Ordering::Relaxed);
                    break RunOutcome::Reload;
                }
                _ = signal::ctrl_c() => {
                    info!("收到退出信号");
                    self.draining.store(true, std::sync::atomic::Ordering::Relaxed);
                    self.force_exit
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    break RunOutcome::Exit;
                }
            }
        };

        shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
        draining.store(true, std::sync::atomic::Ordering::Relaxed);
        state_task.abort();
        session_clean_task.abort();
        thumb_cleanup_task.abort();
        hotreload_task.abort();
        album_task.abort();
        text_merge_task.abort();

        let pending_albums = {
            let aggregator = self.album_aggregator.lock().await;
            aggregator.drain_all().await
        };
        for (key, mut messages) in pending_albums {
            if messages.is_empty() {
                continue;
            }
            messages.sort_by_key(|m| m.id);
            let min_id = messages.first().map(|m| m.id).unwrap_or(0);
            let merged = {
                let mut text_merge = self.text_merge.lock().await;
                if min_id > 0 {
                    text_merge.try_merge_for_album(&key.source_key, min_id)
                } else {
                    None
                }
            };
            normalize_album_messages(&mut messages, merged, self.config.append_limit_with_media);

            let task = ForwardTask {
                source_key: key.source_key,
                messages,
                is_album: true,
                dedup_infos: Vec::new(),
            };
            self.dispatcher.dispatch(task);
        }

        let pending_texts = {
            let mut text_merge = self.text_merge.lock().await;
            text_merge.take_all()
        };
        for (source_key, msg) in pending_texts {
            let task = ForwardTask {
                source_key,
                messages: vec![msg],
                is_album: false,
                dedup_infos: Vec::new(),
            };
            self.dispatcher.dispatch(task);
        }

        let skip_on_timeout = matches!(outcome, RunOutcome::Reload);
        let drained = self
            .wait_dispatcher_drain(dispatcher_drain_timeout, skip_on_timeout)
            .await;
        if skip_on_timeout && !drained && self.config.mode == "past" {
            self.mark_inflight_as_pending("dispatcher_timeout").await;
        }

        {
            let mut p = self.pipeline.lock().await;
            let pending = p.take_all_pending();
            p.commit_all_pending(&pending);
        }

        {
            let p = self.pipeline.lock().await;
            let mut d = self.dedup.lock().await;
            let snap = p.dedup_snapshot();
            d.set_fingerprints(snap.fingerprints);
            d.set_media_id_signatures(snap.media_id_signatures);
            d.set_media_meta_signatures(snap.media_meta_signatures);
            d.set_media_thumb_hashes(snap.media_thumb_hashes);
            d.set_recent_text_features(snap.recent_text_features);
        }

        {
            let mut s = self.state.lock().await;
            if let Err(e) = s.flush(&state_file) {
                error!("退出时保存状态失败: {}", e);
            } else {
                info!("状态已保存");
            }
        }
        {
            let mut d = self.dedup.lock().await;
            if let Err(e) = d.flush(&dedup_file) {
                error!("退出时保存去重记录失败: {}", e);
            } else {
                info!("去重记录已保存");
            }
        }

        {
            let mut skip_log = self.skip_log.lock().await;
            skip_log.flush_all();
        }

        Ok(outcome)
    }

    async fn handle_message(&mut self, message: &MessageView, source_key: &str) -> Result<()> {
        self.handle_message_internal(message, source_key, true)
            .await
    }

    async fn handle_message_past(&mut self, message: &MessageView, source_key: &str) -> Result<()> {
        self.handle_message_internal(message, source_key, true)
            .await
    }

    async fn handle_message_internal(
        &mut self,
        message: &MessageView,
        source_key: &str,
        mark_immediately: bool,
    ) -> Result<()> {
        {
            let mut skip_log = self.skip_log.lock().await;
            skip_log.flush_expired();
        }

        let message = message.clone();

        if message.is_edit {
            if message.album_id.is_some() {
                let updated = self
                    .album_aggregator
                    .lock()
                    .await
                    .update_if_cached(message.clone(), source_key)
                    .await;
                if updated {
                    debug!(
                        "消息编辑已更新相册缓存: source={} msg_id={}",
                        source_key, message.id
                    );
                    return Ok(());
                }
            }

            if message.media.is_none() && self.config.text_merge_window > 0.0 {
                let mut text_merge = self.text_merge.lock().await;
                if text_merge.update_cached_text(source_key, &message) {
                    debug!(
                        "消息编辑已更新文本缓存: source={} msg_id={}",
                        source_key, message.id
                    );
                    return Ok(());
                }
            }

            debug!(
                "消息编辑未命中缓存，转为新消息处理: source={} msg_id={}",
                source_key, message.id
            );
        }

        if mark_immediately {
            // 原子操作：检查并标记为已处理，避免竞态条件
            let state = self.state.lock().await;
            if state.is_processed(Some(source_key), message.id) {
                let mut skip_log = self.skip_log.lock().await;
                skip_log.record(source_key, message.id);
                return Ok(());
            }
        } else {
            let state = self.state.lock().await;
            if state.is_processed(Some(source_key), message.id) {
                let mut skip_log = self.skip_log.lock().await;
                skip_log.record(source_key, message.id);
                return Ok(());
            }
        }

        if mark_immediately {
            let mut inflight = self.inflight.lock().await;
            let entry = inflight
                .entry(source_key.to_string())
                .or_insert_with(HashSet::new);
            if !entry.insert(message.id) {
                debug!(
                    "消息正在处理中，跳过: source={} msg_id={}",
                    source_key, message.id
                );
                return Ok(());
            }
        }

        if message.album_id.is_some() {
            if let Some(album_id) = message.album_id {
                debug!(
                    "相册消息采样: source={} msg_id={} album_id={} media={} text_len={} is_edit={}",
                    source_key,
                    message.id,
                    album_id,
                    message
                        .media
                        .as_ref()
                        .map(|m| format!("{:?}", m.media_type))
                        .unwrap_or_else(|| "None".to_string()),
                    message.text.chars().count(),
                    message.is_edit
                );
            }
            let action = self
                .album_aggregator
                .lock()
                .await
                .add_message(message.clone(), source_key)
                .await;

            if let AlbumAction::Flush(mut messages) = action {
                if messages.is_empty() {
                    return Ok(());
                }

                let min_id = messages.iter().map(|m| m.id).min().unwrap_or(0);
                let max_id = messages.iter().map(|m| m.id).max().unwrap_or(0);
                let album_id = messages.first().and_then(|m| m.album_id).unwrap_or(0);
                info!(
                    "相册满员触发发送: source={} album_id={} count={} min_id={} max_id={}",
                    source_key,
                    album_id,
                    messages.len(),
                    min_id,
                    max_id
                );

                if album_id > 0 {
                    // 只有真实相册才需要补漏
                    let backfilled = self
                        .backfill_album_messages(source_key, album_id, min_id, max_id)
                        .await;
                    extend_unique_messages(&mut messages, backfilled);
                }

                messages.sort_by_key(|m| m.id);
                let min_id = messages.first().map(|m| m.id).unwrap_or(0);
                let merged = {
                    let mut text_merge = self.text_merge.lock().await;
                    // 只有真实相册才合并前置文本
                    if album_id > 0 {
                        text_merge.try_merge_for_album(source_key, min_id)
                    } else {
                        None
                    }
                };
                normalize_album_messages(
                    &mut messages,
                    merged,
                    self.config.append_limit_with_media,
                );

                // 真实相册：作为相册发送
                let task = ForwardTask {
                    source_key: source_key.to_string(),
                    messages,
                    is_album: true,
                    dedup_infos: Vec::new(),
                };
                self.dispatcher.dispatch(task);
            }
        } else {
            // 注意：消息已在上面的原子操作中标记为已处理

            if message.media.is_none() && self.config.text_merge_window > 0.0 {
                let mut text_merge = self.text_merge.lock().await;
                if text_merge.cache_text(source_key, &message) {
                    return Ok(());
                }
            }

            let task = ForwardTask {
                source_key: source_key.to_string(),
                messages: vec![message.clone()],
                is_album: false,
                dedup_infos: Vec::new(),
            };
            self.dispatcher.dispatch(task);
        }

        Ok(())
    }
}

fn extend_unique_messages(messages: &mut Vec<MessageView>, extra: Vec<MessageView>) {
    if extra.is_empty() {
        return;
    }

    let mut index_by_id = std::collections::HashMap::new();
    for (idx, msg) in messages.iter().enumerate() {
        index_by_id.insert(msg.id, idx);
    }

    for msg in extra {
        if let Some(&idx) = index_by_id.get(&msg.id) {
            merge_message(&mut messages[idx], msg);
        } else {
            index_by_id.insert(msg.id, messages.len());
            messages.push(msg);
        }
    }
}

fn merge_message(base: &mut MessageView, incoming: MessageView) {
    let base_text = base.text.trim();
    let incoming_text = incoming.text.trim();
    let replace_text = if incoming_text.is_empty() {
        false
    } else if base_text.is_empty() {
        true
    } else {
        incoming_text.chars().count() > base_text.chars().count()
    };

    if replace_text {
        base.text = incoming.text;
        base.entities = incoming.entities;
    }

    if base.media.is_none() && incoming.media.is_some() {
        base.media = incoming.media;
    }
    if base.album_id.is_none() && incoming.album_id.is_some() {
        base.album_id = incoming.album_id;
    }
    if base.chat_id == 0 && incoming.chat_id != 0 {
        base.chat_id = incoming.chat_id;
    }
}

fn build_message_link(source_key: &str, chat_id: i64, msg_id: i64) -> Option<String> {
    if msg_id <= 0 {
        return None;
    }

    if chat_id <= -1000000000000 {
        let channel_id = -chat_id - 1000000000000;
        return Some(format!("https://t.me/c/{}/{}", channel_id, msg_id));
    }

    let key = TelegramForwarder::sanitize_identifier(source_key);
    if !key.is_empty() {
        if let Ok(id) = key.parse::<i64>() {
            if id <= -1000000000000 {
                let channel_id = -id - 1000000000000;
                return Some(format!("https://t.me/c/{}/{}", channel_id, msg_id));
            }
        } else {
            return Some(format!("https://t.me/{}/{}", key, msg_id));
        }
    }

    None
}

fn normalize_album_messages(
    messages: &mut [MessageView],
    merged: Option<(String, Vec<TextEntity>)>,
    _caption_limit: usize,
) {
    if messages.is_empty() {
        return;
    }

    let first = &mut messages[0];

    if let Some((merged_text, merged_entities)) = merged {
        // 有前置文本需要合并
        if merged_text.trim().is_empty() {
            // merged text 为空，保持原有 first.text 和 first.entities 不变
            return;
        }

        // 将缓存的文本追加到相册文本之后（尾部），而不是头部
        let mut new_text: String = first.text.clone();
        let mut new_entities = first.entities.clone();

        if !new_text.trim().is_empty() {
            new_text.push_str("\n\n");
        }

        let offset = common::utf16::utf16_len(&new_text) as i32;
        new_text.push_str(&merged_text);
        for entity in &merged_entities {
            let mut new_entity = entity.clone();
            new_entity.offset += offset;
            new_entities.push(new_entity);
        }

        // Defer trimming to entity_trim after pipeline so quote text can be removed, not just unformatted.

        first.text = new_text;
        first.entities = new_entities;
    }
    // merged 为 None 时，保持 first.text 和 first.entities 不变
}

fn compute_backfill_range(min_id: i64, max_id: i64, max_range: usize) -> Option<(i64, i64)> {
    if max_range == 0 || min_id <= 0 || max_id <= 0 || max_id < min_id {
        return None;
    }

    let range = max_id.saturating_sub(min_id).saturating_add(1);
    if range == 0 || range as usize > max_range {
        return None;
    }

    let extra = max_range as i64 - range;
    let extra_before = extra / 2;
    let extra_after = extra - extra_before;

    let start_id = min_id.saturating_sub(extra_before).max(1);
    let end_id = max_id.saturating_add(extra_after);

    Some((start_id, end_id))
}

async fn apply_pipeline_to_task(
    pipeline: &Arc<tokio::sync::Mutex<Pipeline>>,
    task: ForwardTask,
) -> Option<ForwardTask> {
    let mut pipeline = pipeline.lock().await;
    let mut processed_messages = Vec::new();
    let mut dedup_infos = Vec::new();

    let ForwardTask {
        source_key,
        messages,
        is_album,
        ..
    } = task;

    if is_album {
        let album_link = messages
            .iter()
            .min_by_key(|m| m.id)
            .and_then(|m| build_message_link(&source_key, m.chat_id, m.id))
            .unwrap_or_else(|| "无".to_string());

        let result = pipeline
            .process_album(&messages, Some(album_link.as_str()))
            .await;

        let (mut processed_msgs, dedup_info) = match result {
            Some(res) => res,
            None => return None,
        };

        if let Some(info) = dedup_info {
            dedup_infos.push(info);
        }

        for (raw_msg, msg) in messages.iter().zip(processed_msgs.iter()) {
            let raw_preview = truncate_text(&raw_msg.text, 100);
            let message_link = build_message_link(&source_key, raw_msg.chat_id, raw_msg.id)
                .unwrap_or_else(|| "无".to_string());
            info!(
                "消息通过处理: source={} link={} 原文=\"{}\" 处理后=\"{}\" media={}",
                source_key,
                message_link,
                raw_preview,
                truncate_text(&msg.text, 120),
                msg.media
                    .as_ref()
                    .map(|m| format!("{:?}", m.media_type))
                    .unwrap_or_else(|| "None".to_string())
            );
        }

        if let Some(first) = processed_msgs.first_mut() {
            let caption_limit = pipeline.append_limit_with_media;
            if first.media.is_some() && common::utf16::utf16_len(&first.text) > caption_limit {
                let header_len = common::utf16::utf16_len(&pipeline.appender.header);
                let footer_len = common::utf16::utf16_len(&pipeline.appender.footer);
                let (trimmed_text, trimmed_entities) =
                    tg_core::entity_trim::trim_long_entities_if_needed(
                        &first.text,
                        &first.entities,
                        caption_limit,
                        header_len,
                        footer_len,
                    );
                first.text = trimmed_text;
                first.entities = trimmed_entities;
            }
        }

        return Some(ForwardTask {
            source_key,
            messages: processed_msgs,
            is_album,
            dedup_infos,
        });
    }

    for (idx, mut msg) in messages.into_iter().enumerate() {
        let raw_preview = truncate_text(&msg.text, 100);
        let message_link = build_message_link(&source_key, msg.chat_id, msg.id)
            .unwrap_or_else(|| "无".to_string());
        let apply_append = !(is_album && idx > 0);
        let result = pipeline
            .process(&msg, apply_append, Some(message_link.as_str()))
            .await;

        if !result.should_forward {
            continue;
        }

        if let Some(info) = result.dedup_info {
            dedup_infos.push(info);
        }

        msg.text = result.text;
        msg.entities = result.entities;

        let caption_limit = pipeline.append_limit_with_media;
        if msg.media.is_some() && common::utf16::utf16_len(&msg.text) > caption_limit {
            let header_len = common::utf16::utf16_len(&pipeline.appender.header);
            let footer_len = common::utf16::utf16_len(&pipeline.appender.footer);
            let (trimmed_text, trimmed_entities) =
                tg_core::entity_trim::trim_long_entities_if_needed(
                    &msg.text,
                    &msg.entities,
                    caption_limit,
                    header_len,
                    footer_len,
                );
            msg.text = trimmed_text;
            msg.entities = trimmed_entities;
        }

        msg.media = result.media_list.first().cloned();

        info!(
            "消息通过处理: source={} link={} 原文=\"{}\" 处理后=\"{}\" media={}",
            source_key,
            message_link,
            raw_preview,
            truncate_text(&msg.text, 120),
            msg.media
                .as_ref()
                .map(|m| format!("{:?}", m.media_type))
                .unwrap_or_else(|| "None".to_string())
        );

        processed_messages.push(msg);
    }

    if processed_messages.is_empty() {
        return None;
    }

    // 关键修复：如果任务本身不是相册（单条发送），必须清除消息携带的 album_id，
    // 否则 tdlib/send.rs 会因为数据不一致（有album_id但不是album任务）而拒绝发送。
    if !is_album {
        for msg in &mut processed_messages {
            msg.album_id = None;
        }
    }

    Some(ForwardTask {
        source_key,
        messages: processed_messages,
        is_album,
        dedup_infos,
    })
}

fn main() -> Result<()> {
    // 增加栈大小到 16MB 以防止 grammers-mtsender 在网络超时时的栈溢出
    // 问题：grammers-mtsender v0.8 在处理多个失败请求时存在栈累积问题
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(4)
        .thread_stack_size(16 * 1024 * 1024)
        .build()
        .unwrap()
        .block_on(async_main())
}

async fn async_main() -> Result<()> {
    dotenv().ok();
    loop {
        let forwarder = match TelegramForwarder::new().await {
            Ok(f) => f,
            Err(e) => {
                error!("初始化转发器失败: {}", e);
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                continue;
            }
        };

        let mut forwarder = forwarder;
        match forwarder.run().await {
            Ok(RunOutcome::Exit) => break,
            Ok(RunOutcome::Reload) => {
                info!("热重启完成，正在重新加载配置并重启...");
                continue;
            }
            Err(e) => {
                let err_msg = e.to_string().to_lowercase();
                let should_reconnect = err_msg.contains("read error")
                    || err_msg.contains("io failed")
                    || err_msg.contains("connection")
                    || err_msg.contains("timeout")
                    || err_msg.contains("timed out")
                    || err_msg.contains("network")
                    || err_msg.contains("reset")
                    || err_msg.contains("broken pipe")
                    || err_msg.contains("transport")
                    || err_msg.contains("rpc");

                if should_reconnect {
                    error!("网络错误: {}, 5秒后重新连接", e);
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    continue;
                } else {
                    error!("非网络错误，退出: {}", e);
                    return Err(e);
                }
            }
        }
    }
    Ok(())
}
