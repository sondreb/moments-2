use tauri::Manager;

mod commands;
mod inference;
mod library;
mod models;
mod services;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(library::LibraryState::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let state = app.state::<library::LibraryState>();
            if let Ok((roots, media_items, metadata, faces)) =
                services::load_library_snapshot(&app.handle())
            {
                let _ = state.hydrate(roots, media_items, metadata, faces);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::add_library_root,
            commands::delete_media_items,
            commands::get_library_media,
            commands::get_media_metadata,
            commands::get_database_stats,
            commands::classify_root_images,
            commands::delete_ai_model,
            commands::install_ai_model,
            commands::list_duplicate_groups,
            commands::list_ai_models,
            commands::list_library_roots,
            commands::open_media_path,
            commands::remove_library_root,
            commands::rename_root_media_by_date,
            commands::scan_library_root,
            commands::set_face_name,
            commands::set_media_favorite,
            commands::set_media_tags,
            commands::analyze_media_faces,
            commands::analyze_root_faces,
            commands::clear_app_cache,
            commands::library_overview
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
