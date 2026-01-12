use super::model::{DedupCommitInfo, MediaView, MessageView, PipelineResult, TextEntity};
use super::{
    append::ContentAppender,
    dedup::{DedupDecision, DedupState, MessageDeduplicator},
    filter::KeywordFilter,
    media_filter::MediaFilter,
    replace::TextReplacer,
    thumb::ThumbHasher,
};
use std::sync::Arc;

pub struct Pipeline {
    pub filter: KeywordFilter,
    pub replacer: TextReplacer,
    pub deduplicator: MessageDeduplicator,
    pub appender: ContentAppender,
    pub media_filter: MediaFilter,
    pub thumb_hasher: Option<Arc<dyn ThumbHasher>>,
    pub append_limit_with_media: usize,
    pub append_limit_text: usize,
}

pub struct PipelineConfig {
    pub filter: KeywordFilter,
    pub replacer: TextReplacer,
    pub deduplicator: MessageDeduplicator,
    pub appender: ContentAppender,
    pub media_filter: MediaFilter,
    pub thumb_hasher: Option<Arc<dyn ThumbHasher>>,
    pub append_limit_with_media: usize,
    pub append_limit_text: usize,
}

impl Pipeline {
    pub fn new(config: PipelineConfig) -> Self {
        let PipelineConfig {
            filter,
            replacer,
            deduplicator,
            appender,
            media_filter,
            thumb_hasher,
            append_limit_with_media,
            append_limit_text,
        } = config;
        Self {
            filter,
            replacer,
            deduplicator,
            appender,
            media_filter,
            thumb_hasher,
            append_limit_with_media,
            append_limit_text,
        }
    }

    pub fn commit_dedup(&mut self, info: DedupCommitInfo) {
        self.deduplicator.commit_pending(info);
    }

    pub fn rollback_dedup(&mut self, info: DedupCommitInfo) {
        self.deduplicator.rollback_pending(info);
    }

    pub async fn process(
        &mut self,
        message: &MessageView,
        apply_append: bool,
        log_link: Option<&str>,
    ) -> PipelineResult {
        let text = message.text.clone();
        let entities = message.entities.clone();

        let (text, entities) = self.replacer.process_with_entities(&text, entities);

        let (should_forward, reason) = self.filter.should_forward(&text);
        if !should_forward {
            tracing::info!("消息被过滤: reason={}", reason.unwrap_or_default());
            return PipelineResult {
                should_forward: false,
                text: String::new(),
                entities: Vec::new(),
                media_list: Vec::new(),
                dedup_info: None,
            };
        }

        let mut media_list = if let Some(ref media) = message.media {
            if self.should_skip_media(media) {
                tracing::info!("媒体被过滤: type={:?}", media.media_type);
                return PipelineResult {
                    should_forward: false,
                    text: String::new(),
                    entities: Vec::new(),
                    media_list: Vec::new(),
                    dedup_info: None,
                };
            }
            vec![media.clone()]
        } else {
            Vec::new()
        };

        let mut prechecked_thumb = false;
        let mut precomputed_thumb_hashes = None;

        if media_list.len() == 1 && !text.trim().is_empty() {
            if let Some(hasher) = &self.thumb_hasher {
                let thumb_hashes = hasher.fill_thumb_hashes(&mut media_list).await;
                if !thumb_hashes.is_empty() {
                    prechecked_thumb = true;
                    if self
                        .deduplicator
                        .check_thumb(&thumb_hashes, &text, log_link)
                    {
                        if let Some(link) = log_link {
                            tracing::info!("检测到缩略图重复: link={}", link);
                        } else {
                            tracing::info!("检测到缩略图重复");
                        }
                        return PipelineResult {
                            should_forward: false,
                            text: String::new(),
                            entities: Vec::new(),
                            media_list: Vec::new(),
                            dedup_info: None,
                        };
                    }
                    precomputed_thumb_hashes = Some(thumb_hashes);
                }
            }
        }

        let decision = self
            .deduplicator
            .check_text_and_meta(&text, &media_list, log_link);

        let dedup_info = match decision {
            DedupDecision::Duplicate => {
                if let Some(link) = log_link {
                    tracing::info!("检测到重复消息: link={}", link);
                } else {
                    tracing::info!("检测到重复消息");
                }
                return PipelineResult {
                    should_forward: false,
                    text: String::new(),
                    entities: Vec::new(),
                    media_list: Vec::new(),
                    dedup_info: None,
                };
            }
            DedupDecision::Unique(candidate) => {
                // 进入待提交队列，发送成功后再入库
                Some(self.deduplicator.stage_candidate(candidate, Vec::new()))
            }
            DedupDecision::NeedThumb(candidate) => {
                let thumb_hashes = if let Some(hashes) = precomputed_thumb_hashes.take() {
                    hashes
                } else if let Some(hasher) = &self.thumb_hasher {
                    hasher.fill_thumb_hashes(&mut media_list).await
                } else {
                    Vec::new()
                };

                if !prechecked_thumb {
                    let is_duplicate =
                        self.deduplicator
                            .check_thumb(&thumb_hashes, &text, log_link);
                    if is_duplicate {
                        if let Some(link) = log_link {
                            tracing::info!("检测到缩略图重复: link={}", link);
                        } else {
                            tracing::info!("检测到缩略图重复");
                        }
                        return PipelineResult {
                            should_forward: false,
                            text: String::new(),
                            entities: Vec::new(),
                            media_list: Vec::new(),
                            dedup_info: None,
                        };
                    }
                }
                // 进入待提交队列，发送成功后再入库
                Some(self.deduplicator.stage_candidate(candidate, thumb_hashes))
            }
        };

        let (text, entities) = if apply_append {
            self.append_content(&text, entities, !media_list.is_empty())
        } else {
            (text, entities)
        };

        // 去重信息已进入待提交队列，发送成功后再落库

        PipelineResult {
            should_forward: true,
            text,
            entities,
            media_list,
            dedup_info,
        }
    }

    pub async fn process_album(
        &mut self,
        messages: &[MessageView],
        log_link: Option<&str>,
    ) -> Option<(Vec<MessageView>, Option<DedupCommitInfo>)> {
        if messages.is_empty() {
            return None;
        }

        let mut processed_messages = Vec::with_capacity(messages.len());
        let mut combined_text = String::new();
        let mut combined_entities = Vec::new();
        let mut media_list = Vec::new();

        use common::utf16::utf16_len;

        for message in messages.iter() {
            let (text, entities) = self
                .replacer
                .process_with_entities(&message.text, message.entities.clone());

            if let Some(ref media) = message.media {
                if self.should_skip_media(media) {
                    tracing::info!("相册媒体被过滤: type={:?}", media.media_type);
                    return None;
                }
                media_list.push(media.clone());
            }

            let mut current_offset = utf16_len(&combined_text) as i32;

            if !text.is_empty() {
                if !combined_text.is_empty() {
                    combined_text.push_str("\n\n");
                    current_offset += 2; // for "\n\n"
                }
                combined_text.push_str(&text);

                for entity in entities {
                    let mut new_entity = entity.clone();
                    new_entity.offset += current_offset;
                    combined_entities.push(new_entity);
                }
            }

            let mut processed = message.clone();
            // 暂时清空，稍后将合并后的文本赋给第一条
            processed.text = String::new();
            processed.entities = Vec::new();
            processed_messages.push(processed);
        }

        let (should_forward, reason) = self.filter.should_forward(&combined_text);
        if !should_forward {
            tracing::info!("相册被过滤: reason={}", reason.unwrap_or_default());
            return None;
        }

        let decision = self
            .deduplicator
            .check_text_and_meta(&combined_text, &media_list, log_link);

        let dedup_info = match decision {
            DedupDecision::Duplicate => {
                if let Some(link) = log_link {
                    tracing::info!("检测到相册重复: link={}", link);
                } else {
                    tracing::info!("检测到相册重复");
                }
                return None;
            }
            DedupDecision::Unique(candidate) => {
                // 进入待提交队列，发送成功后再入库
                Some(self.deduplicator.stage_candidate(candidate, Vec::new()))
            }
            DedupDecision::NeedThumb(candidate) => {
                let thumb_hashes = if let Some(hasher) = &self.thumb_hasher {
                    hasher.fill_thumb_hashes(&mut media_list).await
                } else {
                    Vec::new()
                };
                let is_duplicate =
                    self.deduplicator
                        .check_thumb(&thumb_hashes, &combined_text, log_link);
                if is_duplicate {
                    if let Some(link) = log_link {
                        tracing::info!("检测到相册缩略图重复: link={}", link);
                    } else {
                        tracing::info!("检测到相册缩略图重复");
                    }
                    return None;
                }
                // 进入待提交队列，发送成功后再入库
                Some(self.deduplicator.stage_candidate(candidate, thumb_hashes))
            }
        };

        if let Some(first) = processed_messages.first_mut() {
            let (text, entities) =
                self.append_content(&combined_text, combined_entities, !media_list.is_empty());
            first.text = text;
            first.entities = entities;
        }

        // 去重信息已进入待提交队列，发送成功后再落库

        Some((processed_messages, dedup_info))
    }

    fn should_skip_media(&self, media: &MediaView) -> bool {
        self.media_filter.should_skip(media)
    }

    pub fn dedup_snapshot(&self) -> DedupState {
        self.deduplicator.snapshot()
    }

    pub fn take_all_pending(&mut self) -> Vec<DedupCommitInfo> {
        self.deduplicator.take_all_pending()
    }

    pub fn commit_all_pending(&mut self, infos: &[DedupCommitInfo]) {
        self.deduplicator.commit_all_pending(infos);
    }

    fn append_content(
        &self,
        text: &str,
        mut entities: Vec<TextEntity>,
        has_media: bool,
    ) -> (String, Vec<TextEntity>) {
        use common::utf16::*;

        let text_is_empty = text.trim().is_empty();
        if text_is_empty && self.appender.header.is_empty() && self.appender.footer.is_empty() {
            // 无正文且无追加配置
            return (String::new(), entities);
        }

        let limit = if has_media {
            self.append_limit_with_media
        } else {
            self.append_limit_text
        };

        let mut result_text = String::new();

        if !self.appender.header.is_empty() {
            result_text.push_str(&self.appender.header);
            result_text.push_str("\n\n");
            let header_offset = utf16_len(&self.appender.header) + 2;
            for ent in &mut entities {
                ent.offset += header_offset as i32;
            }
        }

        let header_utf16 = utf16_len(&result_text);
        let body_utf16 = utf16_len(text);
        let footer_utf16 = utf16_len(&self.appender.footer);
        let total_utf16 = header_utf16 + body_utf16 + footer_utf16;

        if total_utf16 > limit && !has_media {
            // 使用 saturating_sub 避免整数下溢
            // 为省略号 "..." 预留 3 个 UTF-16 单元
            let available = limit
                .saturating_sub(header_utf16)
                .saturating_sub(footer_utf16)
                .saturating_sub(3);

            if available > 0 {
                let available_chars = char_from_utf16_offset(text, available).unwrap_or(text.len());
                result_text.push_str(&text[..available_chars]);
                result_text.push_str("...");

                entities.retain(|ent| {
                    let ent_end = (ent.offset + ent.length) as usize;
                    ent_end <= available
                });
            } else {
                // 如果空间不足以容纳任何正文，只保留 header 和 footer
                // 不添加正文和省略号
                tracing::warn!(
                    "内容追加后超出限制且无空间容纳正文: limit={} header={} footer={}",
                    limit,
                    header_utf16,
                    footer_utf16
                );
                entities.clear();
            }
        } else {
            // When not truncating here (especially for media captions), defer trimming so
            // entity_trim can remove blockquote/code text.
            result_text.push_str(text);
        }

        if !self.appender.footer.is_empty() {
            result_text.push_str("\n\n");
            result_text.push_str(&self.appender.footer);
            // footer 不需要调整 entities 偏移，因为 entities 在 header 时已经偏移过
            // footer 添加在最后，不影响前面已有的 entities 位置
        }

        (result_text, entities)
    }
}
