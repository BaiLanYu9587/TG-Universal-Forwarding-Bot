use super::model::MediaView;
use std::future::Future;
use std::pin::Pin;

pub type ThumbHash = (u64, u64);

pub trait ThumbHasher: Send + Sync {
    fn fill_thumb_hashes<'a>(
        &'a self,
        media_list: &'a mut [MediaView],
    ) -> Pin<Box<dyn Future<Output = Vec<ThumbHash>> + Send + 'a>>;
}

pub struct NoopThumbHasher;

impl ThumbHasher for NoopThumbHasher {
    fn fill_thumb_hashes<'a>(
        &'a self,
        _media_list: &'a mut [MediaView],
    ) -> Pin<Box<dyn Future<Output = Vec<ThumbHash>> + Send + 'a>> {
        Box::pin(async { Vec::new() })
    }
}
