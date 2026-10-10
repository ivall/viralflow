use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::ai::captions::{build_transcript, CaptionOptions, Transcript};
use crate::ai::{self, openrouter, whisper};
use crate::error::{AppError, AppResult};
use crate::ffmpeg::{self, ToolsStatus};
use crate::media::{self, Filmstrip, ImportedMedia, MediaKind};
use crate::paths::AppPaths;
use crate::projects::{self, ProjectSummary};
use crate::render::{self, ExportProgress, ExportRequest, Jobs};
use crate::settings::{Settings, SettingsStore};

pub struct AppState {
    pub paths: AppPaths,
    pub settings: SettingsStore,
    pub jobs: Arc<Jobs>,
}

impl AppState {
    fn tools(&self) -> AppResult<ffmpeg::Tools> {
        ffmpeg::require(&self.settings.get().ffmpeg_path)
    }
}

async fn blocking<T: Send + 'static>(task: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(task).await.map_err(|error| AppError::msg(error.to_string()))?
}

fn allow(app: &AppHandle, path: &Path) {
    if let Err(error) = app.asset_protocol_scope().allow_file(path) {
        log::warn!("cannot allow {}: {error}", path.display());
    }
}

// ----------------------------------------------------------------- settings

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> Settings {
    state.settings.get()
}

#[tauri::command]
pub fn save_settings(state: State<AppState>, settings: Settings) -> AppResult<Settings> {
    state.settings.set(settings)
}

#[tauri::command]
pub async fn tools_status(state: State<'_, AppState>) -> AppResult<ToolsStatus> {
    let configured = state.settings.get().ffmpeg_path;
    blocking(move || Ok(ffmpeg::status(&configured))).await
}

// ----------------------------------------------------------------- projects

#[tauri::command]
pub async fn list_projects(app: AppHandle, state: State<'_, AppState>) -> AppResult<Vec<ProjectSummary>> {
    let root = state.paths.projects.clone();
    let list = blocking(move || projects::list(&root)).await?;
    for project in &list {
        if let Some(thumbnail) = &project.thumbnail {
            allow(&app, Path::new(thumbnail));
        }
    }
    Ok(list)
}

#[tauri::command]
pub async fn load_project(app: AppHandle, state: State<'_, AppState>, id: String) -> AppResult<Value> {
    let root = state.paths.projects.clone();
    let document = blocking(move || projects::load(&root, &id)).await?;
    for asset in document.get("assets").and_then(Value::as_array).into_iter().flatten() {
        for key in ["path", "previewPath"] {
            if let Some(path) = asset.get(key).and_then(Value::as_str) {
                allow(&app, Path::new(path));
            }
        }
    }
    Ok(document)
}

#[tauri::command]
pub async fn save_project(state: State<'_, AppState>, id: String, document: Value) -> AppResult<()> {
    let root = state.paths.projects.clone();
    blocking(move || projects::save(&root, &id, &document)).await
}

#[tauri::command]
pub async fn delete_project(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let root = state.paths.projects.clone();
    blocking(move || projects::delete(&root, &id)).await
}

#[tauri::command]
pub async fn duplicate_project(state: State<'_, AppState>, id: String, new_id: String, name: String) -> AppResult<Value> {
    let root = state.paths.projects.clone();
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    blocking(move || projects::duplicate(&root, &id, &new_id, &name, &now)).await
}

#[tauri::command]
pub async fn save_project_thumbnail(app: AppHandle, state: State<'_, AppState>, id: String, jpeg_base64: String) -> AppResult<String> {
    let root = state.paths.projects.clone();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(jpeg_base64.trim_start_matches("data:image/jpeg;base64,"))
        .map_err(|_| AppError::msg("Invalid thumbnail"))?;
    let path = blocking(move || projects::save_thumbnail(&root, &id, &bytes)).await?;
    allow(&app, Path::new(&path));
    Ok(path)
}

#[tauri::command]
pub async fn write_project_file(path: String, document: Value) -> AppResult<()> {
    blocking(move || crate::paths::write_atomic(Path::new(&path), &serde_json::to_vec_pretty(&document)?)).await
}

#[tauri::command]
pub async fn write_text_file(path: String, contents: String) -> AppResult<()> {
    blocking(move || crate::paths::write_atomic(Path::new(&path), contents.as_bytes())).await
}

#[tauri::command]
pub async fn read_text_file(path: String) -> AppResult<String> {
    blocking(move || {
        let bytes = std::fs::read(&path)?;
        if bytes.len() > 20 * 1024 * 1024 {
            return Err(AppError::msg("The file is too large"));
        }
        Ok(String::from_utf8_lossy(&bytes).trim_start_matches('\u{feff}').to_string())
    })
    .await
}

#[tauri::command]
pub async fn read_project_file(path: String) -> AppResult<Value> {
    blocking(move || Ok(serde_json::from_slice(&std::fs::read(&path)?)?)).await
}

// -------------------------------------------------------------------- media

#[tauri::command]
pub async fn import_media(app: AppHandle, state: State<'_, AppState>, paths: Vec<String>) -> AppResult<Vec<Result<ImportedMedia, String>>> {
    let tools = state.tools()?;
    let results =
        blocking(move || Ok(paths.into_iter().map(|path| media::import(&tools, Path::new(&path)).map_err(|error| error.to_string())).collect::<Vec<_>>()))
            .await?;
    for media in results.iter().flatten() {
        allow(&app, Path::new(&media.path));
    }
    Ok(results)
}

#[tauri::command]
pub fn allow_media(app: AppHandle, paths: Vec<String>) {
    for path in paths {
        allow(&app, Path::new(&path));
    }
}

#[tauri::command]
pub fn check_paths(paths: Vec<String>) -> Vec<bool> {
    paths.iter().map(|path| Path::new(path).is_file()).collect()
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProxyProgress {
    path: String,
    progress: f64,
}

#[tauri::command]
pub async fn create_proxy(app: AppHandle, state: State<'_, AppState>, path: String, kind: MediaKind, duration: Option<f64>) -> AppResult<String> {
    let tools = state.tools()?;
    let cache = state.paths.media_cache.clone();
    let emitter = app.clone();
    let source = path.clone();
    let proxy = blocking(move || {
        media::create_proxy(&tools, &cache, Path::new(&source), &kind, duration.unwrap_or(0.0), |progress| {
            let _ = emitter.emit("proxy-progress", ProxyProgress { path: source.clone(), progress });
        })
    })
    .await?;
    allow(&app, &proxy);
    Ok(proxy.display().to_string())
}

#[tauri::command]
pub async fn media_filmstrip(app: AppHandle, state: State<'_, AppState>, path: String, duration: f64, width: u32, height: u32) -> AppResult<Filmstrip> {
    let tools = state.tools()?;
    let cache = state.paths.media_cache.clone();
    let strip = blocking(move || media::filmstrip(&tools, &cache, Path::new(&path), duration, width, height)).await?;
    allow(&app, Path::new(&strip.path));
    Ok(strip)
}

#[tauri::command]
pub async fn media_waveform(state: State<'_, AppState>, path: String) -> AppResult<tauri::ipc::Response> {
    let tools = state.tools()?;
    let cache = state.paths.media_cache.clone();
    let bytes = blocking(move || media::waveform(&tools, &cache, Path::new(&path))).await?;
    Ok(tauri::ipc::Response::new(bytes))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinSound {
    name: String,
    path: String,
    duration: f64,
}

pub const BUILTIN_SOUNDS: &[(&str, &str, &[u8], f64)] = &[
    ("whoosh-in.mp3", "Whoosh in", include_bytes!("../assets/whoosh-in.mp3"), 0.41),
    ("whoosh-out.mp3", "Whoosh out", include_bytes!("../assets/whoosh-out.mp3"), 0.41),
    ("swoosh.mp3", "Swoosh", include_bytes!("../assets/swoosh.mp3"), 0.6),
    ("rise.mp3", "Riser", include_bytes!("../assets/rise.mp3"), 1.5),
    ("pop.mp3", "Pop", include_bytes!("../assets/pop.mp3"), 0.18),
    ("click.mp3", "Click", include_bytes!("../assets/click.mp3"), 0.05),
    ("tick.mp3", "Tick", include_bytes!("../assets/tick.mp3"), 0.08),
    ("ding.mp3", "Ding", include_bytes!("../assets/ding.mp3"), 1.4),
    ("boom.mp3", "Boom", include_bytes!("../assets/boom.mp3"), 1.2),
    ("glitch.mp3", "Glitch", include_bytes!("../assets/glitch.mp3"), 0.45),
];

pub fn install_builtin_sounds(paths: &AppPaths) -> AppResult<()> {
    for (file, _, bytes, _) in BUILTIN_SOUNDS {
        let target = paths.builtin.join(file);
        if std::fs::metadata(&target).map(|m| m.len() != bytes.len() as u64).unwrap_or(true) {
            std::fs::write(&target, bytes)?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn builtin_sounds(state: State<AppState>) -> Vec<BuiltinSound> {
    BUILTIN_SOUNDS
        .iter()
        .map(|(file, name, _, duration)| BuiltinSound {
            name: name.to_string(),
            path: state.paths.builtin.join(file).display().to_string(),
            duration: *duration,
        })
        .collect()
}

#[tauri::command]
pub fn reveal_path(app: AppHandle, path: String) -> AppResult<()> {
    app.opener().reveal_item_in_dir(path).map_err(|error| AppError::msg(error.to_string()))
}

#[tauri::command]
pub fn open_path(app: AppHandle, path: String) -> AppResult<()> {
    app.opener().open_path(path, None::<&str>).map_err(|error| AppError::msg(error.to_string()))
}

// ------------------------------------------------------------------- export

#[tauri::command]
pub fn default_export_path(app: AppHandle, state: State<AppState>, name: String, format: String) -> String {
    let configured = state.settings.get().export_folder;
    let folder = if !configured.trim().is_empty() && Path::new(&configured).is_dir() {
        PathBuf::from(configured)
    } else {
        app.path().video_dir().or_else(|_| app.path().download_dir()).unwrap_or_else(|_| std::env::temp_dir())
    };
    let clean: String = name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' }).collect();
    let stem = if clean.trim().is_empty() { "video".to_string() } else { clean.trim().to_string() };
    let mut candidate = folder.join(format!("{stem}.{format}"));
    let mut index = 2;
    while candidate.exists() {
        candidate = folder.join(format!("{stem} ({index}).{format}"));
        index += 1;
    }
    candidate.display().to_string()
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportFinished {
    job_id: String,
    path: Option<String>,
    error: Option<String>,
    cancelled: bool,
}

#[tauri::command]
pub async fn start_export(app: AppHandle, state: State<'_, AppState>, job_id: String, request: ExportRequest) -> AppResult<()> {
    crate::paths::safe_id(&job_id)?;
    let tools = state.tools()?;
    let jobs = state.jobs.clone();
    let jobs_dir = state.paths.jobs.clone();
    let cancel = jobs.register(&job_id);
    std::thread::spawn(move || {
        let emitter = app.clone();
        let progress = move |event: ExportProgress| {
            let _ = emitter.emit("export-progress", event);
        };
        let result = render::export(render::Context { tools: &tools, job_id: &job_id, jobs_dir: &jobs_dir, cancel, progress: &progress }, request);
        jobs.finish(&job_id);
        let finished = match result {
            Ok(path) => ExportFinished { job_id: job_id.clone(), path: Some(path.display().to_string()), error: None, cancelled: false },
            Err(AppError::Cancelled) => ExportFinished { job_id: job_id.clone(), path: None, error: None, cancelled: true },
            Err(error) => {
                log::error!("export {job_id} failed: {error}");
                ExportFinished { job_id: job_id.clone(), path: None, error: Some(error.to_string()), cancelled: false }
            }
        };
        let _ = app.emit("export-finished", finished);
    });
    Ok(())
}

#[tauri::command]
pub fn cancel_export(state: State<AppState>, job_id: String) {
    state.jobs.cancel(&job_id);
}

// ----------------------------------------------------------------- captions

#[tauri::command]
pub fn whisper_models(state: State<AppState>) -> Vec<whisper::ModelInfo> {
    whisper::list_models(&state.paths.models)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    id: String,
    received: u64,
    total: u64,
}

#[tauri::command]
pub async fn download_whisper_model(app: AppHandle, state: State<'_, AppState>, id: String) -> AppResult<()> {
    let dir = state.paths.models.clone();
    let mut last = std::time::Instant::now();
    whisper::download_model(&dir, &id, |received, total| {
        if last.elapsed().as_millis() > 150 || received >= total {
            last = std::time::Instant::now();
            let _ = app.emit("model-download", DownloadProgress { id: id.clone(), received, total });
        }
    })
    .await?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscribeRequest {
    pub path: String,
    pub offset: f64,
    pub duration: f64,
    pub language: String,
    #[serde(default)]
    pub options: CaptionOptions,
}

#[tauri::command]
pub async fn transcribe(state: State<'_, AppState>, request: TranscribeRequest) -> AppResult<Transcript> {
    let tools = state.tools()?;
    let settings = state.settings.get();
    if settings.transcription_provider == "openrouter" {
        let client = openrouter::Client::new(&settings.openrouter_api_key, &settings.openrouter_model)?;
        let source = request.path.clone();
        let (offset, duration) = (request.offset, request.duration);
        let wav = blocking(move || ai::extract_wav(&tools, Path::new(&source), offset, duration)).await?;
        let (segments, words) = client.transcribe(&settings.openrouter_transcription_model, &wav, &request.language).await?;
        return Ok(build_transcript(segments, words, request.duration, request.options));
    }
    let model = whisper::model_path(&state.paths.models, &settings.whisper_model);
    let work = state.paths.jobs.join(format!("whisper-{}", chrono::Utc::now().timestamp_millis()));
    blocking(move || {
        let result = whisper::transcribe(&tools, &model, Path::new(&request.path), request.offset, request.duration, &request.language, &work);
        let _ = std::fs::remove_dir_all(&work);
        let (segments, words) = result?;
        Ok(build_transcript(segments, words, request.duration, request.options))
    })
    .await
}

// ----------------------------------------------------------------------- AI

fn ai_client(state: &AppState) -> AppResult<openrouter::Client> {
    let settings = state.settings.get();
    openrouter::Client::new(&settings.openrouter_api_key, &settings.openrouter_model)
}

#[tauri::command]
pub async fn ai_plan(state: State<'_, AppState>, brief: openrouter::BriefRequest) -> AppResult<openrouter::ScriptPlan> {
    let client = ai_client(&state)?;
    openrouter::plan(&client, &brief).await
}

#[tauri::command]
pub async fn system_fonts() -> AppResult<Vec<crate::fonts::FontFamily>> {
    Ok(tauri::async_runtime::spawn_blocking(crate::fonts::system_families).await.unwrap_or_default())
}

#[tauri::command]
pub async fn openrouter_models(transcription: Option<bool>) -> AppResult<Vec<openrouter::ModelSummary>> {
    openrouter::list_models(transcription.unwrap_or(false)).await
}

#[tauri::command]
pub async fn ai_rewrite(state: State<'_, AppState>, text: String, instruction: String) -> AppResult<String> {
    let client = ai_client(&state)?;
    openrouter::rewrite(&client, &text, &instruction).await
}

#[tauri::command]
pub async fn ai_match_images(
    state: State<'_, AppState>,
    transcript: String,
    cues: Value,
    duration: f64,
    images: Vec<String>,
) -> AppResult<Vec<openrouter::ImageEvent>> {
    let client = ai_client(&state)?;
    let tools = state.tools()?;
    let prepared = blocking(move || {
        Ok(images
            .iter()
            .filter_map(|path| ai::small_jpeg(&tools, Path::new(path)).ok().map(|bytes| (media::file_name(Path::new(path)), bytes)))
            .collect::<Vec<_>>())
    })
    .await?;
    if prepared.is_empty() {
        return Ok(vec![]);
    }
    openrouter::match_images(&client, &transcript, &cues, duration, &prepared).await
}
