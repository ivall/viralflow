//! Answers tool calls on the UI thread: projects here, everything about the
//! open project in the editor (`editor/agent.rs`).

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde_json::{json, Value};
use vf_core::presets::{aspect_label, create_project, ASPECT_PRESETS};

use super::args::{self, Args};
use super::{jobs, round, specs, Origin, Reply, ToolOutput};
use crate::app::Context;
use crate::editor::EditorController;
use crate::home::Home;
use crate::{media, projects};

pub struct Host {
    context: Rc<Context>,
    editor: Rc<EditorController>,
    home: Rc<Home>,
    /// The "an AI app is editing" note was shown.
    told: Cell<bool>,
}

impl Host {
    pub fn new(context: Rc<Context>, editor: Rc<EditorController>, home: Rc<Home>) -> Rc<Self> {
        Rc::new(Host { context, editor, home, told: Cell::new(false) })
    }

    pub fn editor(&self) -> &Rc<EditorController> {
        &self.editor
    }

    pub fn run(&self, name: &str, value: &Value, origin: &Origin) -> Reply {
        let Some(spec) = specs::all().into_iter().find(|s| s.name == name) else {
            return Reply::error(format!("There is no tool named \"{name}\"."));
        };
        let a = match args::object(value) {
            Ok(a) => a,
            Err(error) => return Reply::error(error),
        };
        if spec.app && matches!(origin, Origin::Chat(_)) {
            return Reply::error("The chat works on the project it belongs to.");
        }
        let result = match name {
            "list_projects" => self.list_projects(),
            "open_project" => self.open_project(&a),
            "create_project" => self.create_project(&a),
            "job_status" => return job_status(&a),
            "find_media_files" => return find_media_files(&a),
            _ => {
                if let Origin::Chat(project) = origin {
                    if self.editor.project_id().as_deref() != Some(project.as_str()) {
                        return Reply::error("The project this chat belongs to is no longer open.");
                    }
                }
                if *origin == Origin::Mcp && !spec.read_only && !self.told.replace(true) {
                    self.context.toasts.info("An AI app is editing", "Changes from Claude, Cursor or another connected app appear live; Ctrl+Z undoes them.");
                }
                return self.editor.agent_tool(name, value);
            }
        };
        Reply::Now(ToolOutput::from_result(result))
    }

    fn list_projects(&self) -> Result<Value, String> {
        let list = projects::list(&self.context.paths.projects).map_err(|e| e.to_string())?;
        let open = self.editor.project_id();
        let rows: Vec<Value> = list
            .iter()
            .map(|p| {
                json!({
                    "id": p.id,
                    "name": p.name,
                    "aspect": aspect_label(p.width, p.height),
                    "duration": round(p.duration),
                    "updated": p.updated_at,
                    "open": open.as_deref() == Some(p.id.as_str()),
                })
            })
            .collect();
        Ok(json!({ "summary": format!("{} projects", rows.len()), "projects": rows }))
    }

    fn open_project(&self, a: &Args) -> Result<Value, String> {
        let id = args::required_string(a, "project_id")?;
        projects::load(&self.context.paths.projects, &id).map_err(|e| format!("Project \"{id}\" can't be opened: {e}. Call list_projects for the ids."))?;
        if self.editor.project_id().as_deref() != Some(id.as_str()) {
            self.editor.open(&id);
        }
        self.opened()
    }

    fn create_project(&self, a: &Args) -> Result<Value, String> {
        let name = args::string(a, "name")?.unwrap_or_default();
        let aspect = args::string(a, "aspect")?.unwrap_or_else(|| "9:16".into());
        let preset = ASPECT_PRESETS
            .iter()
            .find(|p| p.id == aspect.trim())
            .ok_or_else(|| format!("aspect \"{aspect}\" is not one of: {}", ASPECT_PRESETS.iter().map(|p| p.id).collect::<Vec<_>>().join(", ")))?;
        let fps = args::number(a, "fps")?.map(|f| f.round() as u32).filter(|f| vf_core::model::FPS_CHOICES.contains(f)).unwrap_or(30);
        let project = create_project(&name, preset.width, preset.height, fps);
        projects::save(&self.context.paths.projects, &project).map_err(|e| e.to_string())?;
        self.home.refresh();
        self.editor.open(&project.id);
        self.opened()
    }

    /// What the editor shows after opening a project.
    fn opened(&self) -> Result<Value, String> {
        match self.editor.agent_tool("get_project", &json!({})) {
            Reply::Now(output) if !output.error => {
                let name = output.value["project"]["name"].as_str().unwrap_or_default().to_string();
                let mut value = output.value;
                value["summary"] = json!(format!("Opened \"{name}\""));
                Ok(value)
            }
            Reply::Now(output) => Err(output.summary()),
            Reply::Later(_) => Err("The project could not be read.".into()),
        }
    }
}

fn job_status(a: &Args) -> Reply {
    let id = match args::required_string(a, "job_id") {
        Ok(id) => id,
        Err(error) => return Reply::error(error),
    };
    let wait = args::number(a, "wait_seconds").ok().flatten().unwrap_or(30.0).clamp(0.0, 120.0);
    match jobs::find(&id) {
        Some(job) => Reply::Later(Box::new(move |budget| job.output(budget.min(std::time::Duration::from_secs_f64(wait))))),
        None => Reply::error(format!("There is no job \"{id}\" (jobs are kept while ViralFlow runs).")),
    }
}

fn media_kind(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_string_lossy().to_lowercase();
    if !media::MEDIA_EXTENSIONS.contains(&extension.as_str()) {
        return None;
    }
    Some(match extension.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "tif" | "tiff" | "heic" | "avif" => "image",
        "mp3" | "wav" | "m4a" | "aac" | "flac" | "ogg" | "oga" | "opus" | "wma" | "aiff" => "audio",
        _ => "video",
    })
}

fn find_media_files(a: &Args) -> Reply {
    let (folder, recursive) = match (args::string(a, "folder"), args::boolean(a, "recursive")) {
        (Ok(folder), Ok(recursive)) => (folder, recursive.unwrap_or(false)),
        (Err(error), _) | (_, Err(error)) => return Reply::error(error),
    };
    let folders: Vec<PathBuf> = match folder {
        Some(folder) => {
            let path = PathBuf::from(folder.trim());
            if !path.is_dir() {
                return Reply::error(format!("{} is not a folder.", path.display()));
            }
            vec![path]
        }
        None => [dirs::video_dir(), dirs::picture_dir(), dirs::audio_dir(), dirs::download_dir(), dirs::desktop_dir()].into_iter().flatten().collect(),
    };
    Reply::Later(Box::new(move |_| {
        const MOST: usize = 400;
        let mut files = Vec::new();
        let mut stack: Vec<(PathBuf, usize)> = folders.into_iter().map(|f| (f, 0)).collect();
        while let Some((dir, depth)) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(meta) = entry.metadata() else { continue };
                if meta.is_dir() {
                    let hidden = path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'));
                    if recursive && depth < 6 && !hidden {
                        stack.push((path, depth + 1));
                    }
                } else if let Some(kind) = media_kind(&path) {
                    let modified = meta.modified().ok().map(|t| chrono::DateTime::<chrono::Utc>::from(t).format("%Y-%m-%d %H:%M").to_string());
                    files.push((
                        meta.modified().ok(),
                        json!({ "path": path.display().to_string(), "kind": kind, "mb": round(meta.len() as f64 / 1_048_576.0), "modified": modified }),
                    ));
                }
            }
        }
        let total = files.len();
        // Newest first: what the user just recorded or downloaded.
        files.sort_by_key(|file| std::cmp::Reverse(file.0));
        let list: Vec<Value> = files.into_iter().take(MOST).map(|(_, v)| v).collect();
        let mut value = json!({ "summary": format!("Found {total} media files"), "files": list });
        if total > MOST {
            value["note"] = json!(format!("Showing the newest {MOST} of {total}; give a narrower folder."));
        }
        ToolOutput::ok(value)
    }))
}
