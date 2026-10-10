mod ai;
mod commands;
mod engine;
mod error;
mod ffmpeg;
mod fonts;
mod media;
mod model;
mod paths;
mod projects;
mod render;
mod secrets;
mod settings;

use std::sync::Arc;

use tauri::Manager;

use commands::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let paths = paths::AppPaths::resolve(app.handle())?;
            if let Err(error) = commands::install_builtin_sounds(&paths) {
                log::warn!("cannot install built-in sounds: {error}");
            }
            let settings = settings::SettingsStore::load(&paths);
            let _ = app.asset_protocol_scope().allow_directory(&paths.builtin, true);
            let keys = Arc::new(secrets::KeyStore::new(&paths.data));
            app.manage(AppState { paths, settings, keys, jobs: Arc::new(render::Jobs::default()) });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::openrouter_key_status,
            commands::set_openrouter_key,
            commands::tools_status,
            commands::list_projects,
            commands::load_project,
            commands::save_project,
            commands::delete_project,
            commands::duplicate_project,
            commands::save_project_thumbnail,
            commands::write_project_file,
            commands::read_project_file,
            commands::write_text_file,
            commands::read_text_file,
            commands::import_media,
            commands::allow_media,
            commands::check_paths,
            commands::create_proxy,
            commands::media_filmstrip,
            commands::media_waveform,
            commands::builtin_sounds,
            commands::system_fonts,
            commands::reveal_path,
            commands::open_path,
            commands::default_export_path,
            commands::start_export,
            commands::cancel_export,
            commands::whisper_models,
            commands::download_whisper_model,
            commands::transcribe,
            commands::ai_plan,
            commands::ai_rewrite,
            commands::openrouter_models,
            commands::ai_match_images,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ViralFlow");
}
