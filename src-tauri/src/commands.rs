use std::{
    collections::HashSet,
    ffi::c_void,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use exif::{In, Reader as ExifReader, Tag};
use time::OffsetDateTime;

use tauri::{AppHandle, State};

use crate::{
    inference,
    library::{scan_root, LibraryState, ScanResult},
    models::{
        AiModelInfo, CacheClearResult, DatabaseStats, DuplicateGroup, FaceAnalysisResult,
        FaceAnalysisStatus, FaceCandidate, FolderAnalysisResult, FolderOperationResult,
        LibraryOverview, LibraryRoot, MediaDeleteResult, MediaItem, MediaMetadata,
        ModelDeleteResult, ModelInstallResult, ScanStats, SpaceCatalog,
    },
    services,
};

pub fn hydrate_current_space(state: &LibraryState, app: &AppHandle) -> Result<(), String> {
    let (roots, media_items, metadata, faces) = services::load_library_snapshot(app)?;
    state.hydrate(roots, media_items, metadata, faces)?;

    if let Ok(samples_path) = services::ensure_samples_directory(app) {
        let root = state.add_root(samples_path.to_string_lossy().to_string())?;
        services::record_root(app, &root.id, &root.name, &root.path)?;
        if let Ok(scan_result) = scan_root(root.id.clone(), PathBuf::from(&root.path)) {
            let stats = state.finish_scan(scan_result)?;
            let media = state.media(&stats.root_id, 0, usize::MAX)?;
            let payload = media
                .into_iter()
                .map(|item| {
                    (
                        item.id,
                        item.name,
                        item.path,
                        format!("{:?}", item.media_type).to_ascii_lowercase(),
                        item.content_hash,
                    )
                })
                .collect::<Vec<_>>();
            services::record_media(app, &stats.root_id, &payload)?;
        }
    }

    Ok(())
}

#[tauri::command]
pub fn add_library_root(
    path: String,
    state: State<'_, LibraryState>,
    app: AppHandle,
) -> Result<LibraryRoot, String> {
    let root = state.add_root(path)?;
    services::record_root(&app, &root.id, &root.name, &root.path)?;
    Ok(root)
}

#[tauri::command]
pub fn list_library_roots(state: State<'_, LibraryState>) -> Result<Vec<LibraryRoot>, String> {
    state.roots()
}

#[tauri::command]
pub fn list_spaces(app: AppHandle) -> Result<SpaceCatalog, String> {
    services::list_spaces(&app)
}

#[tauri::command]
pub fn create_space(
    name: String,
    state: State<'_, LibraryState>,
    app: AppHandle,
) -> Result<SpaceCatalog, String> {
    let catalog = services::create_space(&app, &name)?;
    hydrate_current_space(state.inner(), &app)?;
    Ok(catalog)
}

#[tauri::command]
pub fn select_space(
    space_id: String,
    state: State<'_, LibraryState>,
    app: AppHandle,
) -> Result<SpaceCatalog, String> {
    let catalog = services::select_space(&app, &space_id)?;
    hydrate_current_space(state.inner(), &app)?;
    Ok(catalog)
}

#[tauri::command]
pub fn remove_library_root(
    root_id: String,
    state: State<'_, LibraryState>,
    app: AppHandle,
) -> Result<FolderOperationResult, String> {
    let removed_media = state.remove_root(&root_id)?;
    services::remove_root(&app, &root_id)?;
    let _ = services::clear_cache(&app);
    Ok(FolderOperationResult {
        root_id,
        affected_media: removed_media.len() as u64,
        message: format!(
            "Removed folder from Moments and cleared cached media for {} items.",
            removed_media.len()
        ),
    })
}

#[tauri::command]
pub fn list_duplicate_groups(
    root_id: Option<String>,
    state: State<'_, LibraryState>,
) -> Result<Vec<DuplicateGroup>, String> {
    state.duplicate_groups(root_id.as_deref())
}

#[tauri::command]
pub async fn delete_media_items(
    media_ids: Vec<String>,
    state: State<'_, LibraryState>,
    app: AppHandle,
) -> Result<MediaDeleteResult, String> {
    let candidates = state.media_by_ids(&media_ids)?;
    let (deleted_ids, failed_paths) =
        tauri::async_runtime::spawn_blocking(move || delete_media_files(candidates))
            .await
            .map_err(|error| format!("media deletion task failed: {error}"))??;

    if !deleted_ids.is_empty() {
        state.delete_media_items(&deleted_ids)?;
        services::delete_media_items(&app, &deleted_ids)?;
        let _ = services::clear_cache(&app);
    }

    Ok(MediaDeleteResult {
        deleted_media: deleted_ids.len() as u64,
        message: format!("Deleted {} duplicate files.", deleted_ids.len()),
        failed_paths,
    })
}

#[tauri::command]
pub fn library_overview(state: State<'_, LibraryState>) -> Result<LibraryOverview, String> {
    state.overview()
}

#[tauri::command]
pub fn get_library_media(
    root_id: String,
    offset: usize,
    limit: usize,
    state: State<'_, LibraryState>,
) -> Result<Vec<MediaItem>, String> {
    state.media(&root_id, offset, limit)
}

#[tauri::command]
pub fn get_media_metadata(
    media_ids: Vec<String>,
    state: State<'_, LibraryState>,
) -> Result<Vec<MediaMetadata>, String> {
    state.metadata_for_media(media_ids)
}

#[tauri::command]
pub fn get_face_candidates(
    media_ids: Vec<String>,
    state: State<'_, LibraryState>,
) -> Result<Vec<FaceCandidate>, String> {
    state.faces_for_media(media_ids)
}

#[tauri::command]
pub fn list_known_people(state: State<'_, LibraryState>) -> Result<Vec<String>, String> {
    state.known_people()
}

#[tauri::command]
pub fn set_media_favorite(
    media_id: String,
    favorite: bool,
    state: State<'_, LibraryState>,
    app: AppHandle,
) -> Result<MediaMetadata, String> {
    let metadata = state.set_favorite(media_id, favorite)?;
    services::record_metadata(&app, &metadata.media_id, metadata.favorite, &metadata.tags)?;
    Ok(metadata)
}

#[tauri::command]
pub fn set_media_tags(
    media_id: String,
    tags: Vec<String>,
    state: State<'_, LibraryState>,
    app: AppHandle,
) -> Result<MediaMetadata, String> {
    let metadata = state.set_tags(media_id, tags)?;
    services::record_metadata(&app, &metadata.media_id, metadata.favorite, &metadata.tags)?;
    Ok(metadata)
}

#[tauri::command]
pub async fn analyze_media_faces(
    media_id: String,
    state: State<'_, LibraryState>,
    app: AppHandle,
) -> Result<FaceAnalysisResult, String> {
    let mut result = state.analyze_faces(media_id)?;
    if let Some(model) = services::installed_model(&app, "Face scanning")? {
        let model_path = services::model_path(&app, &model)?;
        let inference_cache_dir = services::inference_cache_dir(&app)?;
        let media_path = state.media_path(&result.media_id)?;
        let inference_media_id = result.media_id.clone();
        let faces = tauri::async_runtime::spawn_blocking(move || {
            inference::detect_faces(
                &model_path,
                &media_path,
                &inference_cache_dir,
                &inference_media_id,
            )
        })
        .await
        .map_err(|error| format!("face detection task failed: {error}"))??;
        let faces = state.replace_faces_for_media(result.media_id.clone(), faces)?;
        services::replace_faces_for_media(&app, &result.media_id, &faces)?;
        result.status = FaceAnalysisStatus::Ready;
        result.message = format!("ONNX face detection found {} faces.", faces.len());
        result.faces = faces;
    }

    Ok(result)
}

#[tauri::command]
pub fn list_ai_models(app: AppHandle) -> Result<Vec<AiModelInfo>, String> {
    services::available_models(&app)
}

#[tauri::command]
pub async fn install_ai_model(
    model_id: String,
    app: AppHandle,
) -> Result<ModelInstallResult, String> {
    tauri::async_runtime::spawn_blocking(move || services::install_model(&app, &model_id))
        .await
        .map_err(|error| format!("model install task failed: {error}"))?
}

#[tauri::command]
pub fn delete_ai_model(model_id: String, app: AppHandle) -> Result<ModelDeleteResult, String> {
    services::delete_model(&app, &model_id)
}

#[tauri::command]
pub async fn analyze_root_faces(
    root_id: String,
    state: State<'_, LibraryState>,
    app: AppHandle,
) -> Result<FolderAnalysisResult, String> {
    analyze_root_with_model(root_id, "Face scanning", state, app).await
}

#[tauri::command]
pub async fn classify_root_images(
    root_id: String,
    state: State<'_, LibraryState>,
    app: AppHandle,
) -> Result<FolderAnalysisResult, String> {
    analyze_root_with_model(root_id, "Image classification", state, app).await
}

#[tauri::command]
pub fn clear_app_cache(app: AppHandle) -> Result<CacheClearResult, String> {
    services::clear_cache(&app)
}

#[tauri::command]
pub fn get_database_stats(app: AppHandle) -> Result<DatabaseStats, String> {
    services::database_stats(&app)
}

#[tauri::command]
pub fn set_face_name(
    face_id: String,
    name: String,
    state: State<'_, LibraryState>,
    app: AppHandle,
) -> Result<FaceCandidate, String> {
    let updated = state.set_face_name(face_id, name)?;
    let analysis = state.analyze_faces(updated.media_id.clone())?;
    services::replace_faces_for_media(&app, &updated.media_id, &analysis.faces)?;
    Ok(updated)
}

#[tauri::command]
pub fn open_media_path(media_id: String, state: State<'_, LibraryState>) -> Result<(), String> {
    let path = normalize_for_native_open(state.media_path(&media_id)?)?;
    open_with_default_app(&path)
}

#[tauri::command]
pub fn show_media_in_explorer(
    media_id: String,
    state: State<'_, LibraryState>,
) -> Result<(), String> {
    let path = normalize_for_native_open(state.media_path(&media_id)?)?;
    open_in_file_explorer(&path)
}

#[tauri::command]
pub async fn scan_library_root(
    root_id: String,
    state: State<'_, LibraryState>,
    app: AppHandle,
) -> Result<ScanStats, String> {
    let root_path = state.root_path(&root_id)?;
    let scan_root_id = root_id.clone();

    let stats = tauri::async_runtime::spawn_blocking(move || scan_root(scan_root_id, root_path))
        .await
        .map_err(|error| format!("scan task failed: {error}"))?;

    commit_scan_result(&root_id, stats, &state, &app)
}

#[tauri::command]
pub async fn rename_root_media_by_date(
    root_id: String,
    state: State<'_, LibraryState>,
    app: AppHandle,
) -> Result<FolderOperationResult, String> {
    let media = state.media_for_root(&root_id)?;
    let rename_count =
        tauri::async_runtime::spawn_blocking(move || rename_media_files_by_date(media))
            .await
            .map_err(|error| format!("rename task failed: {error}"))??;

    let root_path = state.root_path(&root_id)?;
    let scan_root_id = root_id.clone();
    let stats = tauri::async_runtime::spawn_blocking(move || scan_root(scan_root_id, root_path))
        .await
        .map_err(|error| format!("scan task failed: {error}"))?;
    let _ = commit_scan_result(&root_id, stats, &state, &app)?;
    let _ = services::clear_cache(&app);

    Ok(FolderOperationResult {
        root_id,
        affected_media: rename_count,
        message: format!("Renamed {} files using date-based filenames.", rename_count),
    })
}

fn normalize_for_native_open(path: PathBuf) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("failed to resolve media path: {error}"))?;

    #[cfg(windows)]
    {
        let display = canonical.to_string_lossy();
        if let Some(path) = display.strip_prefix(r"\\?\UNC\") {
            return Ok(PathBuf::from(format!(r"\\{path}")));
        }
        if let Some(path) = display.strip_prefix(r"\\?\") {
            return Ok(PathBuf::from(path));
        }
    }

    Ok(canonical)
}

async fn analyze_root_with_model(
    root_id: String,
    task: &str,
    state: State<'_, LibraryState>,
    app: AppHandle,
) -> Result<FolderAnalysisResult, String> {
    let processed_media = state.photo_count_for_root(&root_id)?;
    let Some(model) = services::installed_model(&app, task)? else {
        return Ok(FolderAnalysisResult {
            root_id,
            task: task.to_string(),
            model_id: String::new(),
            processed_media,
            status: FaceAnalysisStatus::ModelMissing,
            message: format!("Install a {task} model before running folder analysis."),
            faces: Vec::new(),
            metadata: Vec::new(),
        });
    };

    let model_path = services::model_path(&app, &model)?;
    let inference_cache_dir = services::inference_cache_dir(&app)?;
    let photo_media = state.photo_media_for_root(&root_id)?;
    let mut faces = Vec::new();
    let mut metadata = Vec::new();

    for item in photo_media {
        let item_path = PathBuf::from(&item.path);
        if task == "Face scanning" {
            let inference_model_path = model_path.clone();
            let inference_cache_dir = inference_cache_dir.clone();
            let inference_media_id = item.id.clone();
            let detected = tauri::async_runtime::spawn_blocking(move || {
                inference::detect_faces(
                    &inference_model_path,
                    &item_path,
                    &inference_cache_dir,
                    &inference_media_id,
                )
            })
            .await
            .map_err(|error| format!("face detection task failed: {error}"))??;
            let stored_faces = state.replace_faces_for_media(item.id.clone(), detected)?;
            services::replace_faces_for_media(&app, &item.id, &stored_faces)?;
            faces.extend(stored_faces);
        } else {
            let inference_model_path = model_path.clone();
            let inference_cache_dir = inference_cache_dir.clone();
            let tags = tauri::async_runtime::spawn_blocking(move || {
                inference::classify_image(&inference_model_path, &item_path, &inference_cache_dir)
            })
            .await
            .map_err(|error| format!("image classification task failed: {error}"))??;
            let entry = state.add_tags_for_media(item.id.clone(), tags)?;
            services::record_metadata(&app, &entry.media_id, entry.favorite, &entry.tags)?;
            metadata.push(entry);
        }
    }

    Ok(FolderAnalysisResult {
        root_id,
        task: task.to_string(),
        model_id: model.id,
        processed_media,
        status: FaceAnalysisStatus::Ready,
        message: format!(
            "{task} processed {processed_media} photos with ONNX Runtime using the {} model.",
            model.accelerator
        ),
        faces,
        metadata,
    })
}

fn open_with_default_app(path: &PathBuf) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        Command::new("explorer.exe")
            .arg(path)
            .spawn()
            .map_err(|error| format!("failed to open media: {error}"))?;
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        #[cfg(target_os = "macos")]
        let status = Command::new("open").arg(path).status();

        #[cfg(all(unix, not(target_os = "macos")))]
        let status = Command::new("xdg-open").arg(path).status();

        status
            .map_err(|error| format!("failed to open media: {error}"))?
            .success()
            .then_some(())
            .ok_or_else(|| "native viewer returned an error".to_string())
    }
}

fn open_in_file_explorer(path: &PathBuf) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        Command::new("explorer.exe")
            .arg("/select,")
            .arg(path)
            .spawn()
            .map_err(|error| format!("failed to show media in File Explorer: {error}"))?;
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let parent = path
            .parent()
            .ok_or_else(|| "media path does not have a parent directory".to_string())?;
        open_with_default_app(&parent.to_path_buf())
    }
}

fn commit_scan_result(
    root_id: &str,
    scan_result: Result<ScanResult, String>,
    state: &State<'_, LibraryState>,
    app: &AppHandle,
) -> Result<ScanStats, String> {
    match scan_result {
        Ok(stats) => {
            let stats = state.finish_scan(stats)?;
            let media = state
                .media(&stats.root_id, 0, usize::MAX)?
                .into_iter()
                .map(|item| {
                    (
                        item.id,
                        item.name,
                        item.path,
                        format!("{:?}", item.media_type).to_ascii_lowercase(),
                        item.content_hash,
                    )
                })
                .collect::<Vec<_>>();
            services::record_media(app, &stats.root_id, &media)?;
            Ok(stats)
        }
        Err(error) => {
            state.fail_scan(root_id)?;
            state.clear_media(root_id)?;
            Err(error)
        }
    }
}

fn rename_media_files_by_date(media: Vec<MediaItem>) -> Result<u64, String> {
    let mut renamed = 0;
    let mut reserved_paths = HashSet::<PathBuf>::new();

    for item in media {
        let source = PathBuf::from(&item.path);
        if !source.exists() {
            continue;
        }

        let Some(parent) = source.parent() else {
            continue;
        };
        let extension = source
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_default();
        if extension.is_empty() {
            continue;
        }

        let Some(timestamp) = media_timestamp(&source)? else {
            continue;
        };
        let mut destination = parent.join(format!("{timestamp}.{extension}"));
        let mut suffix = 1;
        while (destination.exists() && destination != source)
            || reserved_paths.contains(&destination)
        {
            destination = parent.join(format!("{timestamp}-{suffix:02}.{extension}"));
            suffix += 1;
        }
        reserved_paths.insert(destination.clone());

        if destination != source {
            fs::rename(&source, &destination).map_err(|error| {
                format!(
                    "failed to rename '{}' to '{}': {error}",
                    source.display(),
                    destination.display()
                )
            })?;
            notify_windows_rename(&source, &destination);
            renamed += 1;
        }
    }

    Ok(renamed)
}

fn delete_media_files(media: Vec<MediaItem>) -> Result<(Vec<String>, Vec<String>), String> {
    let mut deleted_ids = Vec::new();
    let mut failed_paths = Vec::new();

    for item in media {
        let path = PathBuf::from(&item.path);
        match fs::remove_file(&path) {
            Ok(()) => deleted_ids.push(item.id),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => deleted_ids.push(item.id),
            Err(_) => failed_paths.push(item.path),
        }
    }

    Ok((deleted_ids, failed_paths))
}

fn media_timestamp(path: &PathBuf) -> Result<Option<String>, String> {
    let datetime = if let Some(exif_datetime) = embedded_media_timestamp(path)? {
        exif_datetime
    } else if let Some(filename_datetime) = filename_timestamp(path) {
        filename_datetime
    } else {
        return Ok(None);
    };

    Ok(Some(format!(
        "{:04}-{:02}-{:02}-{:02}h{:02}m{:02}",
        datetime.year(),
        u8::from(datetime.month()),
        datetime.day(),
        datetime.hour(),
        datetime.minute(),
        datetime.second()
    )))
}

fn embedded_media_timestamp(path: &Path) -> Result<Option<OffsetDateTime>, String> {
    let Some(extension) = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
    else {
        return Ok(None);
    };

    let supports_exif = matches!(
        extension.as_str(),
        "jpg" | "jpeg" | "tif" | "tiff" | "heic" | "heif"
    );
    if !supports_exif {
        return Ok(None);
    }

    let file = fs::File::open(path)
        .map_err(|error| format!("failed to inspect '{}': {error}", path.display()))?;
    let mut reader = std::io::BufReader::new(file);
    let exif = match ExifReader::new().read_from_container(&mut reader) {
        Ok(exif) => exif,
        Err(_) => return Ok(None),
    };

    for tag in [Tag::DateTimeOriginal, Tag::DateTimeDigitized, Tag::DateTime] {
        if let Some(field) = exif.get_field(tag, In::PRIMARY) {
            let value = field.display_value().with_unit(&exif).to_string();
            if let Some(datetime) = parse_exif_datetime(&value) {
                return Ok(Some(datetime));
            }
        }
    }

    Ok(None)
}

fn parse_exif_datetime(value: &str) -> Option<OffsetDateTime> {
    let normalized = value.trim().split_whitespace().collect::<Vec<_>>();
    if normalized.len() < 2 {
        return None;
    }

    let date_parts = normalized[0].split(':').collect::<Vec<_>>();
    let time_parts = normalized[1].split(':').collect::<Vec<_>>();
    if date_parts.len() != 3 || time_parts.len() != 3 {
        return None;
    }

    let year = date_parts[0].parse::<i32>().ok()?;
    let month = date_parts[1].parse::<u8>().ok()?;
    let day = date_parts[2].parse::<u8>().ok()?;
    let hour = time_parts[0].parse::<u8>().ok()?;
    let minute = time_parts[1].parse::<u8>().ok()?;
    let second = time_parts[2].parse::<u8>().ok()?;

    build_timestamp(year, month, day, hour, minute, second)
}

fn filename_timestamp(path: &Path) -> Option<OffsetDateTime> {
    let stem = path.file_stem()?.to_str()?.to_ascii_lowercase();
    let compact = stem
        .chars()
        .map(|character| {
            if character.is_ascii_digit() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>();
    let tokens = compact
        .split_whitespace()
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();

    for window in tokens.windows(2) {
        let [date_token, time_token] = window else {
            continue;
        };
        if date_token.len() == 8 && time_token.len() == 6 {
            if let (Ok(year), Ok(month), Ok(day), Ok(hour), Ok(minute), Ok(second)) = (
                date_token[0..4].parse::<i32>(),
                date_token[4..6].parse::<u8>(),
                date_token[6..8].parse::<u8>(),
                time_token[0..2].parse::<u8>(),
                time_token[2..4].parse::<u8>(),
                time_token[4..6].parse::<u8>(),
            ) {
                if let Some(timestamp) = build_timestamp(year, month, day, hour, minute, second) {
                    return Some(timestamp);
                }
            }
        }
    }

    if tokens.len() >= 6 {
        for window in tokens.windows(6) {
            if let (Ok(year), Ok(month), Ok(day), Ok(hour), Ok(minute), Ok(second)) = (
                window[0].parse::<i32>(),
                window[1].parse::<u8>(),
                window[2].parse::<u8>(),
                window[3].parse::<u8>(),
                window[4].parse::<u8>(),
                window[5].parse::<u8>(),
            ) {
                if let Some(timestamp) = build_timestamp(year, month, day, hour, minute, second) {
                    return Some(timestamp);
                }
            }
        }
    }

    None
}

fn build_timestamp(
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
) -> Option<OffsetDateTime> {
    let month = time::Month::try_from(month).ok()?;
    let date = time::Date::from_calendar_date(year, month, day).ok()?;
    let time = time::Time::from_hms(hour, minute, second).ok()?;
    Some(time::PrimitiveDateTime::new(date, time).assume_utc())
}

#[cfg(windows)]
fn notify_windows_rename(source: &Path, destination: &Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{
        SHChangeNotify, SHCNE_RENAMEITEM, SHCNE_UPDATEDIR, SHCNF_PATHW,
    };

    let source_wide = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<u16>>();
    let destination_wide = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<u16>>();

    unsafe {
        SHChangeNotify(
            SHCNE_RENAMEITEM as i32,
            SHCNF_PATHW,
            source_wide.as_ptr() as *const c_void,
            destination_wide.as_ptr() as *const c_void,
        );
    }

    if let Some(parent) = destination.parent() {
        let parent_wide = parent
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>();
        unsafe {
            SHChangeNotify(
                SHCNE_UPDATEDIR as i32,
                SHCNF_PATHW,
                parent_wide.as_ptr() as *const c_void,
                std::ptr::null(),
            );
        }
    }
}

#[cfg(not(windows))]
fn notify_windows_rename(_source: &Path, _destination: &Path) {}
