mod commands;
mod library;
mod models;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(library::LibraryState::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::add_library_root,
            commands::list_library_roots,
            commands::scan_library_root,
            commands::library_overview
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
