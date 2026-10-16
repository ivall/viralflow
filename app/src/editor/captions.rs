//! The Captions panel: transcribing a clip's speech on a background thread
//! (whisper.cpp in process), placing the captions, and SRT import/export.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Model, ModelRc, VecModel};
use vf_ai::transcript::{build_transcript, CaptionOptions as Grouping};
use vf_ai::whisper;
use vf_core::captions::{caption_cues, highlight_cues, TimedWord};
use vf_core::edit::{CaptionOptions, Editor};
use vf_core::engine::math::uid;
use vf_core::engine::scene::asset_has_audio;
use vf_core::model::{CaptionCue, HighlightMode, ItemKind, TextStyle};
use vf_core::pace::Pace;
use vf_core::presets::{caption_style, text_preset, TEXT_PRESETS};
use vf_core::srt::{format_srt, parse_subtitles};

use super::EditorController;
use crate::{format, CaptionsState, Choice};

/// First guess of seconds of work per second of audio (two passes) per model.
fn default_rate(model: &str) -> f64 {
    match model {
        "tiny" => 0.1,
        "base" => 0.2,
        "small" => 0.6,
        "medium" => 1.6,
        "large-v3-turbo" => 1.2,
        "openrouter" => 0.15,
        _ => 0.5,
    }
}

/// Whisper reports once per this many seconds of audio.
const WHISPER_CHUNK: f64 = 20.0;

struct Outcome {
    cues: Vec<CaptionCue>,
    words: Vec<TimedWord>,
    /// "word", "segment" or "estimated".
    alignment: String,
}

enum Message {
    Progress(f64),
    Download(u64, u64),
    /// An empty error means the user stopped it.
    Transcribed(Result<Outcome, String>),
    Downloaded(Result<(), String>),
}

struct Job {
    cancel: Arc<AtomicBool>,
    messages: Receiver<Message>,
    pace: Option<Pace>,
    /// Where the captions go once the transcript is back.
    target: Option<(String, f64, f64)>,
}

/// What the panel shows and remembers between uses.
pub(super) struct Panel {
    source: String,
    language: String,
    words: usize,
    style: String,
    position: String,
    job: Option<Job>,
    timer: slint::Timer,
}

impl Panel {
    pub(super) fn new(language: &str) -> Self {
        Panel {
            source: String::new(),
            language: if language.is_empty() { "auto".into() } else { language.into() },
            words: 3,
            style: "caption-default".into(),
            position: "bottom".into(),
            job: None,
            timer: slint::Timer::default(),
        }
    }
}

/// Clips whose sound can be transcribed (audio first, then the longest).
fn speech_sources(editor: &Editor, offline: &std::collections::HashSet<String>) -> Vec<(String, String, f64)> {
    let project = &editor.project;
    let mut items: Vec<_> = project
        .items()
        .filter(|item| {
            let Some(asset_id) = item.asset_id.as_deref() else { return false };
            if item.hidden || offline.contains(asset_id) {
                return false;
            }
            item.kind == ItemKind::Audio || (item.kind == ItemKind::Video && asset_has_audio(project.asset(Some(asset_id))))
        })
        .collect();
    items.sort_by(|a, b| (b.kind == ItemKind::Audio).cmp(&(a.kind == ItemKind::Audio)).then(b.duration.total_cmp(&a.duration)));
    items.into_iter().map(|item| (item.id.clone(), format!("{} ({:.1}s)", item.name, item.start), item.duration)).collect()
}

impl EditorController {
    pub(super) fn bind_captions(self: &Rc<Self>, state: &CaptionsState) {
        macro_rules! on {
            ($method:ident, |$this:ident $(, $arg:ident)*| $body:expr) => {{
                let weak = Rc::downgrade(self);
                state.$method(move |$($arg),*| {
                    if let Some($this) = weak.upgrade() {
                        $body
                    }
                });
            }};
        }
        on!(on_choose, |this, field, value| {
            {
                let mut panel = this.captions.borrow_mut();
                match field.as_str() {
                    "source" => panel.source = value.to_string(),
                    "language" => panel.language = value.to_string(),
                    "words" => panel.words = value.parse().unwrap_or(3),
                    "style" => panel.style = value.to_string(),
                    "position" => panel.position = value.to_string(),
                    _ => {}
                }
            }
            this.sync_captions();
        });
        on!(on_download, |this| this.download_model());
        on!(on_generate, |this| this.generate_captions());
        on!(on_cancel, |this| {
            if let Some(job) = this.captions.borrow().job.as_ref() {
                job.cancel.store(true, Ordering::Release);
            }
        });
        on!(on_import_srt, |this| this.import_subtitles());
        on!(on_export_srt, |this| this.export_subtitles());
        let styles: Vec<Choice> =
            std::iter::once(choice("caption-default", "Bold with outline")).chain(TEXT_PRESETS.iter().map(|p| choice(p.id, p.label))).collect();
        state.set_styles(ModelRc::new(VecModel::from(styles)));
    }

    /// Refreshes the panel (sources follow the timeline).
    pub(super) fn sync_captions(&self) {
        let Some(window) = self.context.window() else { return };
        // Only the visible panel needs to be current; it refreshes when shown.
        if window.global::<crate::EditorState>().get_panel() != "captions" {
            return;
        }
        let state = window.global::<CaptionsState>();
        let settings = self.context.settings.get();
        let sources = self.with(|s| speech_sources(&s.editor, &s.offline)).unwrap_or_default();
        let primary = self.with(|s| s.editor.primary.clone()).flatten();
        let mut panel = self.captions.borrow_mut();
        if !sources.iter().any(|(id, ..)| id == &panel.source) {
            panel.source =
                primary.filter(|p| sources.iter().any(|(id, ..)| id == p)).or_else(|| sources.first().map(|(id, ..)| id.clone())).unwrap_or_default();
        }
        let choices: Vec<Choice> = sources.iter().map(|(id, label, _)| choice(id, label)).collect();
        if state.get_sources().iter().ne(choices.iter().cloned()) {
            state.set_sources(ModelRc::new(VecModel::from(choices)));
        }
        state.set_source(panel.source.clone().into());
        state.set_language(panel.language.clone().into());
        state.set_words(panel.words.to_string().into());
        state.set_style(panel.style.clone().into());
        state.set_position(panel.position.clone().into());
        let model_id = settings.whisper_model.clone();
        let cloud = settings.transcription_provider == "openrouter";
        state.set_cloud(cloud);
        state.set_available(cloud || whisper::available());
        let installed = if cloud { self.context.keys.status().has_key } else { whisper::model_path(&self.context.paths.models, &model_id).is_file() };
        state.set_model_installed(installed);
        if let Some(model) = whisper::model(&model_id) {
            let name = model.label.split(" - ").next().unwrap_or(model.label);
            state
                .set_model_note(format!("Captions run on this computer. The “{name}” speech model ({}) is downloaded once.", format::bytes(model.size)).into());
        }
        let busy = panel.job.as_ref().is_some_and(|job| job.target.is_some());
        state.set_busy(busy);
        state.set_downloading(panel.job.as_ref().is_some_and(|job| job.target.is_none()));
    }

    fn caption_style(&self) -> TextStyle {
        let panel = self.captions.borrow();
        match text_preset(&panel.style) {
            Some(preset) => TextStyle { max_width: 80.0, ..preset.text_style() },
            None => caption_style(),
        }
    }

    fn caption_y(&self) -> f64 {
        match self.captions.borrow().position.as_str() {
            "top" => 20.0,
            "middle" => 50.0,
            _ => 74.0,
        }
    }

    fn generate_captions(self: &Rc<Self>) {
        if self.captions.borrow().job.is_some() {
            return;
        }
        let settings = self.context.settings.get();
        let (source, language, words) = {
            let panel = self.captions.borrow();
            (panel.source.clone(), panel.language.clone(), panel.words)
        };
        let found = self.with(|s| {
            let item = s.editor.item(&source)?.clone();
            let asset = s.editor.project.asset(item.asset_id.as_deref())?.clone();
            Some((item, asset))
        });
        let Some(Some((item, asset))) = found else {
            self.context.toasts.error("Captions could not be created", "The clip no longer exists.");
            return;
        };
        let speed = item.speed();
        let audio = item.duration * speed;
        let model = whisper::model_path(&self.context.paths.models, &settings.whisper_model);
        let work = self.context.paths.jobs.join(uid("captions"));
        let cloud = settings.transcription_provider == "openrouter";
        let key = if cloud { self.context.keys.key() } else { String::new() };
        let cloud_model = settings.openrouter_transcription_model.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, messages) = channel();
        let flag = cancel.clone();
        let (offset, path) = (item.source_in(), PathBuf::from(&asset.path));
        std::thread::Builder::new()
            .name("captions".into())
            .spawn(move || {
                let request = whisper::Request { model: &model, source: &path, offset, duration: audio, language: &language, work_dir: &work };
                let progress = |pass: u8, fraction: f64| {
                    let _ = sender.send(Message::Progress((pass as f64 + fraction) / 2.0));
                };
                let transcribed = if cloud {
                    // One request; the bar only creeps until the answer is in.
                    vf_ai::openrouter::Client::new(&key, "").and_then(|client| {
                        let wav = vf_ai::speech::speech_wav(&path, offset, audio)?;
                        client.transcribe(&cloud_model, &wav, &language)
                    })
                } else {
                    whisper::transcribe(&request, &progress, &|| flag.load(Ordering::Acquire))
                };
                let result = transcribed.map(|(segments, timed)| {
                    let transcript = build_transcript(segments, timed, audio, Grouping { max_words: words });
                    Outcome { cues: transcript.cues, words: transcript.words, alignment: transcript.alignment }
                });
                let _ = std::fs::remove_dir_all(&work);
                let message = match result {
                    Ok(outcome) => Message::Transcribed(Ok(outcome)),
                    Err(_) if flag.load(Ordering::Acquire) => Message::Transcribed(Err(String::new())),
                    Err(error) => Message::Transcribed(Err(error.to_string())),
                };
                let _ = sender.send(message);
            })
            .ok();
        let now = clock();
        let pace = if cloud {
            Pace::new(2.0 + audio * default_rate("openrouter"), 0.95, now)
        } else {
            Pace::new(2.0 + audio * default_rate(&settings.whisper_model), (WHISPER_CHUNK / audio.max(1.0)).min(1.0) / 2.0, now)
        };
        self.captions.borrow_mut().job = Some(Job { cancel, messages, pace: Some(pace), target: Some((item.id.clone(), item.start, speed)) });
        self.start_captions_timer();
        self.sync_captions();
    }

    fn download_model(self: &Rc<Self>) {
        if self.captions.borrow().job.is_some() {
            return;
        }
        let id = self.context.settings.get().whisper_model;
        let dir = self.context.paths.models.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, messages) = channel();
        let flag = cancel.clone();
        std::thread::Builder::new()
            .name("model-download".into())
            .spawn(move || {
                let progress = |received: u64, total: u64| {
                    let _ = sender.send(Message::Download(received, total));
                };
                let result = whisper::download_model(&dir, &id, &progress, &|| flag.load(Ordering::Acquire)).map(|_| ()).map_err(|e| e.to_string());
                let _ = sender.send(Message::Downloaded(result));
            })
            .ok();
        self.captions.borrow_mut().job = Some(Job { cancel, messages, pace: None, target: None });
        self.start_captions_timer();
        self.sync_captions();
    }

    fn start_captions_timer(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.captions.borrow().timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || {
            if let Some(this) = weak.upgrade() {
                this.poll_captions();
            }
        });
    }

    fn poll_captions(&self) {
        let Some(window) = self.context.window() else { return };
        let state = window.global::<CaptionsState>();
        let mut finished = None;
        {
            let mut panel = self.captions.borrow_mut();
            let Some(job) = panel.job.as_mut() else { return };
            while let Ok(message) = job.messages.try_recv() {
                match message {
                    Message::Progress(fraction) => {
                        if let Some(pace) = job.pace.as_mut() {
                            pace.report(fraction, clock());
                        }
                    }
                    Message::Download(received, total) => state.set_download_fraction((received as f64 / total.max(1) as f64) as f32),
                    other => finished = Some(other),
                }
            }
            if let Some(pace) = job.pace.as_mut() {
                let now = clock();
                state.set_fraction(pace.value(now) as f32);
                let left = pace.remaining(now);
                state.set_remaining(if left < 3.0 { "Almost done".into() } else { format!("About {} left", format::duration(left)).into() });
            }
        }
        let Some(message) = finished else { return };
        let job = {
            let mut panel = self.captions.borrow_mut();
            panel.timer.stop();
            panel.job.take()
        };
        match message {
            Message::Downloaded(Ok(())) => self.context.toasts.success("Speech model installed", ""),
            Message::Downloaded(Err(error)) => self.context.toasts.error("Model download failed", &error),
            Message::Transcribed(Err(error)) if error.is_empty() => {}
            Message::Transcribed(Err(error)) => self.context.toasts.error("Captions could not be created", &error),
            Message::Transcribed(Ok(Outcome { cues, words, alignment })) => {
                if cues.is_empty() {
                    self.context.toasts.show("No speech was found in this clip");
                } else if let Some((source_id, offset, speed)) = job.and_then(|j| j.target) {
                    let style = self.caption_style();
                    let highlighted = style.highlight_mode != HighlightMode::None;
                    // Highlight styles light up each spoken word: one cue per word.
                    let cues = if highlighted { highlight_cues(&cues, &words) } else { cues };
                    let y = self.caption_y();
                    let mut count = 0;
                    self.edit(|e| count = e.apply_captions(&cues, CaptionOptions { source_id, offset, speed, style, y }));
                    let detail = if alignment == "word" { "Timed word by word." } else { "Timed by sentence; adjust any caption on the timeline." };
                    self.context.toasts.success(&format!("Added {count} captions"), detail);
                }
            }
            _ => {}
        }
        self.sync_captions();
    }

    fn import_subtitles(&self) {
        let Some(path) = rfd::FileDialog::new().set_title("Import subtitles").add_filter("Subtitles", &["srt", "vtt"]).pick_file() else { return };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                self.context.toasts.error("Subtitles could not be imported", &error.to_string());
                return;
            }
        };
        let cues = parse_subtitles(&text);
        if cues.is_empty() {
            self.context.toasts.error("No subtitles found in this file", "");
            return;
        }
        let (style, y) = (self.caption_style(), self.caption_y());
        let mut count = 0;
        self.edit(|e| count = e.apply_captions(&cues, CaptionOptions { source_id: uid("srt"), offset: 0.0, speed: 1.0, style, y }));
        self.context.toasts.success(&format!("Imported {count} subtitles"), "");
    }

    fn export_subtitles(&self) {
        let Some((cues, name)) = self.with(|s| (caption_cues(&s.editor.project), s.editor.project.name.clone())) else { return };
        if cues.is_empty() {
            self.context.toasts.show("There are no captions to export");
            return;
        }
        let Some(path) = rfd::FileDialog::new().set_title("Export subtitles").set_file_name(format!("{name}.srt")).add_filter("SubRip", &["srt"]).save_file()
        else {
            return;
        };
        match std::fs::write(&path, format_srt(&cues)) {
            Ok(()) => self.context.toasts.success("Subtitles exported", &path.display().to_string()),
            Err(error) => self.context.toasts.error("Subtitles could not be exported", &error.to_string()),
        }
    }
}

fn clock() -> f64 {
    use std::sync::OnceLock;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

fn choice(value: &str, label: &str) -> Choice {
    Choice { value: value.into(), label: label.into(), ..Default::default() }
}
