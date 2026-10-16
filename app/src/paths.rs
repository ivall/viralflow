use std::path::{Path, PathBuf};

use vf_core::{Error, Result};

/// Same identifier as the Tauri app, so existing projects and settings are found.
const IDENTIFIER: &str = "io.github.ivall.viralflow";

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
    pub fn resolve() -> Result<Self> {
        let data = dirs::data_dir().ok_or_else(|| Error::msg("No application data folder"))?.join(IDENTIFIER);
        let cache = dirs::cache_dir().ok_or_else(|| Error::msg("No cache folder"))?.join(IDENTIFIER);
        let paths = AppPaths {
            projects: data.join("projects"),
            models: data.join("models"),
            builtin: data.join("builtin"),
            media_cache: cache.join("media"),
            jobs: cache.join("jobs"),
            data,
            cache,
        };
        paths.create()?;
        Ok(paths)
    }

    /// Everything under one folder (tests: never the user's data).
    #[cfg(test)]
    pub fn in_dir(root: &Path) -> Result<Self> {
        let paths = AppPaths {
            data: root.join("data"),
            cache: root.join("cache"),
            projects: root.join("data/projects"),
            models: root.join("data/models"),
            builtin: root.join("data/builtin"),
            media_cache: root.join("cache/media"),
            jobs: root.join("cache/jobs"),
        };
        paths.create()?;
        Ok(paths)
    }

    fn create(&self) -> Result<()> {
        for dir in [&self.data, &self.cache, &self.projects, &self.models, &self.builtin, &self.media_cache, &self.jobs] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

/// Writes a file atomically: a crash mid-write never leaves a truncated file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("{}.tmp", path.extension().and_then(|value| value.to_str()).unwrap_or("dat")));
    {
        use std::io::Write;
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        // On disk before it replaces the old file, so a power cut leaves one or the other.
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Accepts only simple identifiers so ids can never escape their folder.
pub fn safe_id(id: &str) -> Result<&str> {
    let valid = !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if valid {
        Ok(id)
    } else {
        Err(Error::msg("Invalid identifier"))
    }
}
