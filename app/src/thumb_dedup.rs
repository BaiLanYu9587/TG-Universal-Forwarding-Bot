use anyhow::{Context, Result};
use grammers_client::grammers_tl_types as tl;
use grammers_client::types::Downloadable;
use image::imageops::FilterType;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tdlib::TdlibClient;
use tg_core::model::{MediaType, MediaView};
use tg_core::thumb::{ThumbHash, ThumbHasher};
use tracing::{debug, warn};

// RAII guard 用于确保临时文件被清理
struct FileGuard {
    path: PathBuf,
    should_remove: bool,
}

impl FileGuard {
    fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            should_remove: path.exists(),
        }
    }

    fn mark_created(&mut self) {
        self.should_remove = true;
    }
}

impl Drop for FileGuard {
    fn drop(&mut self) {
        if self.should_remove {
            let path = self.path.clone();
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    let _ = tokio::fs::remove_file(&path).await;
                });
            }
        }
    }
}

pub struct ThumbDedupManager {
    client: TdlibClient,
    thumb_dir: PathBuf,
    seq: AtomicU64,
    request_timeout: Duration,
}

impl ThumbDedupManager {
    pub fn new(client: TdlibClient, thumb_dir: PathBuf, request_timeout: Duration) -> Result<Self> {
        if !thumb_dir.exists() {
            std::fs::create_dir_all(&thumb_dir)
                .with_context(|| format!("无法创建缩略图目录: {:?}", thumb_dir))?;
        }

        if let Err(e) = cleanup_orphan_thumbs(&thumb_dir, Duration::from_secs(600)) {
            warn!("清理缩略图缓存失败: {:?}", e);
        }

        Ok(Self {
            client,
            thumb_dir,
            seq: AtomicU64::new(0),
            request_timeout,
        })
    }

    fn should_hash(media: &MediaView) -> bool {
        match media.media_type {
            MediaType::Photo => true,
            MediaType::Video => true,
            MediaType::Document => {
                if let Some(mime) = &media.mime_type {
                    mime.starts_with("image/") || mime.starts_with("video/")
                } else {
                    false
                }
            }
        }
    }

    fn build_temp_path(&self, media_id: Option<i64>) -> PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let name = match media_id {
            Some(id) => format!("thumb_{}_{}_{}", id, ts, seq),
            None => format!("thumb_{}_{}", ts, seq),
        };
        self.thumb_dir.join(name)
    }

    async fn fetch_thumb_bytes(&self, media: &MediaView, path: &Path) -> Result<Option<Vec<u8>>> {
        if let Some(bytes) = &media.thumb_bytes {
            tokio::fs::write(path, bytes)
                .await
                .with_context(|| format!("无法写入缩略图文件: {:?}", path))?;
            return Ok(Some(bytes.clone()));
        }

        let thumb_type = match &media.thumb_type {
            Some(t) => t,
            None => return Ok(None),
        };

        let id = match media.media_id {
            Some(id) => id,
            None => return Ok(None),
        };

        let location = match media.media_type {
            MediaType::Photo => {
                tl::enums::InputFileLocation::from(tl::types::InputPhotoFileLocation {
                    id,
                    access_hash: media.access_hash,
                    file_reference: media.file_reference.clone(),
                    thumb_size: thumb_type.clone(),
                })
            }
            MediaType::Video | MediaType::Document => {
                tl::enums::InputFileLocation::from(tl::types::InputDocumentFileLocation {
                    id,
                    access_hash: media.access_hash,
                    file_reference: media.file_reference.clone(),
                    thumb_size: thumb_type.clone(),
                })
            }
        };

        let download = ThumbDownload { location };
        match tokio::time::timeout(
            self.request_timeout,
            self.client.client().download_media(&download, path),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                warn!("下载缩略图失败: media_id={:?} err={}", media.media_id, e);
                let _ = tokio::fs::remove_file(path).await;
                return Ok(None);
            }
            Err(_) => {
                warn!(
                    "下载缩略图超时: media_id={:?} timeout={}s",
                    media.media_id,
                    self.request_timeout.as_secs()
                );
                let _ = tokio::fs::remove_file(path).await;
                return Ok(None);
            }
        }

        let bytes = match tokio::fs::read(path).await {
            Ok(bytes) => bytes,
            Err(e) => {
                let _ = tokio::fs::remove_file(path).await;
                return Err(anyhow::anyhow!("读取缩略图失败: {:?} err={}", path, e));
            }
        };

        Ok(Some(bytes))
    }

    async fn hash_media(&self, media: &MediaView) -> Result<Option<ThumbHash>> {
        if !Self::should_hash(media) {
            return Ok(None);
        }

        let path = self.build_temp_path(media.media_id);

        // 使用 RAII guard 确保文件被清理
        let mut file_guard = FileGuard::new(&path);

        let bytes = match self.fetch_thumb_bytes(media, &path).await? {
            Some(bytes) => {
                file_guard.mark_created();
                bytes
            }
            None => {
                return Ok(None);
            }
        };
        // fetch_thumb_bytes 成功后，文件已存在于 path
        // FileGuard 会在作用域结束时自动删除文件

        let hash = tokio::task::spawn_blocking(move || compute_hashes(&bytes))
            .await
            .context("缩略图哈希任务失败")??;
        Ok(Some(hash))
    }
}

impl ThumbHasher for ThumbDedupManager {
    fn fill_thumb_hashes<'a>(
        &'a self,
        media_list: &'a mut [MediaView],
    ) -> Pin<Box<dyn Future<Output = Vec<ThumbHash>> + Send + 'a>> {
        Box::pin(async move {
            let mut hashes = Vec::new();
            for media in media_list {
                match self.hash_media(media).await {
                    Ok(Some(hash)) => hashes.push(hash),
                    Ok(None) => {}
                    Err(e) => {
                        debug!("缩略图哈希失败: media_id={:?} err={}", media.media_id, e);
                    }
                }
            }
            hashes
        })
    }
}

struct ThumbDownload {
    location: tl::enums::InputFileLocation,
}

impl Downloadable for ThumbDownload {
    fn to_raw_input_location(&self) -> Option<tl::enums::InputFileLocation> {
        Some(self.location.clone())
    }
}

pub(crate) fn cleanup_orphan_thumbs(thumb_dir: &Path, max_age: Duration) -> Result<usize> {
    let mut removed = 0usize;
    let now = SystemTime::now();
    let entries = std::fs::read_dir(thumb_dir)
        .with_context(|| format!("无法读取缩略图目录: {:?}", thumb_dir))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = entry.file_name();
        let name = match file_name.to_str() {
            Some(name) => name,
            None => continue,
        };
        if !name.starts_with("thumb_") {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(e) => {
                warn!("读取缩略图元数据失败: path={:?} err={}", path, e);
                continue;
            }
        };
        let modified = match metadata.modified() {
            Ok(modified) => modified,
            Err(e) => {
                warn!("读取缩略图修改时间失败: path={:?} err={}", path, e);
                continue;
            }
        };
        let age = match now.duration_since(modified) {
            Ok(age) => age,
            Err(_) => Duration::from_secs(0),
        };
        if age < max_age {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => removed += 1,
            Err(e) => {
                warn!("删除缩略图文件失败: path={:?} err={}", path, e);
            }
        }
    }
    if removed > 0 {
        debug!("已清理遗留缩略图文件: count={}", removed);
    }
    Ok(removed)
}

fn compute_hashes(bytes: &[u8]) -> Result<ThumbHash> {
    let image = image::load_from_memory(bytes).context("无法解析缩略图")?;
    let phash = compute_phash(&image);
    let dhash = compute_dhash(&image);
    Ok((phash, dhash))
}

fn compute_dhash(image: &image::DynamicImage) -> u64 {
    let resized = image.resize_exact(9, 8, FilterType::Triangle).to_luma8();
    let mut hash = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            let left = resized.get_pixel(x, y)[0];
            let right = resized.get_pixel(x + 1, y)[0];
            if left > right {
                hash |= 1u64 << (y * 8 + x);
            }
        }
    }
    hash
}

fn compute_phash(image: &image::DynamicImage) -> u64 {
    let resized = image.resize_exact(32, 32, FilterType::Triangle).to_luma8();
    let mut pixels = [0f32; 32 * 32];
    for y in 0..32 {
        for x in 0..32 {
            pixels[y * 32 + x] = resized.get_pixel(x as u32, y as u32)[0] as f32;
        }
    }

    let mut cos_table = [[0f32; 32]; 8];
    for (u, row) in cos_table.iter_mut().enumerate() {
        for (x, value) in row.iter_mut().enumerate() {
            let angle = (2 * x + 1) as f32 * u as f32 * std::f32::consts::PI / 64.0;
            *value = angle.cos();
        }
    }

    let mut dct = [0f32; 64];
    for u in 0..8 {
        for v in 0..8 {
            let mut sum = 0f32;
            for x in 0..32 {
                for y in 0..32 {
                    let idx = y * 32 + x;
                    sum += pixels[idx] * cos_table[u][x] * cos_table[v][y];
                }
            }
            let cu = if u == 0 { 0.70710677 } else { 1.0 };
            let cv = if v == 0 { 0.70710677 } else { 1.0 };
            dct[u * 8 + v] = 0.25 * cu * cv * sum;
        }
    }

    let mut values = dct[1..].to_vec();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = values[values.len() / 2];

    let mut hash = 0u64;
    for (i, value) in dct.iter().enumerate() {
        if *value > median {
            hash |= 1u64 << i;
        }
    }
    hash
}
