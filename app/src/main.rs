// Release builds are a GUI app without a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autoedit;
mod editor;
mod export;
mod format;
mod home;
mod media;
mod models;
mod paths;
mod projects;
mod secrets;
mod settings;
mod settings_page;
mod tasks;
mod toast;
mod window;

#[cfg(test)]
mod ui_tests;

slint::include_modules!();

/// A shortcut as the platform writes it ("Ctrl+D", or "⌘+D" on macOS,
/// where Slint reports the Command key as Ctrl).
pub fn shortcut_label(shortcut: &str) -> String {
    if cfg!(target_os = "macos") {
        shortcut.replace("Ctrl", "⌘")
    } else {
        shortcut.to_string()
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn")).init();
    if let Err(error) = app::run() {
        log::error!("{error}");
        let _ = rfd::MessageDialog::new().set_level(rfd::MessageLevel::Error).set_title("ViralFlow could not start").set_description(error.to_string()).show();
        std::process::exit(1);
    }
}
