use anyhow::{anyhow, Context, Result};
use grammers_client::{
    grammers_tl_types as tl,
    types::{Downloadable, Peer, Update},
    Client, SignInError, UpdatesConfiguration,
};
use grammers_mtsender::SenderPool;
use grammers_session::{defs::PeerRef, storages::SqliteSession, updates::UpdatesLike};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration as StdDuration, Instant};
use tokio::{
    sync::mpsc,
    task::JoinHandle,
    time::{timeout, Duration},
};
use tracing::{debug, error, info, warn};

use super::model::{EntityType, MediaType, MediaView, MessageView, TextEntity};
use grammers_client::types::photo_sizes::{PhotoSize, VecExt};
use tg_core::catch_up::{CatchUpFetcher, CatchUpFuture};

const HISTORY_PAGE_LIMIT: usize = 100;
const SERVICE_SKIP_MAX_COUNT: usize = 50;
const SERVICE_SKIP_MAX_AGE: StdDuration = StdDuration::from_secs(2);

#[derive(Clone, Copy)]
struct ServiceSkipEntry {
    chat_id: i64,
    min_id: i64,
    max_id: i64,
    count: usize,
}

struct ServiceSkipLog {
    chat_id: Option<i64>,
    min_id: i64,
    max_id: i64,
    count: usize,
    last_flush: Instant,
}

impl ServiceSkipLog {
    fn new() -> Self {
        Self {
            chat_id: None,
            min_id: 0,
            max_id: 0,
            count: 0,
            last_flush: Instant::now(),
        }
    }

    fn record(&mut self, chat_id: i64, msg_id: i64) -> Option<ServiceSkipEntry> {
        let now = Instant::now();
        match self.chat_id {
            None => {
                self.reset(chat_id, msg_id, now);
                return None;
            }
            Some(current) if current != chat_id => {
                let entry = self.entry();
                self.reset(chat_id, msg_id, now);
                return Some(entry);
            }
            _ => {}
        }

        if msg_id < self.min_id {
            self.min_id = msg_id;
        }
        if msg_id > self.max_id {
            self.max_id = msg_id;
        }
        self.count += 1;

        if self.count >= SERVICE_SKIP_MAX_COUNT
            || now.duration_since(self.last_flush) >= SERVICE_SKIP_MAX_AGE
        {
            let entry = self.entry();
            self.clear(now);
            return Some(entry);
        }

        None
    }

    fn entry(&self) -> ServiceSkipEntry {
        ServiceSkipEntry {
            chat_id: self.chat_id.unwrap_or_default(),
            min_id: self.min_id,
            max_id: self.max_id,
            count: self.count,
        }
    }

    fn reset(&mut self, chat_id: i64, msg_id: i64, now: Instant) {
        self.chat_id = Some(chat_id);
        self.min_id = msg_id;
        self.max_id = msg_id;
        self.count = 1;
        self.last_flush = now;
    }

    fn clear(&mut self, now: Instant) {
        self.chat_id = None;
        self.min_id = 0;
        self.max_id = 0;
        self.count = 0;
        self.last_flush = now;
    }
}

static SERVICE_SKIP_LOG: OnceLock<Mutex<ServiceSkipLog>> = OnceLock::new();
static EMPTY_CONTENT_SKIP_LOG: OnceLock<Mutex<ServiceSkipLog>> = OnceLock::new();

fn log_service_skip(chat_id: i64, msg_id: i64) {
    log_skip(&SERVICE_SKIP_LOG, "服务消息批量跳过", chat_id, msg_id, true);
}

fn log_empty_content_skip(chat_id: i64, msg_id: i64) {
    log_skip(
        &EMPTY_CONTENT_SKIP_LOG,
        "无内容消息批量跳过",
        chat_id,
        msg_id,
        false,
    );
}

fn log_skip(
    logger: &OnceLock<Mutex<ServiceSkipLog>>,
    label: &str,
    chat_id: i64,
    msg_id: i64,
    use_info: bool,
) {
    let logger = logger.get_or_init(|| Mutex::new(ServiceSkipLog::new()));
    let mut guard = match logger.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    if let Some(entry) = guard.record(chat_id, msg_id) {
        let range = if entry.min_id == entry.max_id {
            entry.min_id.to_string()
        } else {
            format!("{}-{}", entry.min_id, entry.max_id)
        };
        if use_info {
            info!(
                "{}: chat_id={} count={} ids={}",
                label, entry.chat_id, entry.count, range
            );
        } else {
            debug!(
                "{}: chat_id={} count={} ids={}",
                label, entry.chat_id, entry.count, range
            );
        }
    }
}

#[derive(Clone)]
pub struct TdlibClient {
    client: Client,
    api_hash: String,
    updates_rx: Arc<tokio::sync::Mutex<Option<mpsc::UnboundedReceiver<UpdatesLike>>>>,
    _runner: Arc<RunnerGuard>,
    runner_alive: Arc<AtomicBool>,
}

struct RunnerGuard {
    handle: JoinHandle<()>,
}

impl Drop for RunnerGuard {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

impl TdlibClient {
    pub async fn connect(api_id: i32, api_hash: &str, session_name: &str) -> Result<Self> {
        let session = Arc::new(SqliteSession::open(session_name)?);
        let pool = SenderPool::new(session, api_id);
        let client = Client::new(&pool);

        let runner_alive = Arc::new(AtomicBool::new(true));
        let runner_alive_clone = runner_alive.clone();

        let runner = tokio::spawn(async move {
            // pool.runner.run() 返回 ()，会持续运行直到网络断开或错误
            pool.runner.run().await;
            // Runner 退出时，标记为不再存活
            runner_alive_clone.store(false, Ordering::Release);
            warn!("SenderPool runner 已退出（可能是网络断开或连接错误）");
        });

        Ok(Self {
            client,
            api_hash: api_hash.to_string(),
            updates_rx: Arc::new(tokio::sync::Mutex::new(Some(pool.updates))),
            _runner: Arc::new(RunnerGuard { handle: runner }),
            runner_alive,
        })
    }

    /// 检查 runner 是否仍然存活
    pub fn is_runner_alive(&self) -> bool {
        self.runner_alive.load(Ordering::Acquire) && !self._runner.handle.is_finished()
    }

    pub async fn is_authorized(&self) -> Result<bool> {
        Ok(self.client.is_authorized().await?)
    }

    pub async fn authorize(&self) -> Result<()> {
        if self.is_authorized().await? {
            return Ok(());
        }

        let phone = tokio::task::block_in_place(|| {
            println!("请输入手机号（带国际区号，例如 +8613800138000）：");
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            Ok::<String, std::io::Error>(input.trim().to_string())
        })?;

        let token = self.client.request_login_code(&phone, &self.api_hash);

        info!("正在请求验证码，请稍候...");
        let token = timeout(Duration::from_secs(25), token)
            .await
            .context("请求验证码超时，请检查网络或代理")??;
        info!("验证码已发送，若未收到请检查设备或等待片刻");

        let code = tokio::task::block_in_place(|| {
            println!("请输入验证码：");
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            Ok::<String, std::io::Error>(input.trim().to_string())
        })?;

        let sign_in = timeout(Duration::from_secs(25), self.client.sign_in(&token, &code))
            .await
            .context("登录请求超时，请检查网络或代理")?;

        match sign_in {
            Ok(_) => {}
            Err(SignInError::PasswordRequired(token)) => {
                let password = tokio::task::block_in_place(|| {
                    println!("请输入二步验证密码：");
                    let mut input = String::new();
                    std::io::stdin().read_line(&mut input)?;
                    Ok::<String, std::io::Error>(input.trim().to_string())
                })?;

                timeout(
                    Duration::from_secs(25),
                    self.client
                        .check_password(token, password.as_bytes().to_vec()),
                )
                .await
                .context("二步验证超时，请检查网络或代理")??;
            }
            Err(e) => {
                error!("登录失败: {}", e);
                return Err(e.into());
            }
        }

        Ok(())
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub async fn resolve_chat(&self, identifier: &str) -> Result<Peer> {
        if let Ok(dialog_id) = identifier.parse::<i64>() {
            let peer_ref = dialog_id_to_peer(dialog_id);
            let peer = self.client.resolve_peer(peer_ref).await?;
            return Ok(peer);
        }

        let username = identifier.trim_start_matches('@');
        let peer = self
            .client
            .resolve_username(username)
            .await?
            .ok_or_else(|| anyhow!("未找到用户或频道: {}", identifier))?;
        Ok(peer)
    }

    pub async fn get_messages(
        &self,
        peer: PeerRef,
        limit: usize,
        min_id: i64,
    ) -> Result<Vec<MessageView>> {
        let mut iter = self.client.iter_messages(peer).limit(limit);
        let mut result = Vec::new();

        while let Some(msg) = iter.next().await? {
            if (msg.id() as i64) <= min_id {
                continue;
            }
            if let Ok(view) = Self::convert_message(&msg) {
                result.push(view);
            }
        }

        Ok(result)
    }

    pub async fn get_messages_since(
        &self,
        peer: PeerRef,
        min_id: i64,
        page_limit: usize,
    ) -> Result<Vec<MessageView>> {
        let mut result = Vec::new();
        let limit = clamp_history_page_limit(page_limit);
        let mut offset_id: i32 = 0;

        loop {
            let mut iter = self.client.iter_messages(peer).limit(limit);
            if offset_id > 0 {
                iter = iter.offset_id(offset_id);
            }

            let mut page_count = 0usize;
            let mut reached_min = false;
            while let Some(msg) = iter.next().await? {
                page_count += 1;
                let msg_id = msg.id() as i64;
                if msg_id <= min_id {
                    reached_min = true;
                    break;
                }

                if let Ok(view) = Self::convert_message(&msg) {
                    result.push(view);
                }

                offset_id = msg.id();
            }

            if reached_min || page_count < limit {
                break;
            }
        }

        Ok(result)
    }

    pub async fn get_messages_by_id(
        &self,
        peer: PeerRef,
        message_ids: &[i64],
    ) -> Result<Vec<MessageView>> {
        if message_ids.is_empty() {
            return Ok(Vec::new());
        }

        let ids: Vec<i32> = message_ids
            .iter()
            .filter_map(|id| i32::try_from(*id).ok())
            .collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let messages = self.client.get_messages_by_id(peer, &ids).await?;
        let mut result = Vec::new();
        for msg in messages.into_iter().flatten() {
            if let Ok(view) = Self::convert_message(&msg) {
                result.push(view);
            }
        }

        Ok(result)
    }

    pub async fn subscribe_updates(&self) -> mpsc::UnboundedReceiver<MessageView> {
        let updates = {
            let mut guard = self.updates_rx.lock().await;
            guard.take().expect("更新通道已被消费，无法重复订阅")
        };

        let (sender, receiver) = mpsc::unbounded_channel();
        let client = self.client.clone();
        let client_clone = self.clone(); // Clone TdlibClient to monitor runner status

        tokio::spawn(async move {
            let mut stream = client.stream_updates(
                updates,
                UpdatesConfiguration {
                    catch_up: true,
                    ..Default::default()
                },
            );

            let mut consecutive_errors = 0u32;
            const MAX_CONSECUTIVE_ERRORS: u32 = 10;

            loop {
                match stream.next().await {
                    Ok(Update::NewMessage(msg)) => {
                        consecutive_errors = 0;
                        if let Ok(view) = TdlibClient::convert_message(&msg) {
                            let _ = sender.send(view);
                        }
                    }
                    Ok(Update::MessageEdited(msg)) => {
                        consecutive_errors = 0;
                        if let Ok(view) = TdlibClient::convert_message_with_flag(&msg, true) {
                            let _ = sender.send(view);
                        } else {
                            debug!(
                                "消息编辑: 解析失败 chat_id={} msg_id={}",
                                msg.peer_id().bot_api_dialog_id(),
                                msg.id()
                            );
                        }
                    }
                    Ok(Update::MessageDeleted(_)) => {
                        consecutive_errors = 0;
                        tracing::info!("消息删除: 跳过");
                    }
                    Ok(_) => {
                        consecutive_errors = 0;
                    }
                    Err(e) => {
                        consecutive_errors += 1;
                        tracing::error!("更新流错误 ({}): {}", consecutive_errors, e);

                        // 检查 runner 是否还存活
                        if !client_clone.is_runner_alive() {
                            tracing::error!("Runner 已崩溃，更新流无法继续");
                            let _ = sender.send(MessageView {
                                id: 0,
                                chat_id: 0,
                                text: String::new(),
                                entities: Vec::new(),
                                media: None,
                                album_id: None,
                                is_edit: true, // 使用 is_edit=true 标记为特殊消息
                            });
                            break;
                        }

                        // 连续错误过多，可能流已损坏
                        if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                            tracing::error!(
                                "连续错误次数过多 ({})，更新流可能已损坏",
                                consecutive_errors
                            );
                            break;
                        }

                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    }
                }
            }

            // 循环退出，关闭发送端
            drop(sender);
            tracing::warn!("更新流已停止");
        });

        receiver
    }

    fn convert_message(msg: &grammers_client::types::Message) -> Result<MessageView> {
        Self::convert_message_with_flag(msg, false)
    }

    fn convert_message_with_flag(
        msg: &grammers_client::types::Message,
        is_edit: bool,
    ) -> Result<MessageView> {
        let chat_id = msg.peer_id().bot_api_dialog_id();

        if let Some(action) = msg.action() {
            let is_service_action = matches!(
                action,
                tl::enums::MessageAction::PinMessage
                    | tl::enums::MessageAction::ChannelCreate(_)
                    | tl::enums::MessageAction::ChatCreate(_)
                    | tl::enums::MessageAction::SetMessagesTtl(_)
                    | tl::enums::MessageAction::GroupCall(_)
                    | tl::enums::MessageAction::GroupCallScheduled(_)
                    | tl::enums::MessageAction::InviteToGroupCall(_)
            );
            if is_service_action {
                log_service_skip(chat_id, msg.id() as i64);
                return Err(anyhow!("服务消息: {}", msg.id()));
            }
        }

        let text = msg.text().to_string();

        let entities = msg
            .fmt_entities()
            .map(|ents| ents.iter().filter_map(Self::convert_entity).collect())
            .unwrap_or_default();

        let media = match msg.media() {
            Some(grammers_client::types::Media::Photo(photo)) => {
                let thumbs = photo.thumbs();
                let (size_bytes, width, height) = thumbs
                    .largest()
                    .map(|t| {
                        let (w, h) = match t {
                            grammers_client::types::photo_sizes::PhotoSize::Size(s) => {
                                (Some(s.width), Some(s.height))
                            }
                            grammers_client::types::photo_sizes::PhotoSize::Progressive(s) => {
                                (Some(s.width), Some(s.height))
                            }
                            _ => (None, None),
                        };
                        (t.size(), w, h)
                    })
                    .unwrap_or((0, None, None));
                let (thumb_type, thumb_bytes) = select_thumb_meta(&thumbs);

                let raw = match &photo.raw.photo {
                    Some(tl::enums::Photo::Photo(p)) => p,
                    _ => {
                        return Ok(MessageView {
                            id: msg.id() as i64,
                            chat_id,
                            text,
                            entities,
                            media: None,
                            album_id: msg.grouped_id(),
                            is_edit,
                        })
                    }
                };

                Some(MediaView {
                    media_type: MediaType::Photo,
                    size_bytes,
                    file_name: None,
                    mime_type: None,
                    width,
                    height,
                    duration: None,
                    media_id: Some(raw.id),
                    access_hash: raw.access_hash,
                    file_reference: raw.file_reference.clone(),
                    thumb_type,
                    thumb_bytes,
                    spoiler: photo.is_spoiler(),
                })
            }
            Some(grammers_client::types::Media::Document(doc)) => {
                let (width, height) = doc.resolution().unwrap_or((0, 0));
                let thumbs = doc.thumbs();
                let (thumb_type, thumb_bytes) = select_thumb_meta(&thumbs);
                Some(MediaView {
                    media_type: if doc
                        .mime_type()
                        .map(|m| m.starts_with("video/"))
                        .unwrap_or(false)
                    {
                        MediaType::Video
                    } else {
                        MediaType::Document
                    },
                    size_bytes: doc.size() as usize,
                    file_name: {
                        let name = doc.name();
                        if name.is_empty() {
                            None
                        } else {
                            Some(name.to_string())
                        }
                    },
                    mime_type: doc.mime_type().map(|m| m.to_string()),
                    width: if width == 0 { None } else { Some(width) },
                    height: if height == 0 { None } else { Some(height) },
                    duration: doc.duration().map(|d| d as i32),
                    media_id: Some(doc.id()),
                    access_hash: {
                        let raw = doc.raw.document.as_ref().and_then(|d| match d {
                            tl::enums::Document::Document(inner) => Some(inner.access_hash),
                            _ => None,
                        });
                        raw.unwrap_or_default()
                    },
                    file_reference: {
                        let raw = doc.raw.document.as_ref().and_then(|d| match d {
                            tl::enums::Document::Document(inner) => {
                                Some(inner.file_reference.clone())
                            }
                            _ => None,
                        });
                        raw.unwrap_or_default()
                    },
                    thumb_type,
                    thumb_bytes,
                    spoiler: doc.is_spoiler(),
                })
            }
            _ => None,
        };

        let album_id = msg.grouped_id();

        if text.trim().is_empty() && media.is_none() {
            log_empty_content_skip(chat_id, msg.id() as i64);
            return Err(anyhow!("无内容消息: {}", msg.id()));
        }

        Ok(MessageView {
            id: msg.id() as i64,
            chat_id,
            text,
            entities,
            media,
            album_id,
            is_edit,
        })
    }

    fn convert_entity(entity: &tl::enums::MessageEntity) -> Option<TextEntity> {
        let (offset, length, entity_type, data) = match entity {
            tl::enums::MessageEntity::Bold(e) => (e.offset, e.length, EntityType::Bold, None),
            tl::enums::MessageEntity::Italic(e) => (e.offset, e.length, EntityType::Italic, None),
            tl::enums::MessageEntity::Underline(e) => {
                (e.offset, e.length, EntityType::Underline, None)
            }
            tl::enums::MessageEntity::Strike(e) => {
                (e.offset, e.length, EntityType::Strikethrough, None)
            }
            tl::enums::MessageEntity::Code(e) => (e.offset, e.length, EntityType::Code, None),
            tl::enums::MessageEntity::Pre(e) => (
                e.offset,
                e.length,
                EntityType::Pre,
                Some(e.language.clone()),
            ),
            tl::enums::MessageEntity::TextUrl(e) => {
                (e.offset, e.length, EntityType::TextUrl, Some(e.url.clone()))
            }
            tl::enums::MessageEntity::Mention(e) => (e.offset, e.length, EntityType::Mention, None),
            tl::enums::MessageEntity::Hashtag(e) => (e.offset, e.length, EntityType::Hashtag, None),
            tl::enums::MessageEntity::Spoiler(e) => (e.offset, e.length, EntityType::Spoiler, None),
            tl::enums::MessageEntity::Blockquote(e) => {
                let data = if e.collapsed {
                    Some("true".to_string())
                } else {
                    None
                };
                (e.offset, e.length, EntityType::Blockquote, data)
            }
            tl::enums::MessageEntity::Url(e) => (e.offset, e.length, EntityType::Url, None),
            tl::enums::MessageEntity::Email(e) => (e.offset, e.length, EntityType::Email, None),
            tl::enums::MessageEntity::Phone(e) => (e.offset, e.length, EntityType::Phone, None),
            tl::enums::MessageEntity::Cashtag(e) => (e.offset, e.length, EntityType::Cashtag, None),
            tl::enums::MessageEntity::BankCard(e) => {
                (e.offset, e.length, EntityType::BankCard, None)
            }
            tl::enums::MessageEntity::BotCommand(e) => {
                (e.offset, e.length, EntityType::BotCommand, None)
            }
            tl::enums::MessageEntity::CustomEmoji(e) => (
                e.offset,
                e.length,
                EntityType::CustomEmoji,
                Some(e.document_id.to_string()),
            ),
            _ => return None,
        };

        Some(TextEntity {
            offset,
            length,
            entity_type,
            data,
        })
    }
}

fn select_thumb_meta(thumbs: &[PhotoSize]) -> (Option<String>, Option<Vec<u8>>) {
    let candidates: Vec<&PhotoSize> = thumbs
        .iter()
        .filter(|t| !matches!(t, PhotoSize::Empty(_)))
        .collect();

    if candidates.is_empty() {
        return (None, None);
    }

    let normal: Vec<&PhotoSize> = candidates
        .iter()
        .copied()
        .filter(|t| {
            matches!(
                t,
                PhotoSize::Size(_) | PhotoSize::Progressive(_) | PhotoSize::Cached(_)
            )
        })
        .collect();

    let chosen: &PhotoSize = if normal.is_empty() {
        candidates.iter().copied().min_by_key(|t| t.size()).unwrap()
    } else {
        normal.iter().copied().min_by_key(|t| t.size()).unwrap()
    };

    let thumb_type = Some(chosen.photo_type());
    let thumb_bytes = Downloadable::to_data(chosen);
    (thumb_type, thumb_bytes)
}

pub struct TdlibCatchUpFetcher {
    client: Arc<TdlibClient>,
    limit: usize,
}

impl TdlibCatchUpFetcher {
    pub fn new(client: Arc<TdlibClient>, limit: usize) -> Self {
        Self { client, limit }
    }
}

impl CatchUpFetcher for TdlibCatchUpFetcher {
    fn fetch(&self, source: &str, last_id: i64) -> CatchUpFuture {
        let client = self.client.clone();
        let source = source.to_string();
        let limit = self.limit;

        Box::pin(async move {
            let peer = match client.resolve_chat(&source).await {
                Ok(chat) => chat,
                Err(e) => {
                    tracing::warn!("无法解析来源: {} error: {}", source, e);
                    return Err(e);
                }
            };

            match client.get_messages((&peer).into(), limit, last_id).await {
                Ok(messages) => Ok(messages),
                Err(e) => {
                    tracing::warn!("获取补漏消息失败: {} error: {}", source, e);
                    Err(e)
                }
            }
        })
    }
}

fn dialog_id_to_peer(dialog_id: i64) -> PeerRef {
    use grammers_session::defs::{PeerAuth, PeerId};

    if dialog_id <= -1000000000000 {
        let channel_id = -dialog_id - 1000000000000;
        PeerRef {
            id: PeerId::channel(channel_id),
            auth: PeerAuth::default(),
        }
    } else if dialog_id < 0 {
        PeerRef {
            id: PeerId::chat(-dialog_id),
            auth: PeerAuth::default(),
        }
    } else {
        PeerRef {
            id: PeerId::user(dialog_id),
            auth: PeerAuth::default(),
        }
    }
}

fn clamp_history_page_limit(limit: usize) -> usize {
    if limit == 0 {
        1
    } else if limit > HISTORY_PAGE_LIMIT {
        HISTORY_PAGE_LIMIT
    } else {
        limit
    }
}
