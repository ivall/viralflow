//! Projects on disk: `<data>/projects/<id>/project.json` plus `thumbnail.jpg`.

use std::path::{Path, PathBuf};

use serde_json::Value;
use vf_core::migrate::read_project;
use vf_core::model::Project;
use vf_core::{Error, Result};

use crate::paths::{safe_id, write_atomic};

#[derive(Debug, Clone)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub updated_at: String,
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub thumbnail: Option<PathBuf>,
}

fn project_dir(root: &Path, id: &str) -> Result<PathBuf> {
    Ok(root.join(safe_id(id)?))
}

fn summary(dir: &Path, document: &Value) -> Option<ProjectSummary> {
    let text = |key: &str| document.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let mut end: f64 = 0.0;
    for track in document.get("tracks").and_then(Value::as_array).into_iter().flatten() {
        for item in track.get("items").and_then(Value::as_array).into_iter().flatten() {
            let start = item.get("start").and_then(Value::as_f64).unwrap_or(0.0);
            let duration = item.get("duration").and_then(Value::as_f64).unwrap_or(0.0);
            end = end.max(start + duration);
        }
    }
    let thumbnail = dir.join("thumbnail.jpg");
    Some(ProjectSummary {
        id: document.get("id")?.as_str()?.to_string(),
        name: text("name"),
        updated_at: text("updatedAt"),
        width: document.get("width").and_then(Value::as_u64).unwrap_or(1080) as u32,
        height: document.get("height").and_then(Value::as_u64).unwrap_or(1920) as u32,
        duration: end,
        thumbnail: thumbnail.is_file().then_some(thumbnail),
    })
}

/// All projects, most recently edited first.
pub fn list(root: &Path) -> Result<Vec<ProjectSummary>> {
    let mut result = Vec::new();
    for entry in std::fs::read_dir(root)?.flatten() {
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

pub fn load(root: &Path, id: &str) -> Result<Project> {
    let dir = project_dir(root, id)?;
    let bytes = std::fs::read(dir.join("project.json")).map_err(|_| Error::msg("This project no longer exists."))?;
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(document) => read_project(&document),
        // A damaged file (cut short by a crash or a full disk): the copy kept
        // from the save before.
        Err(error) => {
            let backup = std::fs::read(dir.join("project.backup.json")).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok());
            match backup {
                Some(document) => {
                    log::warn!("project {id} is damaged ({error}); opening the previous save");
                    read_project(&document)
                }
                None => Err(Error::msg("This project file is damaged and has no backup.")),
            }
        }
    }
}

pub fn save(root: &Path, project: &Project) -> Result<()> {
    let dir = project_dir(root, &project.id)?;
    let path = dir.join("project.json");
    // Keep one previous version as a safety net against bad saves (only a
    // readable one: a damaged file must not replace a good backup).
    if std::fs::read(&path).ok().is_some_and(|bytes| serde_json::from_slice::<serde::de::IgnoredAny>(&bytes).is_ok()) {
        let _ = std::fs::copy(&path, dir.join("project.backup.json"));
    }
    write_atomic(&path, &serde_json::to_vec(project)?)
}

pub fn delete(root: &Path, id: &str) -> Result<()> {
    let dir = project_dir(root, id)?;
    if dir.is_dir() {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}

pub fn save_thumbnail(root: &Path, id: &str, jpeg: &[u8]) -> Result<PathBuf> {
    let dir = project_dir(root, id)?;
    // A project deleted while its editor was closing must stay deleted.
    if !dir.join("project.json").is_file() {
        return Err(Error::msg("This project no longer exists."));
    }
    let path = dir.join("thumbnail.jpg");
    write_atomic(&path, jpeg)?;
    Ok(path)
}

pub fn duplicate(root: &Path, id: &str, new_id: &str, name: &str) -> Result<Project> {
    let mut project = load(root, id)?;
    safe_id(new_id)?;
    let now = vf_core::presets::now_iso();
    project.id = new_id.into();
    project.name = name.into();
    project.created_at = now.clone();
    project.updated_at = now;
    save(root, &project)?;
    let thumbnail = project_dir(root, id)?.join("thumbnail.jpg");
    if thumbnail.is_file() {
        let _ = std::fs::copy(thumbnail, project_dir(root, new_id)?.join("thumbnail.jpg"));
    }
    Ok(project)
}

/// Reads a project file saved anywhere (any version) as a new project.
pub fn read_file(path: &Path) -> Result<Project> {
    let bytes = std::fs::read(path)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| Error::msg("This file is not a ViralFlow project."))?;
    read_project(&value)
}

pub fn write_file(path: &Path, project: &Project) -> Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(project)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_list_load_delete() {
        let root = std::env::temp_dir().join(format!("vf-projects-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let project = vf_core::presets::create_project("Demo", 1080, 1920, 30);
        save(&root, &project).unwrap();
        let listed = list(&root).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Demo");
        assert_eq!(load(&root, &project.id).unwrap(), project);
        let copy = duplicate(&root, &project.id, "copy-1", "Demo copy").unwrap();
        assert_eq!(list(&root).unwrap().len(), 2);
        delete(&root, &copy.id).unwrap();
        delete(&root, &project.id).unwrap();
        assert!(list(&root).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(root);
        assert!(safe_id("../evil").is_err());
    }

    #[test]
    fn a_damaged_file_opens_the_previous_save() {
        let root = std::env::temp_dir().join(format!("vf-projects-damaged-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut project = vf_core::presets::create_project("Demo", 1080, 1920, 30);
        save(&root, &project).unwrap();
        project.name = "Demo 2".into();
        save(&root, &project).unwrap();
        // Cut short, as by a crash during a write.
        let file = root.join(&project.id).join("project.json");
        let bytes = std::fs::read(&file).unwrap();
        std::fs::write(&file, &bytes[..bytes.len() / 2]).unwrap();
        assert_eq!(load(&root, &project.id).unwrap().name, "Demo");
        // Saving over the damaged file keeps the good backup.
        project.name = "Demo 3".into();
        save(&root, &project).unwrap();
        assert_eq!(load(&root, &project.id).unwrap().name, "Demo 3");
        let backup: Value = serde_json::from_slice(&std::fs::read(root.join(&project.id).join("project.backup.json")).unwrap()).unwrap();
        assert_eq!(backup["name"], "Demo");
        let _ = std::fs::remove_dir_all(root);
    }
}
