//! Turning files into project assets.

use std::path::{Path, PathBuf};

use vf_core::engine::math::uid;
use vf_core::model::MediaAsset;
use vf_core::presets::now_iso;

pub const MEDIA_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "m4v", "mkv", "webm", "avi", "wmv", "flv", "ts", "mts", "m2ts", "3gp", "gif", "png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "heic",
    "avif", "mp3", "wav", "m4a", "aac", "flac", "ogg", "oga", "opus", "wma", "aiff",
];

pub struct Imported {
    pub assets: Vec<MediaAsset>,
    pub failures: Vec<String>,
}

/// Probes files; paths already in `known` reuse their asset.
pub fn import(paths: &[PathBuf], known: &[MediaAsset]) -> Imported {
    let mut result = Imported { assets: vec![], failures: vec![] };
    for path in paths {
        let text = path.display().to_string();
        if let Some(existing) = known.iter().find(|asset| asset.path == text) {
            result.assets.push(existing.clone());
            continue;
        }
        match vf_media::probe::probe(path) {
            Ok(info) => result.assets.push(MediaAsset {
                id: uid("asset"),
                kind: info.kind.unwrap_or_default(),
                name: file_name(path),
                path: text,
                duration: info.duration,
                width: info.width.map(f64::from),
                height: info.height.map(f64::from),
                fps: info.fps,
                has_audio: Some(info.has_audio),
                size: info.size,
                added_at: now_iso(),
                ..Default::default()
            }),
            Err(error) => result.failures.push(error.to_string()),
        }
    }
    result
}

pub fn file_name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string())
}

/// Asks for media files with the system file picker.
pub fn pick_files() -> Vec<PathBuf> {
    rfd::FileDialog::new().set_title("Import media").add_filter("Media", MEDIA_EXTENSIONS).add_filter("All files", &["*"]).pick_files().unwrap_or_default()
}

/// Sound effects that ship with the app (whooshes for auto-edit, pops, clicks).
const BUILTIN_SOUNDS: &[(&str, &[u8])] = &[
    ("boom.mp3", include_bytes!("../../assets/sounds/boom.mp3")),
    ("click.mp3", include_bytes!("../../assets/sounds/click.mp3")),
    ("ding.mp3", include_bytes!("../../assets/sounds/ding.mp3")),
    ("glitch.mp3", include_bytes!("../../assets/sounds/glitch.mp3")),
    ("pop.mp3", include_bytes!("../../assets/sounds/pop.mp3")),
    ("rise.mp3", include_bytes!("../../assets/sounds/rise.mp3")),
    ("swoosh.mp3", include_bytes!("../../assets/sounds/swoosh.mp3")),
    ("tick.mp3", include_bytes!("../../assets/sounds/tick.mp3")),
    ("whoosh-in.mp3", include_bytes!("../../assets/sounds/whoosh-in.mp3")),
    ("whoosh-out.mp3", include_bytes!("../../assets/sounds/whoosh-out.mp3")),
];

/// Writes the bundled sounds to `dir` (once; files already there are kept).
pub fn install_builtin_sounds(dir: &Path) {
    let sounds = dir.join("sounds");
    if let Err(error) = std::fs::create_dir_all(&sounds) {
        log::warn!("builtin sounds: {error}");
        return;
    }
    for (name, bytes) in BUILTIN_SOUNDS {
        let path = sounds.join(name);
        if !path.is_file() {
            if let Err(error) = std::fs::write(&path, bytes) {
                log::warn!("builtin sound {name}: {error}");
            }
        }
    }
}

/// Path of a bundled sound (installed by `install_builtin_sounds`).
pub fn builtin_sound(dir: &Path, name: &str) -> PathBuf {
    dir.join("sounds").join(name)
}
