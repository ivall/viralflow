//! The auto-edit dialog on the start page: every voiceover becomes a project
//! with captions, a background and photos (matched to the speech by AI when an
//! OpenRouter key is set). The work runs on a background thread.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, VecModel};
use vf_ai::openrouter::{self, Client};
use vf_ai::transcript::{build_transcript, CaptionOptions as Grouping};
use vf_ai::whisper;
use vf_core::engine::math::uid;
use vf_core::generators::{build_auto_edit_project, AutoEditInput, ImageEvent};
use vf_core::model::{MediaAsset, MediaKind};
use vf_core::presets::ASPECT_PRESETS;
use vf_core::Result;

use crate::app::Context;
use crate::{media, projects, AutoEditState, Choice, FileRow, RowState};

enum Message {
    Status(String, RowState, String),
    Created(String),
}

/// Everything the background run needs (all owned, so it can move to the thread).
struct Job {
    voices: Vec<MediaAsset>,
    backgrounds: Vec<MediaAsset>,
    images: Vec<MediaAsset>,
    language: String,
    width: u32,
    height: u32,
    fps: u32,
    accent: String,
    cloud: bool,
    key: String,
    chat_model: String,
    cloud_model: String,
    whisper_model: PathBuf,
    jobs: PathBuf,
    projects: PathBuf,
    sounds: PathBuf,
}

pub struct AutoEdit {
    context: Rc<Context>,
    voices: RefCell<Vec<MediaAsset>>,
    backgrounds: RefCell<Vec<MediaAsset>>,
    images: RefCell<Vec<MediaAsset>>,
    statuses: RefCell<HashMap<String, (RowState, String)>>,
    language: RefCell<String>,
    aspect: RefCell<String>,
    accent: RefCell<String>,
    running: Cell<bool>,
    /// Called after a run with the project to open (when exactly one was made).
    pub on_finished: crate::app::Listener<Option<String>>,
}

impl AutoEdit {
    pub fn new(context: Rc<Context>) -> Rc<Self> {
        let settings = context.settings.get();
        let this = Rc::new(AutoEdit {
            context: context.clone(),
            voices: RefCell::default(),
            backgrounds: RefCell::default(),
            images: RefCell::default(),
            statuses: RefCell::default(),
            language: RefCell::new(if settings.caption_language.is_empty() { "auto".into() } else { settings.caption_language.clone() }),
            aspect: RefCell::new(if settings.default_aspect.is_empty() { "9:16".into() } else { settings.default_aspect.clone() }),
            accent: RefCell::new("#ffd43b".into()),
            running: Cell::new(false),
            on_finished: RefCell::default(),
        });
        let Some(window) = context.window() else { return this };
        let state = window.global::<AutoEditState>();
        macro_rules! on {
            ($method:ident, |$me:ident $(, $arg:ident)*| $body:expr) => {{
                let weak = Rc::downgrade(&this);
                state.$method(move |$($arg),*| {
                    if let Some($me) = weak.upgrade() {
                        $body
                    }
                });
            }};
        }
        on!(on_add, |me, group| me.add(&group));
        on!(on_remove, |me, group, path| {
            me.list(&group).borrow_mut().retain(|a| a.path != path.as_str());
            me.sync();
        });
        on!(on_choose, |me, field, value| {
            match field.as_str() {
                "language" => *me.language.borrow_mut() = value.to_string(),
                "aspect" => *me.aspect.borrow_mut() = value.to_string(),
                _ => {}
            }
            me.sync();
        });
        on!(on_accent_picked, |me, color| {
            *me.accent.borrow_mut() = format!("#{:02x}{:02x}{:02x}", color.red(), color.green(), color.blue());
            me.sync();
        });
        on!(on_accent_entered, |me, value| {
            let hex = value.trim().trim_start_matches('#').to_lowercase();
            if hex.len() == 6 && u32::from_str_radix(&hex, 16).is_ok() {
                *me.accent.borrow_mut() = format!("#{hex}");
            }
            me.sync();
        });
        on!(on_run, |me| me.run());
        on!(on_close, |me| {
            if !me.running.get() {
                if let Some(window) = me.context.window() {
                    window.global::<AutoEditState>().set_open(false);
                }
            }
        });
        let aspects: Vec<Choice> =
            ASPECT_PRESETS.iter().map(|p| Choice { value: p.id.into(), label: p.label.into(), hint: p.hint.into(), ..Default::default() }).collect();
        state.set_aspects(ModelRc::new(VecModel::from(aspects)));
        this
    }

    pub fn open(&self) {
        if let Some(window) = self.context.window() {
            self.statuses.borrow_mut().clear();
            self.sync();
            window.global::<AutoEditState>().set_open(true);
        }
    }

    fn list(&self, group: &str) -> &RefCell<Vec<MediaAsset>> {
        match group {
            "voice" => &self.voices,
            "background" => &self.backgrounds,
            _ => &self.images,
        }
    }

    fn add(&self, group: &str) {
        let (title, extensions, wanted): (&str, &[&str], MediaKind) = match group {
            "voice" => ("Add voiceovers", &["mp3", "wav", "m4a", "aac", "ogg", "flac", "opus"], MediaKind::Audio),
            "background" => ("Add background videos", &["mp4", "mov", "webm", "mkv", "m4v"], MediaKind::Video),
            _ => ("Add photos", &["png", "jpg", "jpeg", "webp"], MediaKind::Image),
        };
        let Some(paths) = rfd::FileDialog::new().set_title(title).add_filter("Media", extensions).pick_files() else { return };
        let imported = media::import(&paths, &[]);
        let list = self.list(group);
        let mut skipped = imported.failures.len();
        for asset in imported.assets {
            if asset.kind != wanted {
                skipped += 1;
            } else if !list.borrow().iter().any(|a| a.path == asset.path) {
                list.borrow_mut().push(asset);
            }
        }
        if skipped > 0 {
            let detail = imported.failures.first().cloned().unwrap_or_else(|| format!("Only {} files fit here.", wanted.as_str()));
            self.context.toasts.error("Some files were skipped", &detail);
        }
        self.sync();
    }

    fn sync(&self) {
        let Some(window) = self.context.window() else { return };
        let state = window.global::<AutoEditState>();
        let statuses = self.statuses.borrow();
        let rows = |list: &RefCell<Vec<MediaAsset>>| -> ModelRc<FileRow> {
            let rows: Vec<FileRow> = list
                .borrow()
                .iter()
                .map(|asset| {
                    let (status, detail) = statuses.get(&asset.path).cloned().unwrap_or((RowState::None, String::new()));
                    FileRow { name: asset.name.clone().into(), path: asset.path.clone().into(), state: status, detail: detail.into() }
                })
                .collect();
            ModelRc::new(VecModel::from(rows))
        };
        state.set_voices(rows(&self.voices));
        state.set_backgrounds(rows(&self.backgrounds));
        state.set_images(rows(&self.images));
        state.set_running(self.running.get());
        let has_key = self.context.keys.status().has_key;
        state.set_ai(has_key);
        state.set_language(self.language.borrow().clone().into());
        state.set_aspect(self.aspect.borrow().clone().into());
        let accent = self.accent.borrow().clone();
        state.set_accent(parse_color(&accent));
        state.set_accent_hex(accent.into());
        let cloud = self.context.settings.get().transcription_provider == "openrouter";
        state.set_provider_note(
            if cloud { "Captions use cloud speech recognition (OpenRouter)." } else { "Captions use the local speech model (Settings → Captions)." }.into(),
        );
    }

    fn run(self: &Rc<Self>) {
        if self.running.get() || self.voices.borrow().is_empty() || self.backgrounds.borrow().is_empty() {
            return;
        }
        let settings = self.context.settings.get();
        let preset = ASPECT_PRESETS.iter().find(|p| p.id == self.aspect.borrow().as_str()).unwrap_or(&ASPECT_PRESETS[0]);
        let cloud = settings.transcription_provider == "openrouter";
        let job = Job {
            voices: self.voices.borrow().clone(),
            backgrounds: self.backgrounds.borrow().clone(),
            images: self.images.borrow().clone(),
            language: self.language.borrow().clone(),
            width: preset.width,
            height: preset.height,
            fps: settings.default_fps.max(1),
            accent: self.accent.borrow().clone(),
            cloud,
            key: self.context.keys.key(),
            chat_model: settings.openrouter_model.clone(),
            cloud_model: settings.openrouter_transcription_model.clone(),
            whisper_model: whisper::model_path(&self.context.paths.models, &settings.whisper_model),
            jobs: self.context.paths.jobs.clone(),
            projects: self.context.paths.projects.clone(),
            sounds: self.context.paths.builtin.clone(),
        };
        {
            let mut statuses = self.statuses.borrow_mut();
            statuses.clear();
            for voice in &job.voices {
                statuses.insert(voice.path.clone(), (RowState::Waiting, String::new()));
            }
        }
        self.running.set(true);
        self.sync();
        let created: Rc<RefCell<Vec<String>>> = Rc::default();
        let (weak, weak_done, created_done) = (Rc::downgrade(self), Rc::downgrade(self), created.clone());
        crate::tasks::stream(
            "auto-edit",
            move |sender| run_job(job, &sender),
            move |message| {
                let Some(me) = weak.upgrade() else { return };
                match message {
                    Message::Status(path, status, detail) => {
                        me.statuses.borrow_mut().insert(path, (status, detail));
                    }
                    Message::Created(id) => created.borrow_mut().push(id),
                }
                me.sync();
            },
            move || {
                let Some(me) = weak_done.upgrade() else { return };
                me.running.set(false);
                me.sync();
                let created = created_done.borrow();
                if !created.is_empty() {
                    let title = if created.len() == 1 { "Video created".to_string() } else { format!("{} videos created", created.len()) };
                    me.context.toasts.success(&title, "Every part stays editable.");
                    if created.len() == 1 {
                        if let Some(window) = me.context.window() {
                            window.global::<AutoEditState>().set_open(false);
                        }
                    }
                }
                let open = (created.len() == 1).then(|| created[0].clone());
                drop(created);
                let callback = me.on_finished.borrow();
                if let Some(callback) = callback.as_ref() {
                    callback(open);
                }
            },
        );
    }
}

fn parse_color(hex: &str) -> slint::Color {
    let value = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0xffd43b);
    slint::Color::from_rgb_u8((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

fn run_job(job: Job, sender: &std::sync::mpsc::Sender<Message>) {
    let status = |path: &str, state: RowState, detail: &str| {
        let _ = sender.send(Message::Status(path.into(), state, detail.into()));
    };
    // The bundled whooshes accompany every photo.
    let sounds = media::import(&[media::builtin_sound(&job.sounds, "whoosh-in.mp3"), media::builtin_sound(&job.sounds, "whoosh-out.mp3")], &[]).assets;
    let whoosh_in = sounds.iter().find(|a| a.name.contains("in"));
    let whoosh_out = sounds.iter().find(|a| a.name.contains("out"));
    let client = (!job.key.is_empty()).then(|| Client::new(&job.key, &job.chat_model).ok()).flatten();
    let photos = if client.is_some() && !job.images.is_empty() { photo_previews(&job.images) } else { vec![] };
    for (index, voice) in job.voices.iter().enumerate() {
        status(&voice.path, RowState::Working, "Transcribing");
        let result = (|| -> Result<(String, usize)> {
            let duration = voice.duration.unwrap_or(60.0);
            let (segments, words) = if job.cloud {
                let client = Client::new(&job.key, "")?;
                let wav = vf_ai::speech::speech_wav(std::path::Path::new(&voice.path), 0.0, duration)?;
                client.transcribe(&job.cloud_model, &wav, &job.language)?
            } else {
                let work = job.jobs.join(uid("auto-edit"));
                let request = whisper::Request {
                    model: &job.whisper_model,
                    source: std::path::Path::new(&voice.path),
                    offset: 0.0,
                    duration,
                    language: &job.language,
                    work_dir: &work,
                };
                let result = whisper::transcribe(&request, &|_, _| {}, &|| false);
                let _ = std::fs::remove_dir_all(&work);
                result?
            };
            let transcript = build_transcript(segments, words, duration, Grouping { max_words: 3 });
            let mut events = Vec::new();
            if let (Some(client), false) = (&client, photos.is_empty() || transcript.text.trim().is_empty()) {
                status(&voice.path, RowState::Working, "Matching photos");
                let cues = serde_json::Value::Array(
                    transcript.cues.iter().map(|c| serde_json::json!({ "text": c.text, "start": c.start, "duration": c.duration })).collect(),
                );
                match openrouter::match_images(client, &transcript.text, &cues, duration, &photos) {
                    Ok(found) => events = found.into_iter().map(|e| ImageEvent { image_index: e.image_index, start: e.start, end: e.end }).collect(),
                    Err(error) => log::warn!("photo matching failed, spreading photos evenly: {error}"),
                }
            }
            let project = build_auto_edit_project(&AutoEditInput {
                voice,
                background: &job.backgrounds[index % job.backgrounds.len()],
                images: &job.images,
                events: &events,
                cues: &transcript.cues,
                whoosh_in,
                whoosh_out,
                accent: &job.accent,
                fps: job.fps,
                width: job.width,
                height: job.height,
            });
            projects::save(&job.projects, &project)?;
            Ok((project.id, transcript.cues.len()))
        })();
        match result {
            Ok((id, count)) => {
                let _ = sender.send(Message::Created(id));
                status(&voice.path, RowState::Done, &format!("{count} captions"));
            }
            Err(error) => status(&voice.path, RowState::Failed, &error.to_string()),
        }
    }
}

/// Small JPEGs of the photos for the AI to look at.
fn photo_previews(images: &[MediaAsset]) -> Vec<(String, Vec<u8>)> {
    images
        .iter()
        .filter_map(|image| {
            let picture = vf_media::video::load_image(std::path::Path::new(&image.path), 768, 768).ok()?;
            let mut jpeg = Vec::new();
            let rgb: Vec<u8> = picture.data.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
            jpeg_encoder::Encoder::new(&mut jpeg, 80).encode(&rgb, picture.width as u16, picture.height as u16, jpeg_encoder::ColorType::Rgb).ok()?;
            Some((image.name.clone(), jpeg))
        })
        .collect()
}
