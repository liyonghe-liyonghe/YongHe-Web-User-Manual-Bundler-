#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use manualbundler::commands;

fn main() {
    tauri::Builder::<tauri::Wry>::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::scan_folder,
            commands::preview_bundle,
            commands::write_bundle,
            commands::save_project,
            commands::load_project,
            commands::read_text_file,
            commands::write_text_file,
            commands::open_output_folder,
            commands::import_dropped_files,
            commands::repair_missing_resources,
            commands::auto_repair_missing_resources,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
