//! The AI tools that need more than the document: rendering frames, reading
//! files, speech recognition, beats, playback and export. The editing tools
//! (`agent::edits`) run here too, each call as one undo step.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use vf_ai::transcript::{build_transcript, CaptionOptions as Grouping, TimedSegment, TimedWord};
use vf_ai::whisper;
use vf_core::captions::highlight_cues;
use vf_core::edit::{CaptionOptions, Placement};
use vf_core::engine::scene::asset_has_audio;
use vf_core::model::{BeatFocus, BeatOptions, CaptionCue, ExportFormat, HighlightMode, ItemKind, MediaAsset, MediaKind, Project};
use vf_render::compositor::{read, render_target};
use vf_render::{FrameRenderer, Gpu, Mode, TextEngine};

use super::EditorController;
use crate::agent::args::{self, Args};
use crate::agent::{bridge, describe, edits, jobs, jpeg, round, Picture, Reply, ToolOutput};
use crate::export;

/// Transcripts already made, by file, part of it, recogniser and language.
static HEARD: Mutex<Vec<(String, Heard)>> = Mutex::new(Vec::new());

#[derive(Clone)]
struct Heard {
    segments: Vec<TimedSegment>,
    words: Option<Vec<TimedWord>>,
}

/// What speech recognition needs (all owned, for its thread).
struct Speech {
    path: PathBuf,
    offset: f64,
    audio: f64,
    language: String,
    cloud: bool,
    key: String,
    cloud_model: String,
    model: PathBuf,
}

impl Speech {
    fn cache_key(&self) -> String {
        let engine = if self.cloud { format!("cloud:{}", self.cloud_model) } else { self.model.display().to_string() };
        format!("{}|{:.3}|{:.3}|{}|{}", self.path.display(), self.offset, self.audio, engine, self.language)
    }

    fn listen(&self, job: &jobs::Job) -> Result<Heard, String> {
        let key = self.cache_key();
        if let Some((_, heard)) = HEARD.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(k, _)| *k == key) {
            return Ok(heard.clone());
        }
        let (segments, words) = if self.cloud {
            job.progress(0.1, "Sending the sound to the cloud");
            let client = vf_ai::openrouter::Client::new(&self.key, "").map_err(|e| e.to_string())?;
            let wav = vf_ai::speech::speech_wav(&self.path, self.offset, self.audio).map_err(|e| e.to_string())?;
            client.transcribe(&self.cloud_model, &wav, &self.language).map_err(|e| e.to_string())?
        } else {
            let request = whisper::Request { model: &self.model, source: &self.path, offset: self.offset, duration: self.audio, language: &self.language };
            whisper::transcribe(&request, &|fraction| job.progress(fraction, "Listening"), &|| false).map_err(|e| e.to_string())?
        };
        let heard = Heard { segments, words };
        let mut cache = HEARD.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= 20 {
            cache.remove(0);
        }
        cache.push((key, heard.clone()));
        Ok(heard)
    }
}

/// A device of its own for the AI's pictures (made once), like the export's.
fn agent_gpu() -> Result<Gpu, String> {
    static GPU: Mutex<Option<Gpu>> = Mutex::new(None);
    let mut slot = GPU.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(gpu) = slot.as_ref() {
        return Ok(gpu.clone());
    }
    let gpu = Gpu::new().map_err(|e| e.to_string())?;
    *slot = Some(gpu.clone());
    Ok(gpu)
}

fn render_frames(project: Project, text: Arc<TextEngine>, times: Vec<f64>, width: u32) -> ToolOutput {
    let gpu = match agent_gpu() {
        Ok(gpu) => gpu,
        Err(error) => return ToolOutput::error(format!("The frames could not be drawn: {error}")),
    };
    let height = (((width as f64 * project.height as f64 / project.width.max(1) as f64) / 2.0).round() * 2.0).max(2.0) as u32;
    let mut renderer = FrameRenderer::new(&gpu, text, Mode::Export);
    let target = render_target(&gpu, width, height);
    let k = width as f64 / project.width.max(1) as f64;
    let mut pictures = Vec::new();
    for &time in &times {
        renderer.render(&project, time, &target, k);
        if let Some(data) = jpeg(width, height, &read(&gpu, &target), 82) {
            pictures.push(Picture { jpeg: data, caption: format!("The video at {} s", round(time)) });
        }
    }
    let times: Vec<f64> = times.iter().map(|t| round(*t)).collect();
    ToolOutput {
        value: json!({ "summary": format!("Looked at {} of the video", if times.len() == 1 { "a frame".to_string() } else { format!("{} frames", times.len()) }), "times": times, "size": format!("{width}×{height}") }),
        pictures,
        error: false,
    }
}

fn media_frames(asset: MediaAsset, times: Vec<f64>) -> ToolOutput {
    let path = Path::new(&asset.path);
    let mut pictures = Vec::new();
    let mut shown = Vec::new();
    if asset.kind == MediaKind::Image {
        match vf_media::video::load_image(path, 768, 768) {
            Ok(image) => {
                if let Some(data) = jpeg(image.width, image.height, &image.data, 82) {
                    pictures.push(Picture { jpeg: data, caption: asset.name.clone() });
                }
            }
            Err(error) => return ToolOutput::error(format!("{} could not be read: {error}", asset.name)),
        }
    } else {
        let mut decoder = match vf_media::video::VideoDecoder::open(path, 640, 640) {
            Ok(decoder) => decoder,
            Err(error) => return ToolOutput::error(format!("{} could not be read: {error}", asset.name)),
        };
        for time in times {
            if let Ok(Some(image)) = decoder.frame_at(time) {
                if let Some(data) = jpeg(image.width, image.height, &image.data, 80) {
                    pictures.push(Picture { jpeg: data, caption: format!("{} at {} s", asset.name, round(time)) });
                    shown.push(round(time));
                }
            }
        }
    }
    if pictures.is_empty() {
        return ToolOutput::error(format!("No picture could be read from {}.", asset.name));
    }
    ToolOutput { value: json!({ "summary": format!("Looked at {}", asset.name), "asset_id": asset.id, "times": shown }), pictures, error: false }
}

/// Words and sentences in timeline seconds.
fn transcript_value(heard: &Heard, item_id: &str, start: f64, speed: f64, audio: f64, language: &str) -> Value {
    let transcript = build_transcript(heard.segments.clone(), heard.words.clone(), audio, Grouping { max_words: 8 });
    let at = |t: f64| (((start + t.clamp(0.0, audio) / speed) * 100.0).round()) / 100.0;
    let words: Vec<Value> = transcript.words.iter().map(|w| json!([at(w.start), at(w.end), w.word])).collect();
    let segments: Vec<Value> = heard.segments.iter().map(|s| json!([at(s.start), at(s.end), s.text])).collect();
    json!({
        "summary": format!("Heard {} words", words.len()),
        "item_id": item_id,
        "language": language,
        "timing": if transcript.alignment == "word" { "each word" } else { "estimated from sentences" },
        "text": transcript.text,
        "segments": segments,
        "words": words,
    })
}

fn tempo(beats: &[f64]) -> Option<f64> {
    let mut gaps: Vec<f64> = beats.windows(2).map(|w| w[1] - w[0]).filter(|g| *g > 0.2 && *g < 2.0).collect();
    if gaps.len() < 4 {
        return None;
    }
    gaps.sort_by(f64::total_cmp);
    let mut bpm = 60.0 / gaps[gaps.len() / 2];
    while bpm < 70.0 {
        bpm *= 2.0;
    }
    while bpm > 180.0 {
        bpm /= 2.0;
    }
    Some(bpm.round())
}

impl EditorController {
    pub fn project_id(&self) -> Option<String> {
        self.with(|s| s.editor.project.id.clone())
    }

    /// The open project as the chat tells the model about it.
    pub fn agent_context(&self) -> Option<String> {
        self.with(|s| describe::context(&s.editor))
    }

    pub fn agent_document(&self) -> Option<Project> {
        self.with(|s| s.editor.project.clone())
    }

    /// Puts back a document (one undo step); false when another project is open.
    pub fn agent_restore(&self, project: Project) -> bool {
        if self.project_id().as_deref() != Some(project.id.as_str()) {
            return false;
        }
        self.edit(|e| e.replace_project(project));
        true
    }

    /// Runs a tool on the open project.
    pub fn agent_tool(self: &Rc<Self>, name: &str, value: &Value) -> Reply {
        if self.session.borrow().is_none() {
            return Reply::error("No project is open in ViralFlow. Call list_projects, then open_project or create_project.");
        }
        let a = match args::object(value) {
            Ok(a) => a,
            Err(error) => return Reply::error(error),
        };
        let reply = match name {
            "get_catalog" => {
                let topic = args::string(&a, "topic").ok().flatten().unwrap_or_else(|| "all".into());
                return Reply::ok(describe::catalog(&topic, self.text.families()));
            }
            "view_frames" => self.view_frames(&a),
            "view_media" => self.view_media(&a),
            "transcribe" => self.speech_tool(&a, false),
            "generate_captions" => self.speech_tool(&a, true),
            "find_beats" => self.find_beats_tool(&a),
            "import_media" => self.import_tool(&a),
            "control_preview" => self.control_preview(&a),
            "export_video" => self.export_tool(&a),
            _ => {
                let mut result = None;
                self.edit(|e| {
                    e.begin_batch();
                    result = edits::run(e, name, value);
                    e.end_batch();
                });
                return match result {
                    Some(result) => Reply::Now(ToolOutput::from_result(result)),
                    None => Reply::error(format!("There is no tool named \"{name}\".")),
                };
            }
        };
        reply.unwrap_or_else(Reply::error)
    }

    fn view_frames(&self, a: &Args) -> Result<Reply, String> {
        let (project, playhead, end) = self.with(|s| (s.editor.project.clone(), s.editor.time, s.editor.playback_end())).ok_or("No project is open.")?;
        if end <= 0.0 {
            return Err("The timeline is empty: there is nothing to look at yet.".into());
        }
        let mut times = args::numbers(a, "times")?;
        if times.is_empty() {
            times.push(playhead);
        }
        times.truncate(6);
        let last = (end - 1.0 / project.fps.max(1) as f64).max(0.0);
        let times: Vec<f64> = times.into_iter().map(|t| t.clamp(0.0, last)).collect();
        let width = ((args::number(a, "width")?.unwrap_or(432.0).clamp(160.0, 1080.0) / 2.0).round() * 2.0) as u32;
        let text = self.text.clone();
        Ok(Reply::Later(Box::new(move |_| render_frames(project, text, times, width))))
    }

    fn view_media(&self, a: &Args) -> Result<Reply, String> {
        let id = args::required_string(a, "asset_id")?;
        let asset = self
            .with(|s| s.editor.project.assets.iter().find(|asset| asset.id == id).cloned())
            .flatten()
            .ok_or_else(|| format!("There is no media with id \"{id}\". Call list_media."))?;
        if asset.kind == MediaKind::Audio {
            return Err(format!("{} is sound only; transcribe tells what is said in it.", asset.name));
        }
        if !Path::new(&asset.path).is_file() {
            return Err(format!("The file of {} is missing ({}).", asset.name, asset.path));
        }
        let length = asset.duration.unwrap_or(0.0);
        let mut times = args::numbers(a, "times")?;
        if times.is_empty() {
            times = if length > 0.0 { vec![length * 0.1, length * 0.5, length * 0.9] } else { vec![0.0] };
        }
        times.truncate(6);
        let times = times.into_iter().map(|t| t.clamp(0.0, (length - 0.05).max(0.0))).collect();
        Ok(Reply::Later(Box::new(move |_| media_frames(asset, times))))
    }

    /// `transcribe`, or with `captions` `generate_captions`.
    fn speech_tool(&self, a: &Args, captions: bool) -> Result<Reply, String> {
        let settings = self.context.settings.get();
        let requested = args::string(a, "item_id")?;
        let (item, asset) = self
            .with(|s| {
                let id = requested.clone().or_else(|| super::captions::speech_sources(&s.editor, &s.offline).first().map(|(id, ..)| id.clone()))?;
                let item = s.editor.item(&id)?.clone();
                let asset = s.editor.project.asset(item.asset_id.as_deref()).cloned();
                Some((item, asset))
            })
            .flatten()
            .ok_or_else(|| match &requested {
                Some(id) => format!("There is no clip with id \"{id}\"."),
                None => "There is no video or audio with sound on the timeline.".to_string(),
            })?;
        let asset = asset.ok_or_else(|| format!("{} has no media file.", item.id))?;
        let speaks = item.kind == ItemKind::Audio || (item.kind == ItemKind::Video && asset_has_audio(Some(&asset)));
        if !speaks {
            return Err(format!("{} has no sound to transcribe.", item.id));
        }
        if !Path::new(&asset.path).is_file() {
            return Err(format!("The file of {} is missing ({}).", asset.name, asset.path));
        }
        let cloud = settings.transcription_provider == "openrouter";
        let key = if cloud { self.context.keys.key() } else { String::new() };
        if cloud && key.is_empty() {
            return Err("Cloud speech recognition is chosen in Settings but there is no OpenRouter key.".into());
        }
        if !cloud && !whisper::is_installed(&self.context.paths.models, &settings.whisper_model) {
            return Err(format!(
                "The speech model \"{}\" is not downloaded yet. Ask the user to download it in the Captions panel (or Settings → Captions); it takes a minute.",
                settings.whisper_model
            ));
        }
        let language = args::string(a, "language")?.filter(|l| !l.trim().is_empty()).unwrap_or_else(|| {
            if settings.caption_language.is_empty() {
                "auto".into()
            } else {
                settings.caption_language.clone()
            }
        });
        let speed = item.speed();
        let speech = Speech {
            path: PathBuf::from(&asset.path),
            offset: item.source_in(),
            audio: item.duration * speed,
            language: language.clone(),
            cloud,
            key,
            cloud_model: settings.openrouter_transcription_model.clone(),
            model: whisper::model_path(&self.context.paths.models, &settings.whisper_model),
        };
        let look = if captions {
            let words = args::number(a, "words")?.unwrap_or(3.0).round().clamp(1.0, 8.0) as usize;
            let style = edits::caption_look(args::string(a, "preset")?.as_deref())?;
            let y = args::number(a, "y")?.unwrap_or(72.0).clamp(0.0, 100.0);
            Some((words, style, y))
        } else {
            None
        };
        let job = jobs::start(if captions { "captions" } else { "transcribe" }, "Listening");
        let worker = job.clone();
        let (item_id, start) = (item.id.clone(), item.start);
        std::thread::Builder::new()
            .name("ai-speech".into())
            .spawn(move || {
                let heard = match speech.listen(&worker) {
                    Ok(heard) => heard,
                    Err(error) => return worker.finish(Err(format!("Speech recognition failed: {error}"))),
                };
                let Some((words, style, y)) = look else {
                    return worker.finish(Ok(transcript_value(&heard, &item_id, start, speed, speech.audio, &speech.language)));
                };
                let transcript = build_transcript(heard.segments.clone(), heard.words.clone(), speech.audio, Grouping { max_words: words });
                if transcript.cues.is_empty() {
                    return worker.finish(Err("No speech was found in this clip.".into()));
                }
                let cues: Vec<CaptionCue> =
                    if style.highlight_mode != HighlightMode::None { highlight_cues(&transcript.cues, &transcript.words) } else { transcript.cues };
                let options = CaptionOptions { source_id: item_id.clone(), offset: start, speed, style, y };
                let added = bridge::on_ui(move |host| host.editor().agent_add_captions(&cues, options));
                worker.finish(match added {
                    Some(count) => Ok(json!({
                        "summary": format!("Added {count} captions"),
                        "group": format!("captions-{item_id}"),
                        "clips": count,
                        "timing": if transcript.alignment == "word" { "each word" } else { "estimated from sentences" },
                    })),
                    None => Err("ViralFlow closed before the captions were added.".into()),
                });
            })
            .map_err(|e| e.to_string())?;
        Ok(Reply::Later(Box::new(move |budget| job.output(budget))))
    }

    pub(crate) fn agent_add_captions(&self, cues: &[CaptionCue], options: CaptionOptions) -> usize {
        let mut count = 0;
        self.edit(|e| count = e.apply_captions(cues, options));
        count
    }

    fn find_beats_tool(&self, a: &Args) -> Result<Reply, String> {
        if let Some(job) = jobs::running("beats") {
            return Ok(Reply::Later(Box::new(move |budget| job.output(budget))));
        }
        let project = self.with(|s| s.editor.project.clone()).ok_or("No project is open.")?;
        let mut options = project.beat_options;
        if let Some(focus) = args::choice(a, "focus", BeatFocus::parse, &["bass", "all"])? {
            options.focus = focus;
        }
        if let Some(amount) = args::number(a, "amount")? {
            options.amount = amount.clamp(0.0, 1.0);
        }
        if !project.items().any(|i| i.kind.has_sound() && !i.muted) {
            return Err("There is no sound on the timeline to find beats in; add music first.".into());
        }
        let job = jobs::start("beats", "Listening to the music");
        let worker = job.clone();
        std::thread::Builder::new()
            .name("ai-beats".into())
            .spawn(move || {
                let analysis = match vf_audio::beats::analyse(&project, &|f| worker.progress(f, "Listening to the music"), &|| false) {
                    Ok(analysis) => analysis,
                    Err(error) => return worker.finish(Err(error.to_string())),
                };
                let beats = analysis.beats(&options);
                if beats.is_empty() {
                    return worker.finish(Err("No beats were found in the sound on the timeline.".into()));
                }
                let bpm = tempo(&beats);
                let list: Vec<f64> = beats.iter().take(300).map(|b| (b * 100.0).round() / 100.0).collect();
                let count = beats.len();
                let stored = bridge::on_ui(move |host| host.editor().agent_store_beats(Arc::new(analysis), beats, options));
                worker.finish(match stored {
                    Some(()) => Ok(json!({
                        "summary": match bpm { Some(bpm) => format!("Found {count} beats (about {bpm} BPM)"), None => format!("Found {count} beats") },
                        "count": count,
                        "bpm": bpm,
                        "beats": list,
                    })),
                    None => Err("ViralFlow closed before the beats were stored.".into()),
                });
            })
            .map_err(|e| e.to_string())?;
        Ok(Reply::Later(Box::new(move |budget| job.output(budget))))
    }

    pub(crate) fn agent_store_beats(&self, analysis: Arc<vf_audio::beats::Analysis>, beats: Vec<f64>, options: BeatOptions) {
        self.with(|s| s.beat_analysis = Some(analysis));
        self.edit(|e| e.set_beats(Some(beats), options, true));
    }

    fn import_tool(&self, a: &Args) -> Result<Reply, String> {
        let paths: Vec<PathBuf> = args::strings(a, "paths")?.into_iter().map(|p| PathBuf::from(p.trim())).collect();
        if paths.is_empty() {
            return Err("`paths` is empty.".into());
        }
        for path in &paths {
            if !path.is_absolute() {
                return Err(format!("{} is not an absolute path.", path.display()));
            }
            if !path.is_file() {
                return Err(format!("{} does not exist (find_media_files lists what is there).", path.display()));
            }
        }
        let add = args::boolean(a, "add_to_timeline")?.unwrap_or(false);
        let start = args::number(a, "start")?;
        let known = self.with(|s| s.editor.project.assets.clone()).unwrap_or_default();
        // Reading the files takes a moment each: off the UI thread.
        Ok(Reply::Later(Box::new(move |_| {
            let imported = crate::media::import(&paths, &known);
            let failures = imported.failures.clone();
            let assets = imported.assets;
            match bridge::on_ui(move |host| host.editor().agent_register_media(assets, add, start)) {
                Some(mut value) => {
                    if !failures.is_empty() {
                        value["failed"] = json!(failures);
                    }
                    ToolOutput::ok(value)
                }
                None => ToolOutput::error("ViralFlow closed before the files were added."),
            }
        })))
    }

    pub(crate) fn agent_register_media(&self, assets: Vec<MediaAsset>, add: bool, start: Option<f64>) -> Value {
        let mut created = Vec::new();
        self.edit(|e| {
            e.begin_batch();
            e.register_assets(assets.clone());
            if add && !assets.is_empty() {
                let at = start.unwrap_or_else(|| e.playback_end());
                created = e.add_assets_sequence(&assets, Placement { start: Some(at), ..Default::default() });
            }
            e.end_batch();
        });
        let media: Vec<Value> = self
            .with(|s| {
                let listed = describe::media(&s.editor.project);
                listed["media"].as_array().cloned().unwrap_or_default().into_iter().filter(|m| assets.iter().any(|a| m["id"] == a.id.as_str())).collect()
            })
            .unwrap_or_default();
        json!({ "summary": format!("Imported {} files", assets.len()), "media": media, "added_clips": created })
    }

    fn control_preview(&self, a: &Args) -> Result<Reply, String> {
        if let Some(ids) = a.get("select").map(|_| args::strings(a, "select")).transpose()? {
            self.edit(|e| e.select_many(&ids));
        }
        if let Some(time) = args::number(a, "time")? {
            self.seek(time.max(0.0));
        }
        if let Some(play) = args::boolean(a, "play")? {
            if self.with(|s| s.playing()).unwrap_or(false) != play {
                self.toggle_play();
            }
        }
        let (time, playing, selection) = self.with(|s| (s.editor.time, s.playing(), s.editor.selection.clone())).ok_or("No project is open.")?;
        Ok(Reply::ok(
            json!({ "summary": format!("Playhead at {} s{}", round(time), if playing { ", playing" } else { "" }), "playhead": round(time), "playing": playing, "selection": selection }),
        ))
    }

    fn export_tool(self: &Rc<Self>, a: &Args) -> Result<Reply, String> {
        if self.export_job.borrow().is_some() {
            return Err("An export is already running; job_status follows it, or wait for it to finish.".into());
        }
        let mut path = args::string(a, "path")?.filter(|p| !p.trim().is_empty()).map(|p| PathBuf::from(p.trim()));
        let mut settings = Args::new();
        for key in ["format", "codec", "quality", "resolution", "fps", "loudness", "hardware"] {
            if let Some(value) = a.get(key) {
                settings.insert(key.into(), value.clone());
            }
        }
        // A file name with a known extension picks the format.
        if let Some(extension) = path.as_ref().and_then(|p| p.extension()).map(|e| e.to_string_lossy().to_lowercase()) {
            if ExportFormat::parse(&extension).is_some() && !settings.contains_key("format") {
                settings.insert("format".into(), json!(extension));
            }
        }
        if !settings.is_empty() {
            let mut result = None;
            self.edit(|e| {
                e.begin_batch();
                result = edits::run(e, "set_project", &Value::Object(settings));
                e.end_batch();
            });
            if let Some(Err(error)) = result {
                return Err(error);
            }
        }
        let (name, format, end) =
            self.with(|s| (s.editor.project.name.clone(), s.editor.project.export.format, s.editor.playback_end())).ok_or("No project is open.")?;
        if end < 0.05 {
            return Err("The timeline is empty; there is nothing to export.".into());
        }
        let output = match path.take() {
            Some(mut path) => {
                if !path.is_absolute() {
                    return Err(format!("{} is not an absolute path.", path.display()));
                }
                if !path.parent().is_some_and(Path::is_dir) {
                    return Err(format!("The folder of {} does not exist.", path.display()));
                }
                path.set_extension(format.as_str());
                path
            }
            None => export::default_path(&self.context.settings.get().export_folder, &name, format),
        };
        let job = jobs::start("export", "Preparing");
        *self.export_watch.borrow_mut() = Some(job.clone());
        *self.export_path.borrow_mut() = output.clone();
        self.export_start();
        if self.export_job.borrow().is_none() {
            self.export_watch.borrow_mut().take();
            job.finish(Err("The export could not start.".into()));
        } else {
            self.context.toasts.info(&format!("Exporting “{name}”"), &output.display().to_string());
        }
        Ok(Reply::Later(Box::new(move |budget| job.output(budget))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tempo_comes_from_the_usual_gap() {
        let beats: Vec<f64> = (0..20).map(|i| i as f64 * 0.5).collect();
        assert_eq!(tempo(&beats), Some(120.0));
        let slow: Vec<f64> = (0..20).map(|i| i as f64 * 1.5).collect();
        assert_eq!(tempo(&slow), Some(80.0));
        assert_eq!(tempo(&[0.0, 1.0]), None);
    }

    #[test]
    fn transcripts_are_given_in_timeline_seconds() {
        let heard = Heard {
            segments: vec![TimedSegment { text: "hello world".into(), start: 0.0, end: 2.0 }],
            words: Some(vec![TimedWord { word: "hello".into(), start: 0.0, end: 0.8 }, TimedWord { word: "world".into(), start: 1.0, end: 2.0 }]),
        };
        // A clip at 10 s playing at double speed.
        let value = transcript_value(&heard, "clip", 10.0, 2.0, 2.0, "en");
        assert_eq!(value["words"][1], json!([10.5, 11.0, "world"]));
        assert_eq!(value["timing"], "each word");
    }
}
