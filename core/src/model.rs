use super::dedup::DedupCandidate;
use super::thumb::ThumbHash;

#[derive(Debug, Clone)]
pub struct DedupCommitInfo {
    pub candidate: DedupCandidate,
    pub thumb_hashes: Vec<ThumbHash>,
}

#[derive(Debug, Clone)]
pub struct ForwardTask {
    pub source_key: String,
    pub messages: Vec<MessageView>,
    pub is_album: bool,
    pub dedup_infos: Vec<DedupCommitInfo>,
}

#[derive(Debug, Clone)]
pub struct MessageView {
    pub id: i64,
    pub chat_id: i64,
    pub text: String,
    pub entities: Vec<TextEntity>,
    pub media: Option<MediaView>,
    pub album_id: Option<i64>,
    pub is_edit: bool,
}

#[derive(Debug, Clone)]
pub struct MediaView {
    pub media_type: MediaType,
    pub size_bytes: usize,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub duration: Option<i32>,
    pub media_id: Option<i64>,
    pub access_hash: i64,
    pub file_reference: Vec<u8>,
    pub thumb_type: Option<String>,
    pub thumb_bytes: Option<Vec<u8>>,
    pub spoiler: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaType {
    Photo,
    Video,
    Document,
}

#[derive(Debug, Clone)]
pub struct TextEntity {
    pub offset: i32,
    pub length: i32,
    pub entity_type: EntityType,
    pub data: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityType {
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Code,
    Pre,
    TextUrl,
    Mention,
    Hashtag,
    Spoiler,
    Blockquote,
    Url,
    Email,
    Phone,
    Cashtag,
    BankCard,
    BotCommand,
    CustomEmoji,
}

#[derive(Debug, Clone)]
pub struct PipelineResult {
    pub should_forward: bool,
    pub text: String,
    pub entities: Vec<TextEntity>,
    pub media_list: Vec<MediaView>,
    pub dedup_info: Option<DedupCommitInfo>,
}
