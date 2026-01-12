use super::model::{DedupCommitInfo, MediaType, MediaView};
use super::thumb::ThumbHash;
use common::text::truncate_text;
use regex::Regex;
use siphasher::sip::SipHasher;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;

static CLEAN_TEXT_RE: OnceLock<Regex> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct DedupState {
    pub fingerprints: VecDeque<u64>,
    pub media_id_signatures: VecDeque<String>,
    pub media_meta_signatures: VecDeque<String>,
    pub media_thumb_hashes: VecDeque<ThumbHash>,
    pub recent_text_features: VecDeque<Vec<u64>>,
}

#[derive(Debug, Clone)]
pub struct DedupConfig {
    pub limit: usize,
    pub media_limit: usize,
    pub hamming_threshold: usize,
    pub jaccard_short_threshold: f64,
    pub jaccard_long_threshold: f64,
    pub recent_text_limit: usize,
    pub thumb_phash_threshold: usize,
    pub thumb_dhash_threshold: usize,
    pub album_thumb_ratio: f64,
    pub short_text_threshold: usize,
}

pub struct MessageDeduplicator {
    fingerprints: VecDeque<u64>,
    recent_text_features: VecDeque<Vec<u64>>,
    media_id_signatures: VecDeque<String>,
    media_id_signature_set: HashSet<String>,
    media_meta_signatures: VecDeque<String>,
    media_meta_signature_set: HashSet<String>,
    media_thumb_hashes: VecDeque<ThumbHash>,
    pending_fingerprints: HashMap<u64, usize>,
    pending_text_features: HashMap<Vec<u64>, usize>,
    pending_media_id_sigs: HashMap<String, usize>,
    pending_media_meta_sigs: HashMap<String, usize>,
    pending_thumb_hashes: HashMap<ThumbHash, usize>,
    pending_candidates: Vec<DedupCommitInfo>,
    hamming_threshold: usize,
    jaccard_short_threshold: f64,
    jaccard_long_threshold: f64,
    thumb_phash_threshold: usize,
    thumb_dhash_threshold: usize,
    album_thumb_ratio: f64,
    short_text_threshold: usize,
    recent_text_limit: usize,
    limit: usize,
    media_limit: usize,
}

#[derive(Debug, Clone)]
pub struct DedupCandidate {
    pub fp: u64,
    pub text_features: Vec<u64>,
    pub media_id_sigs: Vec<String>,
    pub media_meta_sigs: Vec<String>,
}

pub(crate) enum DedupDecision {
    Duplicate,
    NeedThumb(DedupCandidate),
    Unique(DedupCandidate),
}

impl MessageDeduplicator {
    pub fn new(config: DedupConfig) -> Self {
        Self::from_snapshot(config, None)
    }

    pub fn from_snapshot(config: DedupConfig, snapshot: Option<DedupState>) -> Self {
        let DedupConfig {
            limit,
            media_limit,
            hamming_threshold,
            jaccard_short_threshold,
            jaccard_long_threshold,
            recent_text_limit,
            thumb_phash_threshold,
            thumb_dhash_threshold,
            album_thumb_ratio,
            short_text_threshold,
        } = config;

        let mut this = Self {
            fingerprints: VecDeque::with_capacity(limit),
            recent_text_features: VecDeque::with_capacity(recent_text_limit),
            media_id_signatures: VecDeque::with_capacity(media_limit),
            media_id_signature_set: HashSet::new(),
            media_meta_signatures: VecDeque::with_capacity(media_limit),
            media_meta_signature_set: HashSet::new(),
            media_thumb_hashes: VecDeque::with_capacity(media_limit),
            pending_fingerprints: HashMap::new(),
            pending_text_features: HashMap::new(),
            pending_media_id_sigs: HashMap::new(),
            pending_media_meta_sigs: HashMap::new(),
            pending_thumb_hashes: HashMap::new(),
            pending_candidates: Vec::new(),
            hamming_threshold,
            jaccard_short_threshold,
            jaccard_long_threshold,
            thumb_phash_threshold,
            thumb_dhash_threshold,
            album_thumb_ratio,
            short_text_threshold,
            recent_text_limit,
            limit,
            media_limit,
        };

        if let Some(snap) = snapshot {
            this.restore_snapshot(snap);
        }

        this
    }

    pub fn snapshot(&self) -> DedupState {
        DedupState {
            fingerprints: self.fingerprints.clone(),
            media_id_signatures: self.media_id_signatures.clone(),
            media_meta_signatures: self.media_meta_signatures.clone(),
            media_thumb_hashes: self.media_thumb_hashes.clone(),
            recent_text_features: self.recent_text_features.clone(),
        }
    }

    fn restore_snapshot(&mut self, snap: DedupState) {
        self.fingerprints = snap
            .fingerprints
            .into_iter()
            .rev()
            .take(self.limit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        for sig in snap
            .media_id_signatures
            .into_iter()
            .rev()
            .take(self.media_limit)
        {
            if self.media_id_signature_set.contains(&sig) {
                continue;
            }
            self.media_id_signatures.push_front(sig.clone());
            self.media_id_signature_set.insert(sig);
        }

        for sig in snap
            .media_meta_signatures
            .into_iter()
            .rev()
            .take(self.media_limit)
        {
            if self.media_meta_signature_set.contains(&sig) {
                continue;
            }
            self.media_meta_signatures.push_front(sig.clone());
            self.media_meta_signature_set.insert(sig);
        }

        self.media_thumb_hashes = snap
            .media_thumb_hashes
            .into_iter()
            .rev()
            .take(self.media_limit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        self.recent_text_features = snap
            .recent_text_features
            .into_iter()
            .rev()
            .take(self.recent_text_limit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
    }

    fn has_media_id_signature(&self, sig: &str) -> bool {
        self.media_id_signature_set.contains(sig) || self.pending_media_id_sigs.contains_key(sig)
    }

    fn has_media_meta_signature(&self, sig: &str) -> bool {
        self.media_meta_signature_set.contains(sig)
            || self.pending_media_meta_sigs.contains_key(sig)
    }

    pub(crate) fn check_text_and_meta(
        &self,
        text: &str,
        media_list: &[MediaView],
        log_link: Option<&str>,
    ) -> DedupDecision {
        let media_id_sigs = self.extract_media_id_signatures(media_list);
        let media_meta_sigs = self.extract_media_meta_signatures(media_list);

        // 判断是否是有媒体+短文本场景（需要双重验证）
        // 包括：单图+短文本、多图相册+短文本
        let text_char_count = text.chars().count();
        let is_media_short_text =
            !media_list.is_empty() && text_char_count <= self.short_text_threshold;

        // 用于记录文本匹配情况
        let mut text_matched = false;
        let mut text_match_reason = String::new();

        // SimHash 检查
        let fp = self.simhash(text);
        if fp != 0 {
            for history_fp in self
                .fingerprints
                .iter()
                .chain(self.pending_fingerprints.keys())
            {
                let dist = self.hamming_distance(fp, *history_fp);
                if dist <= self.hamming_threshold {
                    if is_media_short_text {
                        // 有媒体+短文本：只记录匹配，稍后进行双重验证
                        text_matched = true;
                        text_match_reason = format!("SimHash(dist={})", dist);
                        tracing::debug!(
                            "有媒体短文本文本匹配(SimHash): dist={} media_count={} chars={} text_preview={}",
                            dist,
                            media_list.len(),
                            text_char_count,
                            truncate_text(text, 20)
                        );
                        break;
                    } else {
                        // 其他场景：文本匹配即判定重复
                        if let Some(link) = log_link {
                            tracing::info!(
                                "SimHash命中重复: dist={} link={} text_preview={}",
                                dist,
                                link,
                                truncate_text(text, 20)
                            );
                        } else {
                            tracing::info!(
                                "SimHash命中重复: dist={} text_preview={}",
                                dist,
                                truncate_text(text, 20)
                            );
                        }
                        return DedupDecision::Duplicate;
                    }
                }
            }
        }

        // Jaccard 相似度检查
        let cleaned_text = self.clean_text(text);
        let text_features = if cleaned_text.is_empty() {
            Vec::new()
        } else {
            self.extract_feature_hashes_from_cleaned(&cleaned_text)
        };
        if !text_matched && !text_features.is_empty() {
            let threshold = if cleaned_text.len() < 50 {
                self.jaccard_short_threshold
            } else {
                self.jaccard_long_threshold
            };

            for recent_feats in self
                .recent_text_features
                .iter()
                .chain(self.pending_text_features.keys())
            {
                let score = self.jaccard_similarity_hash(&text_features, recent_feats);
                if score > threshold {
                    if is_media_short_text {
                        // 有媒体+短文本：只记录匹配，稍后进行双重验证
                        text_matched = true;
                        text_match_reason = format!("Jaccard(score={:.2})", score);
                        tracing::debug!(
                            "有媒体短文本文本匹配(Jaccard): score={:.2} media_count={} chars={} text_preview={}",
                            score,
                            media_list.len(),
                            text_char_count,
                            truncate_text(text, 20)
                        );
                        break;
                    } else {
                        // 其他场景：文本匹配即判定重复
                        if let Some(link) = log_link {
                            tracing::info!(
                                "Jaccard相似度命中重复: score={:.2} link={} text_preview={}",
                                score,
                                link,
                                truncate_text(text, 20)
                            );
                        } else {
                            tracing::info!(
                                "Jaccard相似度命中重复: score={:.2} text_preview={}",
                                score,
                                truncate_text(text, 20)
                            );
                        }
                        return DedupDecision::Duplicate;
                    }
                }
            }
        }

        // 有媒体+短文本的双重验证逻辑（包括单图、多图相册）
        if is_media_short_text && text_matched {
            // 文本已匹配，检查媒体是否也匹配
            let media_matched = media_id_sigs
                .iter()
                .any(|sig| self.has_media_id_signature(sig))
                || media_meta_sigs
                    .iter()
                    .any(|sig| self.has_media_meta_signature(sig));

            if media_matched {
                // 文本和媒体都匹配 → 判定重复
                if let Some(link) = log_link {
                    tracing::info!(
                        "有媒体短文本双重验证命中重复: {} + 媒体匹配 media_count={} link={} text_preview={}",
                        text_match_reason,
                        media_list.len(),
                        link,
                        truncate_text(text, 20)
                    );
                } else {
                    tracing::info!(
                        "有媒体短文本双重验证命中重复: {} + 媒体匹配 media_count={} text_preview={}",
                        text_match_reason,
                        media_list.len(),
                        truncate_text(text, 20)
                    );
                }
                return DedupDecision::Duplicate;
            } else {
                // 文本匹配但媒体不匹配 → 继续走缩略图判断流程
                tracing::debug!(
                    "有媒体短文本仅文本匹配，媒体签名不同，等待缩略图验证: {} media_count={} text_preview={}",
                    text_match_reason,
                    media_list.len(),
                    truncate_text(text, 20)
                );
                // 继续执行后续逻辑，不返回
            }
        }

        // 空文本+有媒体的特殊处理：直接进行媒体签名去重
        // 这是为了解决源频道发送相同图片但不同文本（或无文本）的情况
        if is_media_short_text && !text_matched && text_char_count == 0 {
            let total_media = media_id_sigs.len().max(media_meta_sigs.len());
            if total_media > 0 {
                let mut matched_count = 0;

                for idx in 0..total_media {
                    let id_match = media_id_sigs
                        .get(idx)
                        .is_some_and(|sig| self.has_media_id_signature(sig));
                    let meta_match = media_meta_sigs
                        .get(idx)
                        .is_some_and(|sig| self.has_media_meta_signature(sig));
                    if id_match || meta_match {
                        matched_count += 1;
                    }
                }

                // 计算匹配比例
                let match_ratio = matched_count as f64 / total_media as f64;

                // 判断是否重复：空文本消息只要媒体签名匹配就算重复
                let is_duplicate = if total_media == 1 {
                    matched_count > 0
                } else {
                    match_ratio > self.album_thumb_ratio
                };

                if is_duplicate {
                    if let Some(link) = log_link {
                        tracing::info!(
                            "空文本媒体签名命中重复: 匹配={}/{} 比例={:.2} 阈值={:.2} link={}",
                            matched_count,
                            total_media,
                            match_ratio,
                            self.album_thumb_ratio,
                            link
                        );
                    } else {
                        tracing::info!(
                            "空文本媒体签名命中重复: 匹配={}/{} 比例={:.2} 阈值={:.2}",
                            matched_count,
                            total_media,
                            match_ratio,
                            self.album_thumb_ratio
                        );
                    }
                    return DedupDecision::Duplicate;
                }
            }
        }

        // 媒体去重逻辑：支持多图片比例判断
        // 注意：有媒体+短文本场景已在上面处理过，这里只处理常规场景
        // - 单张图片（非短文本场景）：任何匹配都算重复
        // - 多张图片（非短文本场景）：匹配比例超过阈值才算重复
        let total_media = media_id_sigs.len().max(media_meta_sigs.len());

        if total_media > 0 && !is_media_short_text {
            let mut matched_count = 0;

            for idx in 0..total_media {
                let id_match = media_id_sigs
                    .get(idx)
                    .is_some_and(|sig| self.has_media_id_signature(sig));
                let meta_match = media_meta_sigs
                    .get(idx)
                    .is_some_and(|sig| self.has_media_meta_signature(sig));
                if id_match || meta_match {
                    matched_count += 1;
                }
            }

            // 计算匹配比例
            let match_ratio = matched_count as f64 / total_media as f64;

            // 判断是否重复
            let is_duplicate = if total_media == 1 {
                // 单张图片：任何匹配都算重复
                matched_count > 0
            } else {
                // 多张图片：匹配比例超过阈值才算重复
                match_ratio > self.album_thumb_ratio
            };

            if is_duplicate {
                if let Some(link) = log_link {
                    tracing::info!(
                        "媒体签名命中重复: 匹配={}/{} 比例={:.2} 阈值={:.2} link={} text_preview={}",
                        matched_count,
                        total_media,
                        match_ratio,
                        self.album_thumb_ratio,
                        link,
                        truncate_text(text, 20)
                    );
                } else {
                    tracing::info!(
                        "媒体签名命中重复: 匹配={}/{} 比例={:.2} 阈值={:.2} text_preview={}",
                        matched_count,
                        total_media,
                        match_ratio,
                        self.album_thumb_ratio,
                        truncate_text(text, 20)
                    );
                }
                return DedupDecision::Duplicate;
            }
        }

        let candidate = DedupCandidate {
            fp,
            text_features,
            media_id_sigs,
            media_meta_sigs,
        };

        if media_list.is_empty() {
            DedupDecision::Unique(candidate)
        } else {
            DedupDecision::NeedThumb(candidate)
        }
    }

    fn pending_add_fingerprint(&mut self, fp: u64) {
        if fp == 0 {
            return;
        }
        let entry = self.pending_fingerprints.entry(fp).or_insert(0);
        *entry += 1;
    }

    fn pending_remove_fingerprint(&mut self, fp: u64) {
        if fp == 0 {
            return;
        }
        if let Some(entry) = self.pending_fingerprints.get_mut(&fp) {
            *entry = entry.saturating_sub(1);
            if *entry == 0 {
                self.pending_fingerprints.remove(&fp);
            }
        }
    }

    fn pending_add_text_features(&mut self, features: &[u64]) {
        if features.is_empty() {
            return;
        }
        let entry = self
            .pending_text_features
            .entry(features.to_vec())
            .or_insert(0);
        *entry += 1;
    }

    fn pending_remove_text_features(&mut self, features: &[u64]) {
        if features.is_empty() {
            return;
        }
        if let Some(entry) = self.pending_text_features.get_mut(features) {
            *entry = entry.saturating_sub(1);
            if *entry == 0 {
                self.pending_text_features.remove(features);
            }
        }
    }

    fn pending_add_media_id_sig(&mut self, sig: &str) {
        let entry = self
            .pending_media_id_sigs
            .entry(sig.to_string())
            .or_insert(0);
        *entry += 1;
    }

    fn pending_remove_media_id_sig(&mut self, sig: &str) {
        if let Some(entry) = self.pending_media_id_sigs.get_mut(sig) {
            *entry = entry.saturating_sub(1);
            if *entry == 0 {
                self.pending_media_id_sigs.remove(sig);
            }
        }
    }

    fn pending_add_media_meta_sig(&mut self, sig: &str) {
        let entry = self
            .pending_media_meta_sigs
            .entry(sig.to_string())
            .or_insert(0);
        *entry += 1;
    }

    fn pending_remove_media_meta_sig(&mut self, sig: &str) {
        if let Some(entry) = self.pending_media_meta_sigs.get_mut(sig) {
            *entry = entry.saturating_sub(1);
            if *entry == 0 {
                self.pending_media_meta_sigs.remove(sig);
            }
        }
    }

    fn pending_add_thumb_hash(&mut self, hash: ThumbHash) {
        let entry = self.pending_thumb_hashes.entry(hash).or_insert(0);
        *entry += 1;
    }

    fn pending_remove_thumb_hash(&mut self, hash: ThumbHash) {
        if let Some(entry) = self.pending_thumb_hashes.get_mut(&hash) {
            *entry = entry.saturating_sub(1);
            if *entry == 0 {
                self.pending_thumb_hashes.remove(&hash);
            }
        }
    }

    fn pending_add_candidate(&mut self, candidate: &DedupCandidate) {
        self.pending_add_fingerprint(candidate.fp);
        self.pending_add_text_features(&candidate.text_features);
        for sig in &candidate.media_id_sigs {
            self.pending_add_media_id_sig(sig);
        }
        for sig in &candidate.media_meta_sigs {
            self.pending_add_media_meta_sig(sig);
        }
    }

    fn pending_remove_candidate(&mut self, candidate: &DedupCandidate) {
        self.pending_remove_fingerprint(candidate.fp);
        self.pending_remove_text_features(&candidate.text_features);
        for sig in &candidate.media_id_sigs {
            self.pending_remove_media_id_sig(sig);
        }
        for sig in &candidate.media_meta_sigs {
            self.pending_remove_media_meta_sig(sig);
        }
    }

    fn pending_add_thumb_hashes(&mut self, hashes: &[ThumbHash]) {
        for hash in hashes {
            self.pending_add_thumb_hash(*hash);
        }
    }

    fn pending_remove_thumb_hashes(&mut self, hashes: &[ThumbHash]) {
        for hash in hashes {
            self.pending_remove_thumb_hash(*hash);
        }
    }

    fn remove_pending_candidate_info(&mut self, info: &DedupCommitInfo) {
        if let Some(pos) = self.pending_candidates.iter().position(|item| {
            item.candidate.fp == info.candidate.fp
                && item.candidate.text_features == info.candidate.text_features
                && item.candidate.media_id_sigs == info.candidate.media_id_sigs
                && item.candidate.media_meta_sigs == info.candidate.media_meta_sigs
                && item.thumb_hashes == info.thumb_hashes
        }) {
            self.pending_candidates.remove(pos);
        }
    }

    pub fn stage_candidate(
        &mut self,
        candidate: DedupCandidate,
        thumb_hashes: Vec<ThumbHash>,
    ) -> DedupCommitInfo {
        self.pending_add_candidate(&candidate);
        self.pending_add_thumb_hashes(&thumb_hashes);
        let info = DedupCommitInfo {
            candidate: candidate.clone(),
            thumb_hashes,
        };
        self.pending_candidates.push(info.clone());
        info
    }

    pub fn commit_pending(&mut self, info: DedupCommitInfo) {
        self.pending_remove_candidate(&info.candidate);
        self.pending_remove_thumb_hashes(&info.thumb_hashes);
        self.record_candidate(info.candidate.clone());
        for hash in &info.thumb_hashes {
            self.record_thumb_hash(*hash);
        }
        self.remove_pending_candidate_info(&info);
    }

    pub fn take_all_pending(&mut self) -> Vec<DedupCommitInfo> {
        let infos = std::mem::take(&mut self.pending_candidates);

        self.pending_fingerprints.clear();
        self.pending_text_features.clear();
        self.pending_media_id_sigs.clear();
        self.pending_media_meta_sigs.clear();
        self.pending_thumb_hashes.clear();

        infos
    }

    pub fn commit_all_pending(&mut self, infos: &[DedupCommitInfo]) {
        for info in infos {
            self.commit_pending(info.clone());
        }
    }

    pub fn rollback_pending(&mut self, info: DedupCommitInfo) {
        self.pending_remove_candidate(&info.candidate);
        self.pending_remove_thumb_hashes(&info.thumb_hashes);
        self.remove_pending_candidate_info(&info);
    }

    pub fn record_candidate(&mut self, candidate: DedupCandidate) {
        if candidate.fp != 0 && !self.fingerprints.contains(&candidate.fp) {
            self.fingerprints.push_back(candidate.fp);
            if self.fingerprints.len() > self.limit {
                self.fingerprints.pop_front();
            }
        }

        if !candidate.text_features.is_empty()
            && !self.recent_text_features.contains(&candidate.text_features)
        {
            self.recent_text_features.push_back(candidate.text_features);
            if self.recent_text_features.len() > self.recent_text_limit {
                self.recent_text_features.pop_front();
            }
        }

        for sig in candidate.media_id_sigs {
            self.record_media_id(sig);
        }
        for sig in candidate.media_meta_sigs {
            self.record_media_meta(sig);
        }
    }

    pub(crate) fn check_thumb(
        &self,
        thumb_hashes: &[ThumbHash],
        text_preview: &str,
        log_link: Option<&str>,
    ) -> bool {
        if thumb_hashes.is_empty() {
            return false;
        }

        let total_thumbs = thumb_hashes.len();
        let mut matched_count = 0;

        for (phash, dhash) in thumb_hashes {
            if self.thumb_match(*phash, *dhash) {
                matched_count += 1;
            }
        }

        if matched_count == 0 {
            return false;
        }

        // 计算匹配比例
        let match_ratio = matched_count as f64 / total_thumbs as f64;

        // 判断是否重复
        let is_duplicate = if total_thumbs == 1 {
            // 单张图片：任何匹配都算重复
            true
        } else {
            // 多张图片：匹配比例超过阈值才算重复
            match_ratio > self.album_thumb_ratio
        };

        if is_duplicate {
            if let Some(link) = log_link {
                tracing::info!(
                    "缩略图哈希命中重复: 匹配={}/{} 比例={:.2} 阈值={:.2} link={} text_preview={}",
                    matched_count,
                    total_thumbs,
                    match_ratio,
                    self.album_thumb_ratio,
                    link,
                    truncate_text(text_preview, 20)
                );
            } else {
                tracing::info!(
                    "缩略图哈希命中重复: 匹配={}/{} 比例={:.2} 阈值={:.2} text_preview={}",
                    matched_count,
                    total_thumbs,
                    match_ratio,
                    self.album_thumb_ratio,
                    truncate_text(text_preview, 20)
                );
            }
        }

        is_duplicate
    }

    fn extract_media_id_signatures(&self, media_list: &[MediaView]) -> Vec<String> {
        media_list
            .iter()
            .map(|media| match media.media_type {
                MediaType::Photo => {
                    let id = media.media_id.unwrap_or(0);
                    format!(
                        "photo_id_{}_hash_{}_size_{}",
                        id, media.access_hash, media.size_bytes
                    )
                }
                MediaType::Video => {
                    let id = media.media_id.unwrap_or(0);
                    format!(
                        "video_id_{}_hash_{}_size_{}_dur_{:?}",
                        id, media.access_hash, media.size_bytes, media.duration
                    )
                }
                MediaType::Document => {
                    let id = media.media_id.unwrap_or(0);
                    format!(
                        "doc_id_{}_hash_{}_size_{}_mime_{:?}",
                        id, media.access_hash, media.size_bytes, media.mime_type
                    )
                }
            })
            .collect()
    }

    fn extract_media_meta_signatures(&self, media_list: &[MediaView]) -> Vec<String> {
        media_list
            .iter()
            .map(|media| match media.media_type {
                MediaType::Photo => {
                    format!(
                        "photo_meta_size_{}_wh_{:?}x{:?}",
                        media.size_bytes, media.width, media.height
                    )
                }
                MediaType::Video => {
                    format!(
                        "video_meta_size_{}_duration_{:?}",
                        media.size_bytes, media.duration
                    )
                }
                MediaType::Document => {
                    format!(
                        "doc_meta_size_{}_mime_{:?}",
                        media.size_bytes, media.mime_type
                    )
                }
            })
            .collect()
    }

    fn simhash(&self, text: &str) -> u64 {
        let all_features = self.extract_features(text);

        if all_features.is_empty() {
            return 0;
        }

        let mut v = [0i32; 64];
        for feature in &all_features {
            let mut hasher = SipHasher::new();
            feature.hash(&mut hasher);
            let h = hasher.finish();
            for (i, value) in v.iter_mut().enumerate() {
                if (h >> i) & 1 == 1 {
                    *value += 1;
                } else {
                    *value -= 1;
                }
            }
        }

        let mut fingerprint = 0u64;
        for (i, value) in v.iter().enumerate() {
            if *value >= 0 {
                fingerprint |= 1 << i;
            }
        }

        fingerprint
    }

    fn extract_features(&self, text: &str) -> Vec<String> {
        let cleaned = self.clean_text(text);
        if cleaned.is_empty() {
            return Vec::new();
        }

        let chars: Vec<char> = cleaned.chars().collect();
        let width = 4;

        if chars.len() < width {
            return vec![cleaned];
        }

        (0..=chars.len() - width)
            .map(|i| chars[i..i + width].iter().collect())
            .collect()
    }

    fn extract_feature_hashes_from_cleaned(&self, cleaned: &str) -> Vec<u64> {
        let chars: Vec<char> = cleaned.chars().collect();
        let width = 4;

        if chars.len() < width {
            let mut hasher = SipHasher::new();
            cleaned.hash(&mut hasher);
            return vec![hasher.finish()];
        }

        (0..=chars.len() - width)
            .map(|i| {
                let mut hasher = SipHasher::new();
                let feature: String = chars[i..i + width].iter().collect();
                feature.hash(&mut hasher);
                hasher.finish()
            })
            .collect()
    }

    fn clean_text(&self, text: &str) -> String {
        // 激进清洗：移除所有空格、标点、Emoji，只保留字母、数字、下划线和汉字
        // 这样可以最大程度忽略排版和噪音差异，提升 SimHash 和 Jaccard 的去重效果
        let re = CLEAN_TEXT_RE.get_or_init(|| {
            Regex::new(r"[^\w\u4e00-\u9fa5]+").expect("clean_text regex must be valid")
        });
        re.replace_all(text, "").to_lowercase()
    }

    fn hamming_distance(&self, a: u64, b: u64) -> usize {
        (a ^ b).count_ones() as usize
    }

    fn jaccard_similarity_hash(&self, feats1: &[u64], feats2: &[u64]) -> f64 {
        if feats1.is_empty() || feats2.is_empty() {
            return 0.0;
        }

        let set1: HashSet<u64> = feats1.iter().cloned().collect();
        let set2: HashSet<u64> = feats2.iter().cloned().collect();

        if set1.is_empty() || set2.is_empty() {
            return 0.0;
        }

        let intersection = set1.intersection(&set2).count();
        let union = set1.union(&set2).count();

        if union == 0 {
            return 0.0;
        }

        intersection as f64 / union as f64
    }

    fn record_media_id(&mut self, sig: String) {
        if self.media_id_signature_set.contains(&sig) {
            return;
        }
        if self.media_id_signatures.len() >= self.media_limit {
            if let Some(removed) = self.media_id_signatures.pop_front() {
                self.media_id_signature_set.remove(&removed);
            }
        }
        self.media_id_signatures.push_back(sig.clone());
        self.media_id_signature_set.insert(sig);
        if self.media_id_signatures.len() > self.media_limit {
            self.media_id_signatures.pop_front();
        }
    }

    fn record_media_meta(&mut self, sig: String) {
        if self.media_meta_signature_set.contains(&sig) {
            return;
        }
        if self.media_meta_signatures.len() >= self.media_limit {
            if let Some(removed) = self.media_meta_signatures.pop_front() {
                self.media_meta_signature_set.remove(&removed);
            }
        }
        self.media_meta_signatures.push_back(sig.clone());
        self.media_meta_signature_set.insert(sig);
        if self.media_meta_signatures.len() > self.media_limit {
            self.media_meta_signatures.pop_front();
        }
    }

    fn thumb_match(&self, phash: u64, dhash: u64) -> bool {
        for (stored_phash, stored_dhash) in &self.media_thumb_hashes {
            if self.hamming_distance(phash, *stored_phash) <= self.thumb_phash_threshold {
                return true;
            }
            if self.hamming_distance(dhash, *stored_dhash) <= self.thumb_dhash_threshold {
                return true;
            }
        }
        for (stored_phash, stored_dhash) in self.pending_thumb_hashes.keys() {
            if self.hamming_distance(phash, *stored_phash) <= self.thumb_phash_threshold {
                return true;
            }
            if self.hamming_distance(dhash, *stored_dhash) <= self.thumb_dhash_threshold {
                return true;
            }
        }
        false
    }

    pub fn record_thumb_hash(&mut self, hash: ThumbHash) {
        if self.media_thumb_hashes.contains(&hash) {
            return;
        }
        if self.media_thumb_hashes.len() >= self.media_limit {
            self.media_thumb_hashes.pop_front();
        }
        self.media_thumb_hashes.push_back(hash);
    }
}
