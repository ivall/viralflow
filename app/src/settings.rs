use std::sync::RwLock;

use serde::{Deserialize, Serialize};
use vf_core::Result;

use crate::paths::{write_atomic, AppPaths};

/// User preferences, stored as settings.json (same file as the Tauri app).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub openrouter_model: String,
    pub openrouter_transcription_model: String,
    /// "local" (whisper through FFmpeg) or "openrouter".
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
    /// AI apps (Claude, Cursor…) may edit through the local MCP server.
    pub mcp_enabled: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            openrouter_model: "openai/gpt-6-luna".into(),
            openrouter_transcription_model: "openai/whisper-large-v3".into(),
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
            mcp_enabled: true,
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
        self.value.read().map(|value| value.clone()).unwrap_or_default()
    }

    pub fn update(&self, change: impl FnOnce(&mut Settings)) -> Result<Settings> {
        let mut next = self.get();
        change(&mut next);
        write_atomic(&self.path, &serde_json::to_vec_pretty(&next)?)?;
        if let Ok(mut value) = self.value.write() {
            *value = next.clone();
        }
        Ok(next)
    }
}
