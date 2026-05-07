use std::{
    collections::VecDeque,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
};

use sha2::{Digest, Sha256};

use crate::models::{
    DuplicateGroup, FaceAnalysisResult, FaceAnalysisStatus, FaceCandidate, LibraryOverview,
    LibraryRoot, LibraryRootStatus, MediaItem, MediaMetadata, MediaType, ScanStats,
};

#[derive(Default)]
pub struct LibraryState {
    roots: Mutex<Vec<LibraryRoot>>,
    media_items: Mutex<Vec<MediaItem>>,
    metadata: Mutex<Vec<MediaMetadata>>,
    faces: Mutex<Vec<FaceCandidate>>,
}

impl LibraryState {
    pub fn hydrate(
        &self,
        mut roots: Vec<LibraryRoot>,
        media_items: Vec<MediaItem>,
        metadata: Vec<MediaMetadata>,
        faces: Vec<FaceCandidate>,
    ) -> Result<(), String> {
        recount_root_counts(&mut roots, &media_items);
        *self
            .roots
            .lock()
            .map_err(|_| "library state is unavailable")? = roots;
        *self
            .media_items
            .lock()
            .map_err(|_| "library media state is unavailable")? = media_items;
        *self
            .metadata
            .lock()
            .map_err(|_| "library metadata state is unavailable")? = metadata;
        *self
            .faces
            .lock()
            .map_err(|_| "face metadata state is unavailable")? = faces;
        Ok(())
    }

    pub fn add_root(&self, path: String) -> Result<LibraryRoot, String> {
        let normalized_path = normalize_path(&path)?;
        let mut roots = self
            .roots
            .lock()
            .map_err(|_| "library state is unavailable")?;

        if let Some(existing) = roots.iter().find(|root| root.path == normalized_path) {
            return Ok(existing.clone());
        }

        let id = next_root_id(&roots);
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

    pub fn remove_root(&self, root_id: &str) -> Result<Vec<MediaItem>, String> {
        let mut roots = self
            .roots
            .lock()
            .map_err(|_| "library state is unavailable")?;
        let removed_root = roots
            .iter()
            .find(|root| root.id == root_id)
            .cloned()
            .ok_or_else(|| format!("library root '{root_id}' was not found"))?;
        roots.retain(|root| root.id != root_id);
        drop(roots);

        let mut media_items = self
            .media_items
            .lock()
            .map_err(|_| "library media state is unavailable")?;
        let removed_media = media_items
            .iter()
            .filter(|item| item.root_id == root_id)
            .cloned()
            .collect::<Vec<_>>();
        media_items.retain(|item| item.root_id != root_id);
        let removed_ids = removed_media
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>();
        drop(media_items);

        let mut metadata = self
            .metadata
            .lock()
            .map_err(|_| "library metadata state is unavailable")?;
        metadata.retain(|entry| {
            !removed_ids
                .iter()
                .any(|media_id| media_id == &entry.media_id)
        });
        drop(metadata);

        let mut faces = self
            .faces
            .lock()
            .map_err(|_| "face metadata state is unavailable")?;
        faces.retain(|face| {
            !removed_ids
                .iter()
                .any(|media_id| media_id == &face.media_id)
        });
        drop(faces);

        let _ = removed_root;
        Ok(removed_media)
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

    pub fn photo_media_for_root(&self, root_id: &str) -> Result<Vec<MediaItem>, String> {
        let media_items = self
            .media_items
            .lock()
            .map_err(|_| "library media state is unavailable")?;

        Ok(media_items
            .iter()
            .filter(|item| item.root_id == root_id && matches!(item.media_type, MediaType::Photo))
            .cloned()
            .collect())
    }

    pub fn media_for_root(&self, root_id: &str) -> Result<Vec<MediaItem>, String> {
        let media_items = self
            .media_items
            .lock()
            .map_err(|_| "library media state is unavailable")?;

        Ok(media_items
            .iter()
            .filter(|item| item.root_id == root_id)
            .cloned()
            .collect())
    }

    pub fn media_by_ids(&self, media_ids: &[String]) -> Result<Vec<MediaItem>, String> {
        let media_items = self
            .media_items
            .lock()
            .map_err(|_| "library media state is unavailable")?;

        Ok(media_items
            .iter()
            .filter(|item| media_ids.iter().any(|media_id| media_id == &item.id))
            .cloned()
            .collect())
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

    pub fn add_tags_for_media(
        &self,
        media_id: String,
        tags: Vec<String>,
    ) -> Result<MediaMetadata, String> {
        let current = self
            .metadata_for_media(vec![media_id.clone()])?
            .into_iter()
            .next()
            .unwrap_or(MediaMetadata {
                media_id: media_id.clone(),
                ..MediaMetadata::default()
            });
        let mut merged_tags = current.tags;
        merged_tags.extend(tags);
        self.set_tags(media_id, merged_tags)
    }

    pub fn replace_faces_for_media(
        &self,
        media_id: String,
        detected_faces: Vec<FaceCandidate>,
    ) -> Result<Vec<FaceCandidate>, String> {
        self.ensure_media_exists(&media_id)?;
        let mut faces = self
            .faces
            .lock()
            .map_err(|_| "face metadata state is unavailable")?;
        faces.retain(|face| face.media_id != media_id);
        faces.extend(detected_faces.clone());
        drop(faces);

        let mut metadata = self
            .metadata
            .lock()
            .map_err(|_| "library metadata state is unavailable")?;
        let entry = metadata_entry(&mut metadata, media_id);
        entry.face_ids = detected_faces.iter().map(|face| face.id.clone()).collect();

        Ok(detected_faces)
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

    pub fn delete_media_items(&self, media_ids: &[String]) -> Result<Vec<MediaItem>, String> {
        if media_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut media_items = self
            .media_items
            .lock()
            .map_err(|_| "library media state is unavailable")?;
        let removed_media = media_items
            .iter()
            .filter(|item| media_ids.iter().any(|media_id| media_id == &item.id))
            .cloned()
            .collect::<Vec<_>>();
        media_items.retain(|item| !media_ids.iter().any(|media_id| media_id == &item.id));

        let mut roots = self
            .roots
            .lock()
            .map_err(|_| "library state is unavailable")?;
        recount_root_counts(&mut roots, &media_items);
        drop(roots);
        drop(media_items);

        let mut metadata = self
            .metadata
            .lock()
            .map_err(|_| "library metadata state is unavailable")?;
        metadata.retain(|entry| !media_ids.iter().any(|media_id| media_id == &entry.media_id));
        drop(metadata);

        let mut faces = self
            .faces
            .lock()
            .map_err(|_| "face metadata state is unavailable")?;
        faces.retain(|face| !media_ids.iter().any(|media_id| media_id == &face.media_id));

        Ok(removed_media)
    }

    pub fn duplicate_groups(&self, root_id: Option<&str>) -> Result<Vec<DuplicateGroup>, String> {
        let media_items = self
            .media_items
            .lock()
            .map_err(|_| "library media state is unavailable")?;
        let mut grouped = std::collections::BTreeMap::<String, Vec<MediaItem>>::new();

        for item in media_items.iter() {
            if root_id.is_some_and(|candidate| candidate != item.root_id) {
                continue;
            }
            let Some(content_hash) = item.content_hash.clone() else {
                continue;
            };
            grouped.entry(content_hash).or_default().push(item.clone());
        }

        Ok(grouped
            .into_iter()
            .filter_map(|(hash, mut items)| {
                if items.len() < 2 {
                    return None;
                }
                items.sort_by(|first, second| first.path.cmp(&second.path));
                Some(DuplicateGroup { hash, items })
            })
            .collect())
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
                let content_hash = matches!(media_type, MediaType::Photo)
                    .then(|| compute_file_hash(&path))
                    .transpose()?;
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
                    content_hash,
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

fn next_root_id(roots: &[LibraryRoot]) -> String {
    let mut index = 1;
    loop {
        let candidate = format!("root-{index}");
        if !roots.iter().any(|root| root.id == candidate) {
            return candidate;
        }
        index += 1;
    }
}

fn recount_root_counts(roots: &mut [LibraryRoot], media_items: &[MediaItem]) {
    for root in roots.iter_mut() {
        root.photo_count = media_items
            .iter()
            .filter(|item| item.root_id == root.id && matches!(item.media_type, MediaType::Photo))
            .count() as u64;
        root.video_count = media_items
            .iter()
            .filter(|item| item.root_id == root.id && matches!(item.media_type, MediaType::Video))
            .count() as u64;
        root.media_count = root.photo_count + root.video_count;
    }
}

fn compute_file_hash(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("failed to open '{}' for hashing: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];

    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("failed to hash '{}': {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(format!("{:x}", hasher.finalize()))
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
