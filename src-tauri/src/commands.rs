use tauri::State;

use crate::{
    library::{scan_root, LibraryState},
    models::{LibraryOverview, LibraryRoot, MediaItem, ScanStats},
};

#[tauri::command]
pub fn add_library_root(
    path: String,
    state: State<'_, LibraryState>,
) -> Result<LibraryRoot, String> {
    state.add_root(path)
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
pub async fn scan_library_root(
    root_id: String,
    state: State<'_, LibraryState>,
) -> Result<ScanStats, String> {
    let root_path = state.root_path(&root_id)?;
    let scan_root_id = root_id.clone();

    let stats = tauri::async_runtime::spawn_blocking(move || scan_root(scan_root_id, root_path))
        .await
        .map_err(|error| format!("scan task failed: {error}"))?;

    match stats {
        Ok(stats) => state.finish_scan(stats),
        Err(error) => {
            state.fail_scan(&root_id)?;
            state.clear_media(&root_id)?;
            Err(error)
        }
    }
}
