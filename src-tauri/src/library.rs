use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use crate::models::{LibraryOverview, LibraryRoot, LibraryRootStatus, ScanStats};

#[derive(Default)]
pub struct LibraryState {
    roots: Mutex<Vec<LibraryRoot>>,
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

    pub fn finish_scan(&self, stats: &ScanStats) -> Result<(), String> {
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
}

pub fn scan_root(root_id: String, root_path: PathBuf) -> Result<ScanStats, String> {
    let mut pending = VecDeque::from([root_path]);
    let mut photo_count = 0;
    let mut skipped_count = 0;

    while let Some(path) = pending.pop_front() {
        let Ok(entries) = fs::read_dir(&path) else {
            skipped_count += 1;
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_dir() {
                pending.push_back(path);
            } else if is_supported_image(&path) {
                photo_count += 1;
            } else {
                skipped_count += 1;
            }
        }
    }

    Ok(ScanStats {
        root_id,
        photo_count,
        skipped_count,
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
