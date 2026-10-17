// Release builds are a GUI app without a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autoedit;
mod editor;
mod export;
mod format;
mod home;
mod logs;
mod media;
mod models;
mod paths;
mod projects;
mod secrets;
mod settings;
mod settings_page;
mod system;
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
    let log_file = logs::init(paths::data_dir().ok().map(|dir| dir.join("logs")).as_deref());
    let see_log = log_file
        .map(|path| {
            format!(
                "

Details are in the log:
{}",
                path.display()
            )
        })
        .unwrap_or_default();
    // The panic itself is already in the log (see `logs`); this tells the user.
    match std::panic::catch_unwind(app::run) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            log::error!("{error}");
            let _ = rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("ViralFlow could not start")
                .set_description(format!("{error}{see_log}"))
                .show();
            std::process::exit(1);
        }
        Err(_) => {
            let _ = rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("ViralFlow stopped")
                .set_description(format!("ViralFlow ran into an unexpected error and has to close. Changes made after the last save may be lost.{see_log}"))
                .show();
            std::process::exit(101);
        }
    }
}
