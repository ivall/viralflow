use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::paths::{safe_id, write_atomic};

/// Projects are stored as opaque JSON documents: the frontend owns the schema,
/// the backend only reads the few header fields it needs for listing.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub width: u64,
    pub height: u64,
    pub duration: f64,
    pub thumbnail: Option<String>,
}

fn project_dir(root: &Path, id: &str) -> AppResult<PathBuf> {
    Ok(root.join(safe_id(id)?))
}

fn document_duration(document: &Value) -> f64 {
    let mut end: f64 = 0.0;
    if let Some(tracks) = document.get("tracks").and_then(Value::as_array) {
        for track in tracks {
            for item in track.get("items").and_then(Value::as_array).into_iter().flatten() {
                let start = item.get("start").and_then(Value::as_f64).unwrap_or(0.0);
                let duration = item.get("duration").and_then(Value::as_f64).unwrap_or(0.0);
                end = end.max(start + duration);
            }
        }
    }
    end
}

fn summary(dir: &Path, document: &Value) -> Option<ProjectSummary> {
    let text = |key: &str| document.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let thumbnail = dir.join("thumbnail.jpg");
    Some(ProjectSummary {
        id: document.get("id")?.as_str()?.to_string(),
        name: text("name"),
        created_at: text("createdAt"),
        updated_at: text("updatedAt"),
        width: document.get("width").and_then(Value::as_u64).unwrap_or(1080),
        height: document.get("height").and_then(Value::as_u64).unwrap_or(1920),
        duration: document_duration(document),
        thumbnail: thumbnail.is_file().then(|| thumbnail.display().to_string()),
    })
}

pub fn list(root: &Path) -> AppResult<Vec<ProjectSummary>> {
    let mut result = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let Ok(entry) = entry else { continue };
        let dir = entry.path();
        let Ok(bytes) = std::fs::read(dir.join("project.json")) else { continue };
        let Ok(document) = serde_json::from_slice::<Value>(&bytes) else {
            log::warn!("skipping unreadable project {}", dir.display());
            continue;
        };
        if let Some(item) = summary(&dir, &document) {
            result.push(item);
        }
    }
    result.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(result)
}

pub fn load(root: &Path, id: &str) -> AppResult<Value> {
    let path = project_dir(root, id)?.join("project.json");
    let bytes = std::fs::read(&path).map_err(|_| AppError::msg("This project no longer exists."))?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn save(root: &Path, id: &str, document: &Value) -> AppResult<()> {
    let document_id = document.get("id").and_then(Value::as_str).unwrap_or_default();
    if document_id != id {
        return Err(AppError::msg("Project id mismatch"));
    }
    let dir = project_dir(root, id)?;
    let path = dir.join("project.json");
    // Keep one previous version as a safety net against bad saves.
    if path.is_file() {
        let _ = std::fs::copy(&path, dir.join("project.backup.json"));
    }
    write_atomic(&path, &serde_json::to_vec(document)?)
}

pub fn delete(root: &Path, id: &str) -> AppResult<()> {
    let dir = project_dir(root, id)?;
    if dir.is_dir() {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}

pub fn save_thumbnail(root: &Path, id: &str, bytes: &[u8]) -> AppResult<String> {
    let dir = project_dir(root, id)?;
    // The editor saves a thumbnail when it closes; a project deleted meanwhile must stay deleted.
    if !dir.join("project.json").is_file() {
        return Err(AppError::msg("This project no longer exists."));
    }
    let path = dir.join("thumbnail.jpg");
    write_atomic(&path, bytes)?;
    Ok(path.display().to_string())
}

pub fn duplicate(root: &Path, id: &str, new_id: &str, name: &str, now: &str) -> AppResult<Value> {
    let mut document = load(root, id)?;
    safe_id(new_id)?;
    document["id"] = Value::String(new_id.to_string());
    document["name"] = Value::String(name.to_string());
    document["createdAt"] = Value::String(now.to_string());
    document["updatedAt"] = Value::String(now.to_string());
    save(root, new_id, &document)?;
    let thumbnail = project_dir(root, id)?.join("thumbnail.jpg");
    if thumbnail.is_file() {
        let _ = std::fs::copy(thumbnail, project_dir(root, new_id)?.join("thumbnail.jpg"));
    }
    Ok(document)
}
