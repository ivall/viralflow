//! What the app writes down about its own run.
//!
//! A packaged app has no console: whatever it logged about a failed decode or
//! a lost GPU went nowhere, and a bug report came with nothing behind it. So
//! every line also goes to a file, one per run in `<data>/logs`, named for the
//! moment it started. The newest [`KEEP`] files stay, older ones are deleted
//! at start, and a file stops growing at [`CAP`] so a loop that logs every
//! frame cannot fill the disk. Panics, on any thread, are written there with
//! their backtrace.

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Log files kept, this run's included.
const KEEP: usize = 10;
/// Bytes after which a run's file stops growing.
const CAP: u64 = 8 << 20;
const FILTER: &str = "info,wgpu_core=warn,wgpu_hal=warn,naga=warn,whisper_rs=warn";
const PREFIX: &str = "viralflow-";

/// Starts logging to standard error and to a new file in `dir`. Returns the
/// file, or None when it could not be created (logging still goes to stderr).
pub fn init(dir: Option<&Path>) -> Option<PathBuf> {
    let opened = dir.and_then(|dir| match open(dir, &chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string()) {
        Ok(opened) => Some(opened),
        Err(error) => {
            eprintln!("no log file in {}: {error}", dir.display());
            None
        }
    });
    let path = opened.as_ref().map(|(path, _)| path.clone());
    let tee = Tee { file: opened.map(|(_, file)| file), written: 0 };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(FILTER))
        .target(env_logger::Target::Pipe(Box::new(tee)))
        .format_timestamp_millis()
        .init();
    install_panic_hook();
    log::info!("ViralFlow {} on {} {} ({})", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH, vf_media::version());
    path
}

/// Deletes all but the newest `KEEP - 1` logs, then creates this run's file.
fn open(dir: &Path, stamp: &str) -> io::Result<(PathBuf, File)> {
    std::fs::create_dir_all(dir)?;
    let mut old: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(PREFIX) && n.ends_with(".log")))
        .collect();
    // The names are timestamps, so they sort oldest first.
    old.sort();
    let surplus = (old.len() + 1).saturating_sub(KEEP);
    for path in &old[..surplus] {
        let _ = std::fs::remove_file(path);
    }
    let mut path = dir.join(format!("{PREFIX}{stamp}.log"));
    if path.exists() {
        path = dir.join(format!("{PREFIX}{stamp}-{}.log", std::process::id()));
    }
    let file = File::create(&path)?;
    Ok((path, file))
}

/// Every record to stderr and, up to `CAP`, to the run's file. The file is
/// written unbuffered (one write per record), so the last lines before a
/// crash are on disk.
struct Tee {
    file: Option<File>,
    written: u64,
}

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // A release build on Windows has no console; stderr then fails, which is fine.
        let _ = io::stderr().write_all(buf);
        if let Some(file) = self.file.as_mut() {
            if self.written < CAP && file.write_all(buf).is_ok() {
                self.written += buf.len() as u64;
                if self.written >= CAP {
                    let _ = file.write_all(b"--- log capped; later lines go to stderr only ---\n");
                }
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let _ = io::stderr().flush();
        Ok(())
    }
}

/// Panics go to the log with the thread, the place and a backtrace, instead
/// of only to a console nobody sees.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        let payload = info.payload();
        let message = payload.downcast_ref::<&str>().copied().or_else(|| payload.downcast_ref::<String>().map(String::as_str)).unwrap_or("(no message)");
        let location = info.location().map(|l| l.to_string()).unwrap_or_else(|| "an unknown place".into());
        log::error!("panic in thread '{}' at {location}: {message}\n{}", thread.name().unwrap_or("unnamed"), std::backtrace::Backtrace::force_capture());
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_the_newest_logs() {
        let dir = std::env::temp_dir().join(format!("vf-logs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for day in 1..=12 {
            std::fs::write(dir.join(format!("{PREFIX}2026-01-{day:02}_10-00-00.log")), b"x").unwrap();
        }
        std::fs::write(dir.join("notes.txt"), b"keep me").unwrap();
        let (path, _) = open(&dir, "2026-02-01_10-00-00").unwrap();
        let mut logs: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        logs.sort();
        assert_eq!(logs.len(), KEEP + 1, "{logs:?}");
        assert!(logs.contains(&"notes.txt".to_string()));
        assert!(!logs.iter().any(|n| n.contains("2026-01-03")), "oldest deleted: {logs:?}");
        assert!(path.ends_with(format!("{PREFIX}2026-02-01_10-00-00.log")));
        // The same second twice gets a second file.
        let (again, _) = open(&dir, "2026-02-01_10-00-00").unwrap();
        assert_ne!(again, path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_file_stops_growing_at_the_cap() {
        let dir = std::env::temp_dir().join(format!("vf-logs-cap-{}", std::process::id()));
        let (path, file) = open(&dir, "cap").unwrap();
        let mut tee = Tee { file: Some(file), written: CAP - 10 };
        tee.write_all(b"0123456789abcdef\n").unwrap();
        tee.write_all(b"dropped\n").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("0123456789abcdef\n--- log capped"), "{text}");
        assert!(!text.contains("dropped"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
