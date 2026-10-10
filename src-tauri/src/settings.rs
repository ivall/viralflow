use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use crate::error::AppResult;
use crate::paths::{write_atomic, AppPaths};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Folder or full path of ffmpeg; empty = auto-detect.
    pub ffmpeg_path: String,
    pub openrouter_api_key: String,
    pub openrouter_model: String,
    pub openrouter_transcription_model: String,
    pub transcription_provider: String,
    pub whisper_model: String,
    pub caption_language: String,
    pub default_aspect: String,
    pub default_fps: u32,
    pub autosave: bool,
    pub snap: bool,
    pub safe_area: bool,
    pub export_folder: String,
    pub hardware_encoding: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            ffmpeg_path: String::new(),
            openrouter_api_key: String::new(),
            openrouter_model: "openai/gpt-6-luna".into(),
            openrouter_transcription_model: crate::ai::openrouter::DEFAULT_TRANSCRIPTION_MODEL.into(),
            transcription_provider: "local".into(),
            whisper_model: "base".into(),
            caption_language: "auto".into(),
            default_aspect: "9:16".into(),
            default_fps: 30,
            autosave: true,
            snap: true,
            safe_area: false,
            export_folder: String::new(),
            hardware_encoding: false,
        }
    }
}

pub struct SettingsStore {
    path: std::path::PathBuf,
    value: RwLock<Settings>,
}

impl SettingsStore {
    pub fn load(paths: &AppPaths) -> Self {
        let path = paths.data.join("settings.json");
        let value = std::fs::read(&path).ok().and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok()).unwrap_or_default();
        SettingsStore { path, value: RwLock::new(value) }
    }

    pub fn get(&self) -> Settings {
        self.value.read().expect("settings lock").clone()
    }

    pub fn set(&self, settings: Settings) -> AppResult<Settings> {
        write_atomic(&self.path, &serde_json::to_vec_pretty(&settings)?)?;
        *self.value.write().expect("settings lock") = settings.clone();
        Ok(settings)
    }
}
