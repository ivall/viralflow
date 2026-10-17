//! The desktop around the app: the file manager and default applications.

use std::path::Path;

/// Shows a file in the system file manager.
pub fn reveal(path: &Path) {
    // Explorer wants the path quoted after the comma, which normal argument
    // quoting does not do: written out as it is.
    #[cfg(target_os = "windows")]
    let result = {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn()
    };
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn();
    if let Err(error) = result {
        log::warn!("could not reveal {}: {error}", path.display());
    }
}

/// Opens a file with its default application.
pub fn open_path(path: &Path) {
    // Explorer opens a file with its default app; `cmd /C start` would read
    // characters such as & in the file name as commands.
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(path).spawn();
    if let Err(error) = result {
        log::warn!("could not open {}: {error}", path.display());
    }
}
