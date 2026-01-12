use super::model::{MediaType, MediaView};

pub struct MediaFilter {
    size_limit: usize,
    ext_allowlist: Vec<String>,
    allow_documents: bool,
}

impl MediaFilter {
    pub fn new(size_limit: usize, ext_allowlist: Vec<String>, allow_documents: bool) -> Self {
        Self {
            size_limit,
            ext_allowlist,
            allow_documents,
        }
    }

    pub fn should_skip(&self, media: &MediaView) -> bool {
        if media.size_bytes > self.size_limit && self.size_limit > 0 {
            tracing::info!("媒体大小超过限制: {} 字节", media.size_bytes);
            return true;
        }

        match media.media_type {
            MediaType::Photo => return false,
            MediaType::Video => return false,
            MediaType::Document => {
                if !self.allow_documents {
                    tracing::info!("文档转发已禁用");
                    return true;
                }

                if !self.ext_allowlist.is_empty() {
                    if let Some(ref filename) = media.file_name {
                        let ext = filename
                            .rsplit('.')
                            .next()
                            .map(|s| s.to_lowercase())
                            .unwrap_or_default();
                        if !self.ext_allowlist.contains(&ext) {
                            tracing::info!("文档后缀不在白名单: {}", ext);
                            return true;
                        }
                    } else {
                        tracing::info!("文档无文件名且已配置白名单");
                        return true;
                    }
                }
            }
        }

        false
    }
}
