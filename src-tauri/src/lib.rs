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
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let state = app.state::<library::LibraryState>();
            let _ = commands::hydrate_current_space(state.inner(), &app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::add_library_root,
            commands::create_space,
            commands::delete_media_items,
            commands::get_library_media,
            commands::get_media_metadata,
            commands::get_face_candidates,
            commands::get_database_stats,
            commands::get_updater_state,
            commands::classify_root_images,
            commands::delete_ai_model,
            commands::install_ai_model,
            commands::list_duplicate_groups,
            commands::list_ai_models,
            commands::list_library_roots,
            commands::list_known_people,
            commands::list_spaces,
            commands::open_media_path,
            commands::show_media_in_explorer,
            commands::remove_library_root,
            commands::rename_root_media_by_date,
            commands::scan_library_root,
            commands::select_space,
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
