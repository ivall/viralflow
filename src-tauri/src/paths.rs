use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

use crate::error::{AppError, AppResult};

/// Well-known folders of the application. Everything the app writes lives here.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub data: PathBuf,
    pub cache: PathBuf,
    pub projects: PathBuf,
    pub media_cache: PathBuf,
    pub models: PathBuf,
    pub jobs: PathBuf,
    pub builtin: PathBuf,
}

impl AppPaths {
    pub fn resolve(app: &AppHandle) -> AppResult<Self> {
        let data = app.path().app_data_dir().map_err(|error| AppError::msg(error.to_string()))?;
        let cache = app.path().app_cache_dir().map_err(|error| AppError::msg(error.to_string()))?;
        let paths = AppPaths {
            projects: data.join("projects"),
            models: data.join("models"),
            builtin: data.join("builtin"),
            media_cache: cache.join("media"),
            jobs: cache.join("jobs"),
            data,
            cache,
        };
        for dir in [&paths.data, &paths.cache, &paths.projects, &paths.models, &paths.builtin, &paths.media_cache, &paths.jobs] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(paths)
    }
}

/// Writes a file atomically: a crash mid-write never leaves a truncated file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("{}.tmp", path.extension().and_then(|value| value.to_str()).unwrap_or("dat")));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Accepts only simple identifiers so ids can never escape their folder.
pub fn safe_id(id: &str) -> AppResult<&str> {
    let valid = !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if valid {
        Ok(id)
    } else {
        Err(AppError::msg("Invalid identifier"))
    }
}

/// Stable 64-bit FNV-1a hash used for cache keys.
pub fn fnv64(parts: &[&[u8]]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for part in parts {
        for byte in *part {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
