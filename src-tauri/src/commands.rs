use std::{path::PathBuf, process::Command};

use tauri::{AppHandle, State};

use crate::{
    library::{scan_root, LibraryState},
    models::{
        AiModelInfo, CacheClearResult, DatabaseStats, FaceAnalysisResult, FaceAnalysisStatus,
        FaceCandidate, FolderAnalysisResult, LibraryOverview, LibraryRoot, MediaItem,
        MediaMetadata, ModelDeleteResult, ModelInstallResult, ScanStats,
    },
    services,
};

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
pub fn analyze_media_faces(
    media_id: String,
    state: State<'_, LibraryState>,
    app: AppHandle,
) -> Result<FaceAnalysisResult, String> {
    let mut result = state.analyze_faces(media_id)?;
    let face_model_installed = services::available_models(&app)?
        .into_iter()
        .any(|model| model.task == "Face scanning" && model.installed);

    if face_model_installed {
        result.status = FaceAnalysisStatus::Ready;
        result.message = "Face detection model is installed and ready for local inference. The detection runtime will use this model bundle.".to_string();
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
pub fn analyze_root_faces(
    root_id: String,
    state: State<'_, LibraryState>,
    app: AppHandle,
) -> Result<FolderAnalysisResult, String> {
    analyze_root_with_model(root_id, "Face scanning", state, app)
}

#[tauri::command]
pub fn classify_root_images(
    root_id: String,
    state: State<'_, LibraryState>,
    app: AppHandle,
) -> Result<FolderAnalysisResult, String> {
    analyze_root_with_model(root_id, "Image classification", state, app)
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
) -> Result<FaceCandidate, String> {
    state.set_face_name(face_id, name)
}

#[tauri::command]
pub fn open_media_path(media_id: String, state: State<'_, LibraryState>) -> Result<(), String> {
    let path = normalize_for_native_open(state.media_path(&media_id)?)?;
    open_with_default_app(&path)
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

    match stats {
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
                    )
                })
                .collect::<Vec<_>>();
            services::record_media(&app, &stats.root_id, &media)?;
            Ok(stats)
        }
        Err(error) => {
            state.fail_scan(&root_id)?;
            state.clear_media(&root_id)?;
            Err(error)
        }
    }
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

fn analyze_root_with_model(
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

    let (faces, metadata) = if task == "Face scanning" {
        let faces = state.scan_faces_for_root(&root_id)?;
        services::record_faces(&app, &faces)?;
        (faces, Vec::new())
    } else {
        let metadata = state.add_feature_tags_for_root(&root_id)?;
        for entry in &metadata {
            services::record_metadata(&app, &entry.media_id, entry.favorite, &entry.tags)?;
        }
        (Vec::new(), metadata)
    };

    Ok(FolderAnalysisResult {
        root_id,
        task: task.to_string(),
        model_id: model.id,
        processed_media,
        status: FaceAnalysisStatus::Ready,
        message: format!(
            "{task} processed {processed_media} photos in this folder. The {} model is installed for this task.",
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
