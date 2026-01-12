use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;
use tracing_subscriber::{fmt, EnvFilter};

static LOG_GUARD: OnceLock<WorkerGuard> = OnceLock::new();
static GAP_LOG_FLUSH: OnceLock<()> = OnceLock::new();
static GAP_DEBUG: OnceLock<bool> = OnceLock::new();

pub type LogGuard = WorkerGuard;

#[derive(Default)]
struct GapLogAggregator {
    inner: std::sync::Mutex<HashMap<i64, GapLogStat>>,
}

#[derive(Clone, Copy)]
struct GapLogStat {
    count: usize,
    local_min: i64,
    remote_max: i64,
}

impl GapLogAggregator {
    fn record(&self, item: GapLogItem) {
        let mut inner = self.inner.lock().unwrap();
        let entry = inner.entry(item.channel_id).or_insert(GapLogStat {
            count: 0,
            local_min: item.local,
            remote_max: item.remote,
        });

        entry.count += 1;
        if item.local < entry.local_min {
            entry.local_min = item.local;
        }
        if item.remote > entry.remote_max {
            entry.remote_max = item.remote;
        }
    }

    fn drain(&self) -> Vec<(i64, GapLogStat)> {
        let mut inner = self.inner.lock().unwrap();
        inner.drain().collect()
    }
}

struct GapLogLayer {
    aggregator: Arc<GapLogAggregator>,
}

impl GapLogLayer {
    fn new(aggregator: Arc<GapLogAggregator>) -> Self {
        Self { aggregator }
    }
}

impl<S> Layer<S> for GapLogLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        if let Some(item) = extract_gap_item(event) {
            if *GAP_DEBUG.get().unwrap_or(&false) {
                eprintln!(
                    "[GAP_DEBUG] 捕获 gap 日志: channel={}, local={}, remote={}",
                    item.channel_id, item.local, item.remote
                );
            }
            self.aggregator.record(item);
        }
    }
}

#[derive(Clone)]
struct GapFilterFormat<F> {
    inner: F,
}

impl<F> GapFilterFormat<F> {
    fn new(inner: F) -> Self {
        Self { inner }
    }
}

impl<S, N, F> FormatEvent<S, N> for GapFilterFormat<F>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
    F: FormatEvent<S, N>,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        writer: Writer<'_>,
        event: &Event<'_>,
    ) -> std::fmt::Result {
        if let Some(item) = extract_gap_item(event) {
            if *GAP_DEBUG.get().unwrap_or(&false) {
                eprintln!(
                    "[GAP_DEBUG] 过滤 gap 日志: channel={}, local={}, remote={}",
                    item.channel_id, item.local, item.remote
                );
            }
            return Ok(());
        }

        self.inner.format_event(ctx, writer, event)
    }
}

#[derive(Clone, Copy)]
struct GapLogItem {
    channel_id: i64,
    local: i64,
    remote: i64,
}

struct MessageVisitor {
    message: Option<String>,
}

impl Visit for MessageVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() != "message" || self.message.is_some() {
            return;
        }

        let mut msg = format!("{:?}", value);
        if msg.len() >= 2 && msg.starts_with('"') && msg.ends_with('"') {
            msg = msg[1..msg.len() - 1].to_string();
        }
        self.message = Some(msg);
    }
}

fn extract_gap_item(event: &Event<'_>) -> Option<GapLogItem> {
    if event.metadata().target() != "grammers_session::message_box" {
        return None;
    }

    let mut visitor = MessageVisitor { message: None };
    event.record(&mut visitor);
    let message = visitor.message?;

    parse_gap_message(&message)
}

fn parse_gap_message(message: &str) -> Option<GapLogItem> {
    let prefix = "gap on update for Channel(";
    let rest = message.strip_prefix(prefix)?;
    let (channel_str, rest) = rest.split_once(')')?;
    let channel_id = channel_str.parse::<i64>().ok()?;
    let local = parse_number_after(rest, "local ")?;
    let remote = parse_number_after(rest, "remote ")?;

    Some(GapLogItem {
        channel_id,
        local,
        remote,
    })
}

fn parse_number_after(text: &str, key: &str) -> Option<i64> {
    let start = text.find(key)? + key.len();
    let rest = text[start..].trim_start();
    let mut end = 0;
    for (idx, ch) in rest.char_indices() {
        if !ch.is_ascii_digit() {
            break;
        }
        end = idx + ch.len_utf8();
    }
    if end == 0 {
        return None;
    }
    rest[..end].parse().ok()
}

fn start_gap_flush_task(aggregator: Arc<GapLogAggregator>) {
    if GAP_LOG_FLUSH.set(()).is_err() {
        return;
    }

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
        loop {
            interval.tick().await;
            let items = aggregator.drain();
            if items.is_empty() {
                if *GAP_DEBUG.get().unwrap_or(&false) {
                    eprintln!("[GAP_DEBUG] 汇总周期: 无 gap 记录");
                }
                continue;
            }

            if *GAP_DEBUG.get().unwrap_or(&false) {
                eprintln!("[GAP_DEBUG] 汇总周期: {} 个频道的 gap 记录", items.len());
            }

            for (channel_id, stat) in items {
                tracing::info!(
                    target: "app::gap",
                    "更新缺口汇总: 频道={} 次数={} 本地最小={} 远端最大={}",
                    channel_id,
                    stat.count,
                    stat.local_min,
                    stat.remote_max
                );
            }
        }
    });
}

fn ensure_log_dir(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

pub fn init(log_file: &Path, level: &str) -> &'static WorkerGuard {
    LOG_GUARD.get_or_init(|| {
        ensure_log_dir(log_file).expect("无法创建日志目录");

        let gap_debug = std::env::var("GAP_DEBUG").is_ok();
        GAP_DEBUG.get_or_init(|| gap_debug);

        if gap_debug {
            eprintln!("[GAP_DEBUG] gap 调试模式已启用");
        }

        let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new(level)
                .add_directive("grammers_mtsender=warn".parse().unwrap())
                .add_directive("grammers_mtproto=info".parse().unwrap())
                .add_directive("grammers_session=warn".parse().unwrap())
        });

        let (non_blocking, guard) = tracing_appender::non_blocking(
            std::fs::File::create(log_file).expect("无法创建日志文件"),
        );

        let gap_aggregator = Arc::new(GapLogAggregator::default());
        let gap_layer = GapLogLayer::new(gap_aggregator.clone());
        let gap_format = GapFilterFormat::new(fmt::format());

        tracing_subscriber::registry()
            .with(env_filter)
            .with(gap_layer)
            .with(
                fmt::layer()
                    .event_format(gap_format.clone())
                    .with_writer(std::io::stdout),
            )
            .with(
                fmt::layer()
                    .event_format(gap_format)
                    .with_ansi(false)
                    .with_writer(non_blocking),
            )
            .init();

        start_gap_flush_task(gap_aggregator);
        guard
    })
}
