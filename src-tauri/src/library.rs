use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use crate::models::{
    FaceAnalysisResult, FaceAnalysisStatus, FaceCandidate, LibraryOverview, LibraryRoot,
    LibraryRootStatus, MediaItem, MediaMetadata, MediaType, ScanStats,
};

#[derive(Default)]
pub struct LibraryState {
    roots: Mutex<Vec<LibraryRoot>>,
    media_items: Mutex<Vec<MediaItem>>,
    metadata: Mutex<Vec<MediaMetadata>>,
    faces: Mutex<Vec<FaceCandidate>>,
}

impl LibraryState {
    pub fn add_root(&self, path: String) -> Result<LibraryRoot, String> {
        let normalized_path = normalize_path(&path)?;
        let mut roots = self
            .roots
            .lock()
            .map_err(|_| "library state is unavailable")?;

        if let Some(existing) = roots.iter().find(|root| root.path == normalized_path) {
            return Ok(existing.clone());
        }

        let id = format!("root-{}", roots.len() + 1);
        let root = LibraryRoot {
            id,
            name: display_name(&normalized_path),
            path: normalized_path,
            status: LibraryRootStatus::Ready,
            photo_count: 0,
            video_count: 0,
            media_count: 0,
        };

        roots.push(root.clone());
        Ok(root)
    }

    pub fn roots(&self) -> Result<Vec<LibraryRoot>, String> {
        self.roots
            .lock()
            .map(|roots| roots.clone())
            .map_err(|_| "library state is unavailable".to_string())
    }

    pub fn overview(&self) -> Result<LibraryOverview, String> {
        let roots = self
            .roots
            .lock()
            .map_err(|_| "library state is unavailable")?;
        Ok(LibraryOverview {
            root_count: roots.len(),
            photo_count: roots.iter().map(|root| root.photo_count).sum(),
            video_count: roots.iter().map(|root| root.video_count).sum(),
            media_count: roots.iter().map(|root| root.media_count).sum(),
        })
    }

    pub fn root_path(&self, root_id: &str) -> Result<PathBuf, String> {
        let mut roots = self
            .roots
            .lock()
            .map_err(|_| "library state is unavailable")?;
        let root = roots
            .iter_mut()
            .find(|root| root.id == root_id)
            .ok_or_else(|| format!("library root '{root_id}' was not found"))?;

        root.status = LibraryRootStatus::Scanning;
        Ok(PathBuf::from(&root.path))
    }

    pub fn finish_scan(&self, result: ScanResult) -> Result<ScanStats, String> {
        let stats = result.stats;

        let mut roots = self
            .roots
            .lock()
            .map_err(|_| "library state is unavailable")?;
        let root = roots
            .iter_mut()
            .find(|root| root.id == stats.root_id)
            .ok_or_else(|| format!("library root '{}' was not found", stats.root_id))?;

        root.status = LibraryRootStatus::Ready;
        root.photo_count = stats.photo_count;
        root.video_count = stats.video_count;
        root.media_count = stats.media_count;
        drop(roots);

        let mut media_items = self
            .media_items
            .lock()
            .map_err(|_| "library media state is unavailable")?;
        media_items.retain(|item| item.root_id != stats.root_id);
        media_items.extend(result.items);

        Ok(stats)
    }

    pub fn media(
        &self,
        root_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<MediaItem>, String> {
        let media_items = self
            .media_items
            .lock()
            .map_err(|_| "library media state is unavailable")?;

        Ok(media_items
            .iter()
            .filter(|item| item.root_id == root_id)
            .skip(offset)
            .take(limit)
            .cloned()
            .collect())
    }

    pub fn photo_count_for_root(&self, root_id: &str) -> Result<u64, String> {
        let media_items = self
            .media_items
            .lock()
            .map_err(|_| "library media state is unavailable")?;

        Ok(media_items
            .iter()
            .filter(|item| item.root_id == root_id && matches!(item.media_type, MediaType::Photo))
            .count() as u64)
    }

    pub fn media_path(&self, media_id: &str) -> Result<PathBuf, String> {
        let media_items = self
            .media_items
            .lock()
            .map_err(|_| "library media state is unavailable")?;

        media_items
            .iter()
            .find(|item| item.id == media_id)
            .map(|item| PathBuf::from(&item.path))
            .ok_or_else(|| format!("media item '{media_id}' was not found"))
    }

    pub fn metadata_for_media(&self, media_ids: Vec<String>) -> Result<Vec<MediaMetadata>, String> {
        let metadata = self
            .metadata
            .lock()
            .map_err(|_| "library metadata state is unavailable")?;

        Ok(media_ids
            .into_iter()
            .map(|media_id| {
                metadata
                    .iter()
                    .find(|entry| entry.media_id == media_id)
                    .cloned()
                    .unwrap_or(MediaMetadata {
                        media_id,
                        ..MediaMetadata::default()
                    })
            })
            .collect())
    }

    pub fn set_favorite(&self, media_id: String, favorite: bool) -> Result<MediaMetadata, String> {
        self.ensure_media_exists(&media_id)?;
        let mut metadata = self
            .metadata
            .lock()
            .map_err(|_| "library metadata state is unavailable")?;
        let entry = metadata_entry(&mut metadata, media_id);
        entry.favorite = favorite;
        Ok(entry.clone())
    }

    pub fn set_tags(&self, media_id: String, tags: Vec<String>) -> Result<MediaMetadata, String> {
        self.ensure_media_exists(&media_id)?;
        let mut normalized_tags = tags
            .into_iter()
            .map(|tag| tag.trim().to_string())
            .filter(|tag| !tag.is_empty())
            .collect::<Vec<_>>();
        normalized_tags.sort_by_key(|tag| tag.to_ascii_lowercase());
        normalized_tags.dedup_by(|first, second| first.eq_ignore_ascii_case(second));

        let mut metadata = self
            .metadata
            .lock()
            .map_err(|_| "library metadata state is unavailable")?;
        let entry = metadata_entry(&mut metadata, media_id);
        entry.tags = normalized_tags;
        Ok(entry.clone())
    }

    pub fn analyze_faces(&self, media_id: String) -> Result<FaceAnalysisResult, String> {
        self.ensure_media_exists(&media_id)?;
        let faces = self
            .faces
            .lock()
            .map_err(|_| "face metadata state is unavailable")?
            .iter()
            .filter(|face| face.media_id == media_id)
            .cloned()
            .collect::<Vec<_>>();

        Ok(FaceAnalysisResult {
            media_id,
            status: FaceAnalysisStatus::ModelMissing,
            message:
                "Face analysis is ready for a local model, but no model bundle is installed yet."
                    .to_string(),
            faces,
        })
    }

    pub fn set_face_name(&self, face_id: String, name: String) -> Result<FaceCandidate, String> {
        let mut faces = self
            .faces
            .lock()
            .map_err(|_| "face metadata state is unavailable")?;
        let face = faces
            .iter_mut()
            .find(|face| face.id == face_id)
            .ok_or_else(|| format!("face '{face_id}' was not found"))?;

        let trimmed = name.trim();
        face.name = (!trimmed.is_empty()).then(|| trimmed.to_string());
        Ok(face.clone())
    }

    pub fn clear_media(&self, root_id: &str) -> Result<(), String> {
        let mut media_items = self
            .media_items
            .lock()
            .map_err(|_| "library media state is unavailable")?;
        media_items.retain(|item| item.root_id != root_id);
        Ok(())
    }

    pub fn fail_scan(&self, root_id: &str) -> Result<(), String> {
        let mut roots = self
            .roots
            .lock()
            .map_err(|_| "library state is unavailable")?;
        if let Some(root) = roots.iter_mut().find(|root| root.id == root_id) {
            root.status = LibraryRootStatus::Error;
        }
        Ok(())
    }

    fn ensure_media_exists(&self, media_id: &str) -> Result<(), String> {
        let media_items = self
            .media_items
            .lock()
            .map_err(|_| "library media state is unavailable")?;
        media_items
            .iter()
            .any(|item| item.id == media_id)
            .then_some(())
            .ok_or_else(|| format!("media item '{media_id}' was not found"))
    }
}

fn metadata_entry(metadata: &mut Vec<MediaMetadata>, media_id: String) -> &mut MediaMetadata {
    if let Some(index) = metadata.iter().position(|entry| entry.media_id == media_id) {
        return &mut metadata[index];
    }

    metadata.push(MediaMetadata {
        media_id,
        ..MediaMetadata::default()
    });
    metadata.last_mut().expect("metadata was just inserted")
}

pub struct ScanResult {
    stats: ScanStats,
    items: Vec<MediaItem>,
}

pub fn scan_root(root_id: String, root_path: PathBuf) -> Result<ScanResult, String> {
    let mut pending = VecDeque::from([root_path]);
    let mut photo_count = 0;
    let mut video_count = 0;
    let mut skipped_count = 0;
    let mut items = Vec::new();

    while let Some(path) = pending.pop_front() {
        let Ok(entries) = fs::read_dir(&path) else {
            skipped_count += 1;
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_dir() {
                pending.push_back(path);
            } else if let Some(media_type) = media_type_for_path(&path) {
                match media_type {
                    MediaType::Photo => photo_count += 1,
                    MediaType::Video => video_count += 1,
                }

                items.push(MediaItem {
                    id: String::new(),
                    root_id: root_id.clone(),
                    name: display_name(&path.to_string_lossy()),
                    path: path.to_string_lossy().to_string(),
                    media_type,
                });
            } else {
                skipped_count += 1;
            }
        }
    }

    items.sort_by(|first, second| first.path.cmp(&second.path));

    for (index, item) in items.iter_mut().enumerate() {
        item.id = format!("{}-media-{}", root_id, index + 1);
    }

    Ok(ScanResult {
        stats: ScanStats {
            root_id,
            photo_count,
            video_count,
            media_count: photo_count + video_count,
            skipped_count,
        },
        items,
    })
}

fn normalize_path(path: &str) -> Result<String, String> {
    let path = PathBuf::from(path);
    let metadata =
        fs::metadata(&path).map_err(|error| format!("folder is unavailable: {error}"))?;

    if !metadata.is_dir() {
        return Err("selected path is not a folder".to_string());
    }

    path.canonicalize()
        .map_err(|error| format!("failed to resolve folder path: {error}"))
        .map(|path| path.to_string_lossy().to_string())
}

fn display_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn is_supported_image(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return false;
    };

    matches!(
        extension.to_ascii_lowercase().as_str(),
        "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "heic" | "heif" | "avif"
    )
}

fn media_type_for_path(path: &Path) -> Option<MediaType> {
    if is_supported_image(path) {
        Some(MediaType::Photo)
    } else if is_supported_video(path) {
        Some(MediaType::Video)
    } else {
        None
    }
}

fn is_supported_video(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return false;
    };

    matches!(
        extension.to_ascii_lowercase().as_str(),
        "mp4"
            | "mov"
            | "m4v"
            | "avi"
            | "mkv"
            | "webm"
            | "wmv"
            | "mpg"
            | "mpeg"
            | "3gp"
            | "mts"
            | "m2ts"
    )
}
