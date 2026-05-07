use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryRoot {
    pub id: String,
    pub name: String,
    pub path: String,
    pub status: LibraryRootStatus,
    pub photo_count: u64,
    pub video_count: u64,
    pub media_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaItem {
    pub id: String,
    pub root_id: String,
    pub name: String,
    pub path: String,
    pub media_type: MediaType,
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMetadata {
    pub media_id: String,
    pub favorite: bool,
    pub tags: Vec<String>,
    pub face_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceCandidate {
    pub id: String,
    pub media_id: String,
    pub name: Option<String>,
    pub confidence: f32,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSpace {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceCatalog {
    pub current_space_id: String,
    pub spaces: Vec<AppSpace>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceAnalysisResult {
    pub media_id: String,
    pub status: FaceAnalysisStatus,
    pub message: String,
    pub faces: Vec<FaceCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FaceAnalysisStatus {
    Ready,
    ModelMissing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaType {
    Photo,
    Video,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LibraryRootStatus {
    Ready,
    Scanning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanStats {
    pub root_id: String,
    pub photo_count: u64,
    pub video_count: u64,
    pub media_count: u64,
    pub skipped_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryOverview {
    pub root_count: usize,
    pub photo_count: u64,
    pub video_count: u64,
    pub media_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiModelInfo {
    pub id: String,
    pub name: String,
    pub task: String,
    pub accelerator: String,
    pub description: String,
    pub file_name: String,
    pub download_url: String,
    pub installed: bool,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInstallResult {
    pub model: AiModelInfo,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDeleteResult {
    pub model: AiModelInfo,
    pub removed_bytes: u64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderAnalysisResult {
    pub root_id: String,
    pub task: String,
    pub model_id: String,
    pub processed_media: u64,
    pub status: FaceAnalysisStatus,
    pub message: String,
    pub faces: Vec<FaceCandidate>,
    pub metadata: Vec<MediaMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheClearResult {
    pub removed_files: u64,
    pub removed_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseStats {
    pub path: String,
    pub size_bytes: u64,
    pub root_count: u64,
    pub media_count: u64,
    pub metadata_count: u64,
    pub favorite_count: u64,
    pub tag_count: u64,
    pub face_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderOperationResult {
    pub root_id: String,
    pub affected_media: u64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaDeleteResult {
    pub deleted_media: u64,
    pub failed_paths: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGroup {
    pub hash: String,
    pub items: Vec<MediaItem>,
}
