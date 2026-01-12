use anyhow::Result;
use common::text::truncate_text;
use grammers_client::{grammers_tl_types as tl, types::InputMessage, Client, InvocationError};
use grammers_session::defs::PeerRef;
use tracing::{debug, error, info, warn};

use super::model::{EntityType, ForwardTask, MediaType, MediaView, TextEntity};
use super::TdlibClient;
use std::future::Future;
use std::sync::OnceLock;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use tg_core::throttle::SendThrottle;
use tokio::sync::Mutex;

pub struct MessageSender;

impl MessageSender {
    pub async fn send_text(
        client: &Client,
        peer: &PeerRef,
        text: &str,
        entities: &[TextEntity],
        policy: FloodWaitPolicy,
    ) -> Result<i64> {
        info!(
            "发送文本: chat_id={} 长度={} 内容预览=\"{}\"",
            peer.id.bot_api_dialog_id(),
            text.chars().count(),
            truncate_text(text, 120)
        );
        let mut msg = InputMessage::new().text(text);

        if !entities.is_empty() {
            let parsed_entities = entities
                .iter()
                .filter_map(convert_entity)
                .collect::<Vec<_>>();
            msg = msg.fmt_entities(parsed_entities);
        }

        let result: grammers_client::types::Message = handle_flood_wait(
            || async { client.send_message(*peer, msg.clone()).await },
            "发送文本",
            policy,
        )
        .await?;
        Ok(result.id() as i64)
    }

    pub async fn send_media(
        client: &Client,
        peer: &PeerRef,
        media: &MediaView,
        caption: &str,
        entities: &[TextEntity],
        policy: FloodWaitPolicy,
    ) -> Result<i64> {
        info!(
            "发送媒体: chat_id={} type={:?} size_bytes={} caption预览=\"{}\"",
            peer.id.bot_api_dialog_id(),
            media.media_type,
            media.size_bytes,
            truncate_text(caption, 120)
        );
        let mut msg = InputMessage::new().text(caption);

        if !entities.is_empty() {
            let parsed_entities = entities
                .iter()
                .filter_map(convert_entity)
                .collect::<Vec<_>>();
            msg = msg.fmt_entities(parsed_entities);
        }

        let input_media = convert_media(media)?;
        msg = msg.media(input_media);

        let result: grammers_client::types::Message = handle_flood_wait(
            || async { client.send_message(*peer, msg.clone()).await },
            "发送媒体",
            policy,
        )
        .await?;
        Ok(result.id() as i64)
    }

    pub async fn send_album(
        client: &Client,
        peer: &PeerRef,
        medias: &[MediaView],
        caption: &str,
        entities: &[TextEntity],
        policy: FloodWaitPolicy,
    ) -> Result<()> {
        if medias.is_empty() {
            return Ok(());
        }

        info!(
            "发送相册: chat_id={} 张数={} caption预览=\"{}\"",
            peer.id.bot_api_dialog_id(),
            medias.len(),
            truncate_text(caption, 120)
        );

        let input_peer: tl::enums::InputPeer = (*peer).into();

        let mut multi_media = Vec::new();
        for (idx, media) in medias.iter().enumerate() {
            let input_media = convert_media(media)?;

            let message = if idx == 0 {
                caption.to_string()
            } else {
                String::new()
            };
            let parsed_entities = if idx == 0 && !entities.is_empty() {
                Some(entities.iter().filter_map(convert_entity).collect())
            } else {
                None
            };

            let random_id = next_random_id();

            multi_media.push(tl::enums::InputSingleMedia::Media(
                tl::types::InputSingleMedia {
                    media: input_media,
                    random_id,
                    message,
                    entities: parsed_entities,
                },
            ));
        }

        handle_flood_wait(
            || async {
                client
                    .invoke(&tl::functions::messages::SendMultiMedia {
                        silent: false,
                        background: false,
                        clear_draft: false,
                        noforwards: false,
                        update_stickersets_order: false,
                        invert_media: false,
                        allow_paid_floodskip: false,
                        peer: input_peer.clone(),
                        reply_to: None,
                        multi_media: multi_media.clone(),
                        schedule_date: None,
                        send_as: None,
                        quick_reply_shortcut: None,
                        effect: None,
                        allow_paid_stars: None,
                    })
                    .await
            },
            "发送相册",
            policy,
        )
        .await?;

        Ok(())
    }
}

static RANDOM_ID_SEQ: OnceLock<AtomicU64> = OnceLock::new();

fn next_random_id() -> i64 {
    let seq = RANDOM_ID_SEQ.get_or_init(|| {
        AtomicU64::new(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64,
        )
    });
    seq.fetch_add(1, Ordering::Relaxed) as i64
}

fn convert_media(media: &MediaView) -> Result<tl::enums::InputMedia> {
    let id = media
        .media_id
        .ok_or_else(|| anyhow::anyhow!("媒体 ID 缺失"))?;

    match media.media_type {
        MediaType::Photo => Ok(tl::types::InputMediaPhoto {
            spoiler: media.spoiler,
            id: tl::types::InputPhoto {
                id,
                access_hash: media.access_hash,
                file_reference: media.file_reference.clone(),
            }
            .into(),
            ttl_seconds: None,
        }
        .into()),
        MediaType::Video | MediaType::Document => Ok(tl::types::InputMediaDocument {
            spoiler: media.spoiler,
            id: tl::types::InputDocument {
                id,
                access_hash: media.access_hash,
                file_reference: media.file_reference.clone(),
            }
            .into(),
            ttl_seconds: None,
            query: None,
            video_cover: None,
            video_timestamp: None,
        }
        .into()),
    }
}

fn convert_entity(entity: &TextEntity) -> Option<tl::enums::MessageEntity> {
    match entity.entity_type {
        EntityType::Bold => Some(
            tl::types::MessageEntityBold {
                offset: entity.offset,
                length: entity.length,
            }
            .into(),
        ),
        EntityType::Italic => Some(
            tl::types::MessageEntityItalic {
                offset: entity.offset,
                length: entity.length,
            }
            .into(),
        ),
        EntityType::Underline => Some(
            tl::types::MessageEntityUnderline {
                offset: entity.offset,
                length: entity.length,
            }
            .into(),
        ),
        EntityType::Strikethrough => Some(
            tl::types::MessageEntityStrike {
                offset: entity.offset,
                length: entity.length,
            }
            .into(),
        ),
        EntityType::Code => Some(
            tl::types::MessageEntityCode {
                offset: entity.offset,
                length: entity.length,
            }
            .into(),
        ),
        EntityType::Pre => Some(
            tl::types::MessageEntityPre {
                offset: entity.offset,
                length: entity.length,
                language: entity.data.clone().unwrap_or_default(),
            }
            .into(),
        ),
        EntityType::TextUrl => {
            let url = entity.data.as_ref()?;
            Some(
                tl::types::MessageEntityTextUrl {
                    offset: entity.offset,
                    length: entity.length,
                    url: url.clone(),
                }
                .into(),
            )
        }
        EntityType::Mention => Some(
            tl::types::MessageEntityMention {
                offset: entity.offset,
                length: entity.length,
            }
            .into(),
        ),
        EntityType::Hashtag => Some(
            tl::types::MessageEntityHashtag {
                offset: entity.offset,
                length: entity.length,
            }
            .into(),
        ),
        EntityType::Spoiler => Some(
            tl::types::MessageEntitySpoiler {
                offset: entity.offset,
                length: entity.length,
            }
            .into(),
        ),
        EntityType::Blockquote => Some(
            tl::types::MessageEntityBlockquote {
                offset: entity.offset,
                length: entity.length,
                collapsed: entity.data.as_deref() == Some("true"),
            }
            .into(),
        ),
        EntityType::Url => Some(
            tl::types::MessageEntityUrl {
                offset: entity.offset,
                length: entity.length,
            }
            .into(),
        ),
        EntityType::Email => Some(
            tl::types::MessageEntityEmail {
                offset: entity.offset,
                length: entity.length,
            }
            .into(),
        ),
        EntityType::Phone => Some(
            tl::types::MessageEntityPhone {
                offset: entity.offset,
                length: entity.length,
            }
            .into(),
        ),
        EntityType::Cashtag => Some(
            tl::types::MessageEntityCashtag {
                offset: entity.offset,
                length: entity.length,
            }
            .into(),
        ),
        EntityType::BankCard => Some(
            tl::types::MessageEntityBankCard {
                offset: entity.offset,
                length: entity.length,
            }
            .into(),
        ),
        EntityType::BotCommand => Some(
            tl::types::MessageEntityBotCommand {
                offset: entity.offset,
                length: entity.length,
            }
            .into(),
        ),
        EntityType::CustomEmoji => {
            let document_id = entity.data.as_ref()?.parse::<i64>().ok()?;
            Some(
                tl::types::MessageEntityCustomEmoji {
                    offset: entity.offset,
                    length: entity.length,
                    document_id,
                }
                .into(),
            )
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FloodWaitPolicy {
    pub max_retries: usize,
    pub max_total_wait: std::time::Duration,
    pub request_timeout: std::time::Duration,
}

async fn handle_flood_wait<F, Fut, T>(f: F, action_desc: &str, policy: FloodWaitPolicy) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, InvocationError>>,
{
    let mut retry_count = 0;
    let mut total_wait_time = std::time::Duration::from_secs(0);

    loop {
        let result = tokio::time::timeout(policy.request_timeout, f()).await;
        match result {
            Ok(Ok(result)) => return Ok(result),
            Ok(Err(InvocationError::Rpc(e))) if e.name == "FLOOD_WAIT" => {
                retry_count += 1;

                if retry_count > policy.max_retries {
                    error!(
                        "{} FloodWait 重试次数超限: {} 次",
                        action_desc, policy.max_retries
                    );
                    return Err(InvocationError::Rpc(e).into());
                }

                let wait = match e.value {
                    Some(w) => std::time::Duration::from_secs(w as u64),
                    None => {
                        error!("{} FloodWait 缺少等待时间", action_desc);
                        return Err(InvocationError::Rpc(e).into());
                    }
                };

                if total_wait_time + wait > policy.max_total_wait {
                    error!(
                        "{} FloodWait 累计等待时间超限: {} 秒",
                        action_desc,
                        policy.max_total_wait.as_secs()
                    );
                    return Err(InvocationError::Rpc(e).into());
                }

                total_wait_time += wait;

                warn!(
                    "{} 遇到 FloodWait (重试 {}/{}), 等待 {} 秒 (累计 {} 秒)",
                    action_desc,
                    retry_count,
                    policy.max_retries,
                    wait.as_secs(),
                    total_wait_time.as_secs()
                );

                tokio::time::sleep(wait).await;
                info!("{} FloodWait 重试", action_desc);
            }
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => {
                error!(
                    "{} 请求超时: timeout={}s",
                    action_desc,
                    policy.request_timeout.as_secs()
                );
                return Err(anyhow::anyhow!("{} 请求超时", action_desc));
            }
        }
    }
}

pub struct MessageHandler {
    client: Arc<TdlibClient>,
    target_peer: PeerRef,
    send_throttle: Arc<Mutex<SendThrottle>>,
    flood_wait_policy: FloodWaitPolicy,
}

impl MessageHandler {
    pub fn new(
        client: Arc<TdlibClient>,
        target_peer: PeerRef,
        send_throttle: Arc<Mutex<SendThrottle>>,
        flood_wait_policy: FloodWaitPolicy,
    ) -> Self {
        Self {
            client,
            target_peer,
            send_throttle,
            flood_wait_policy,
        }
    }

    async fn send_with_throttle<F, Fut, T>(&self, action: F) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let mut throttle = self.send_throttle.lock().await;
        let wait_time = throttle.wait().await;
        if !wait_time.is_zero() {
            debug!("转发延迟生效: {:.2} 秒", wait_time.as_secs_f64());
        }

        let result = action().await;
        if result.is_ok() {
            throttle.mark_sent();
        }
        result
    }

    pub async fn process_task(&self, task: ForwardTask) -> Result<()> {
        let mut messages = Vec::new();
        for msg in &task.messages {
            if msg.album_id.is_some() && !task.is_album {
                continue;
            }
            messages.push(msg.clone());
        }

        if messages.is_empty() {
            info!("任务无有效消息，跳过");
            return Ok(());
        }

        let client = self.client.client();
        let target_peer = self.target_peer;

        if task.is_album {
            info!("发送相册: {} 张", messages.len());
            // pipeline 已经把所有文本和 entities 合并到第一条消息，直接使用
            let caption = &messages[0].text;
            let entities = &messages[0].entities;
            let policy = self.flood_wait_policy;

            let mut medias = Vec::new();
            for msg in &messages {
                if let Some(ref media) = msg.media {
                    medias.push(media.clone());
                }
            }

            if medias.is_empty() {
                info!("相册无媒体，仅发送文本");
                self.send_with_throttle(|| async {
                    MessageSender::send_text(client, &target_peer, caption, entities, policy).await
                })
                .await?;
            } else {
                self.send_with_throttle(|| async {
                    MessageSender::send_album(
                        client,
                        &target_peer,
                        &medias,
                        caption,
                        entities,
                        policy,
                    )
                    .await
                })
                .await?;
            }
        } else {
            info!("发送单条消息");
            let message = &messages[0];
            let caption = &message.text;
            let entities = &message.entities;
            let policy = self.flood_wait_policy;

            if let Some(ref media) = message.media {
                self.send_with_throttle(|| async {
                    MessageSender::send_media(
                        client,
                        &target_peer,
                        media,
                        caption,
                        entities,
                        policy,
                    )
                    .await
                })
                .await?;
            } else if !caption.is_empty() {
                self.send_with_throttle(|| async {
                    MessageSender::send_text(client, &target_peer, caption, entities, policy).await
                })
                .await?;
            } else {
                info!("消息无文本和媒体，跳过");
            }
        }

        Ok(())
    }
}
