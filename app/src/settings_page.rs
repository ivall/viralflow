//! The settings page: preferences, speech models, the OpenRouter key and model.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, VecModel};
use vf_ai::openrouter::{self, ModelSummary};
use vf_ai::whisper;
use vf_core::presets::ASPECT_PRESETS;
use vf_media::encode::{encoder_for, VideoCodec};

use crate::app::Context;
use crate::secrets::Storage;
use crate::{format, Choice, ModelRow, SettingsState, SpeechModel};

pub struct SettingsPage {
    context: Rc<Context>,
    /// Model being downloaded and how far it got.
    download: RefCell<Option<(String, f32)>>,
    /// OpenRouter catalogues (text, speech), loaded on first use.
    catalogue: RefCell<[Option<Vec<ModelSummary>>; 2]>,
    picker_speech: Cell<bool>,
}

impl SettingsPage {
    pub fn new(context: Rc<Context>) -> Rc<Self> {
        let page =
            Rc::new(SettingsPage { context: context.clone(), download: RefCell::default(), catalogue: RefCell::default(), picker_speech: Cell::new(false) });
        let Some(window) = context.window() else { return page };
        let state = window.global::<SettingsState>();
        macro_rules! on {
            ($method:ident, |$me:ident $(, $arg:ident)*| $body:expr) => {{
                let weak = Rc::downgrade(&page);
                state.$method(move |$($arg),*| {
                    if let Some($me) = weak.upgrade() {
                        $body
                    }
                });
            }};
        }
        on!(on_shown, |me| me.sync());
        on!(on_toggle, |me, field| me.update(|s| match field.as_str() {
            "autosave" => s.autosave = !s.autosave,
            "snap" => s.snap = !s.snap,
            "safe-area" => s.safe_area = !s.safe_area,
            "hardware" => s.hardware_encoding = !s.hardware_encoding,
            "mcp" => s.mcp_enabled = !s.mcp_enabled,
            _ => {}
        }));
        on!(on_copy, |me, what| me.copy(&what));
        on!(on_choose, |me, field, value| {
            let value = value.to_string();
            me.update(|s| match field.as_str() {
                "default-fps" => s.default_fps = value.parse().unwrap_or(s.default_fps),
                "default-aspect" => s.default_aspect = value.clone(),
                "provider" => s.transcription_provider = value.clone(),
                "language" => s.caption_language = value.clone(),
                "whisper-model" => s.whisper_model = value.clone(),
                _ => {}
            })
        });
        on!(on_save_key, |me, key| {
            match me.context.keys.set(&key) {
                Ok(status) if status.has_key => me.context.toasts.success("API key saved", ""),
                Ok(_) => me.context.toasts.show("API key removed"),
                Err(error) => me.context.toasts.error("The key could not be saved", &error.to_string()),
            }
            me.sync();
        });
        on!(on_download, |me, id| me.download_model(&id));
        on!(on_choose_folder, |me| {
            if let Some(folder) = rfd::FileDialog::new().set_title("Default export folder").pick_folder() {
                me.update(|s| s.export_folder = folder.display().to_string());
            }
        });
        on!(on_open_logs, |me| crate::system::open_path(&me.context.paths.logs));
        on!(on_open_picker, |me, speech| me.open_picker(speech));
        on!(on_picker_changed, |me| me.show_models());
        on!(on_picker_filter, |me, field, value| {
            if let Some(window) = me.context.window() {
                let state = window.global::<SettingsState>();
                match field.as_str() {
                    "vision" => state.set_picker_vision(!state.get_picker_vision()),
                    "free" => state.set_picker_free(!state.get_picker_free()),
                    "sort" => state.set_picker_sort(value),
                    _ => {}
                }
            }
            me.show_models();
        });
        on!(on_pick, |me, id| {
            let id = id.trim().to_string();
            let speech = me.picker_speech.get();
            me.update(|s| {
                if speech {
                    s.openrouter_transcription_model = id.clone();
                } else {
                    s.openrouter_model = id.clone();
                }
            });
            if let Some(window) = me.context.window() {
                window.global::<SettingsState>().set_picker_open(false);
            }
        });
        on!(on_close_picker, |me| {
            if let Some(window) = me.context.window() {
                window.global::<SettingsState>().set_picker_open(false);
            }
        });
        let aspects: Vec<Choice> =
            ASPECT_PRESETS.iter().map(|p| Choice { value: p.id.into(), label: p.label.into(), hint: p.hint.into(), ..Default::default() }).collect();
        state.set_aspects(ModelRc::new(VecModel::from(aspects)));
        page
    }

    fn update(&self, change: impl FnOnce(&mut crate::settings::Settings)) {
        let mcp = self.context.settings.get().mcp_enabled;
        if let Err(error) = self.context.settings.update(change) {
            self.context.toasts.error("Settings could not be saved", &error.to_string());
        }
        // The MCP server follows its switch.
        match (mcp, self.context.settings.get().mcp_enabled) {
            (false, true) => {
                if let Err(error) = self.context.mcp.start() {
                    self.context.toasts.error("AI apps can't connect", &error);
                }
            }
            (true, false) => self.context.mcp.stop(),
            _ => {}
        }
        self.sync();
    }

    fn copy(&self, what: &str) {
        let setup = crate::agent::mcp::setup(self.context.mcp.details());
        let (text, title) = match what {
            "desktop" => (setup.desktop, "Config copied"),
            "claude-code" => (setup.claude_code, "Command copied"),
            _ => (setup.http, "Command copied"),
        };
        if text.is_empty() {
            return;
        }
        match crate::system::copy_text(&text) {
            Ok(()) => self.context.toasts.success(title, "Paste it into the AI app's settings."),
            Err(error) => self.context.toasts.error("Could not copy", &error),
        }
    }

    pub fn sync(&self) {
        let Some(window) = self.context.window() else { return };
        let state = window.global::<SettingsState>();
        let s = self.context.settings.get();
        state.set_autosave(s.autosave);
        state.set_snap(s.snap);
        state.set_safe_area(s.safe_area);
        state.set_default_fps(s.default_fps.to_string().into());
        state.set_default_aspect(s.default_aspect.clone().into());
        state.set_provider(s.transcription_provider.clone().into());
        state.set_language(if s.caption_language.is_empty() { "auto".into() } else { s.caption_language.clone().into() });
        state.set_whisper_model(s.whisper_model.clone().into());
        state.set_ai_model(s.openrouter_model.clone().into());
        state.set_transcription_model(s.openrouter_transcription_model.clone().into());
        state.set_export_folder(s.export_folder.clone().into());
        state.set_hardware(s.hardware_encoding);
        state.set_mcp_enabled(s.mcp_enabled);
        let details = self.context.mcp.details();
        state.set_mcp_running(details.is_some());
        state.set_mcp_status(
            match details {
                Some((url, _)) => format!("Ready for AI apps at {url} while ViralFlow runs. Their changes appear live and Ctrl+Z undoes them."),
                None if s.mcp_enabled => "The MCP server could not start; see the logs.".to_string(),
                None => String::new(),
            }
            .into(),
        );
        let key = self.context.keys.status();
        state.set_has_key(key.has_key);
        state.set_key_hint(key.hint.into());
        state.set_key_in_file(key.storage == Storage::File);
        let download = self.download.borrow().clone();
        state.set_downloading(download.is_some());
        let models: Vec<SpeechModel> = whisper::MODELS
            .iter()
            .map(|m| SpeechModel {
                id: m.id.into(),
                label: m.label.into(),
                size: format::bytes(m.size).into(),
                installed: whisper::is_installed(&self.context.paths.models, m.id),
                downloading: download.as_ref().is_some_and(|(id, _)| id == m.id),
                fraction: download.as_ref().filter(|(id, _)| id == m.id).map(|(_, f)| *f).unwrap_or(0.0),
            })
            .collect();
        state.set_speech_models(ModelRc::new(VecModel::from(models)));
        let hardware: Vec<&str> =
            [encoder_for(VideoCodec::H264, true), encoder_for(VideoCodec::H265, true)].into_iter().filter(|n| !n.starts_with("lib")).collect();
        state.set_engine(format!("FFmpeg is built in ({})", vf_media::version()).into());
        state.set_engine_detail(
            format!(
                "Speech recognition: built in (whisper.cpp) · GPU encoders: {}",
                if hardware.is_empty() { "none".to_string() } else { hardware.join(", ") }
            )
            .into(),
        );
    }

    fn download_model(self: &Rc<Self>, id: &str) {
        if self.download.borrow().is_some() {
            return;
        }
        *self.download.borrow_mut() = Some((id.to_string(), 0.0));
        self.sync();
        let (dir, model) = (self.context.paths.models.clone(), id.to_string());
        let (weak, weak_done) = (Rc::downgrade(self), Rc::downgrade(self));
        let result: Rc<RefCell<Option<Result<(), String>>>> = Rc::default();
        let result_done = result.clone();
        crate::tasks::stream(
            "model-download",
            move |sender| {
                let progress = |received: u64, total: u64| {
                    let _ = sender.send(Err(received as f32 / total.max(1) as f32));
                };
                let outcome = whisper::download_model(&dir, &model, &progress, &|| false).map(|_| ()).map_err(|e| e.to_string());
                let _ = sender.send(Ok(outcome));
            },
            move |message: std::result::Result<std::result::Result<(), String>, f32>| {
                let Some(me) = weak.upgrade() else { return };
                match message {
                    Err(fraction) => {
                        if let Some((_, f)) = me.download.borrow_mut().as_mut() {
                            *f = fraction;
                        }
                    }
                    Ok(outcome) => *result.borrow_mut() = Some(outcome),
                }
                me.sync();
            },
            move || {
                let Some(me) = weak_done.upgrade() else { return };
                let id = me.download.borrow_mut().take().map(|(id, _)| id);
                match result_done.borrow_mut().take() {
                    Some(Ok(())) => {
                        if let Some(id) = id {
                            let _ = me.context.settings.update(|s| s.whisper_model = id);
                        }
                        me.context.toasts.success("Speech model installed", "");
                    }
                    Some(Err(error)) => me.context.toasts.error("Model download failed", &error),
                    None => {}
                }
                me.sync();
            },
        );
    }

    fn open_picker(self: &Rc<Self>, speech: bool) {
        let Some(window) = self.context.window() else { return };
        let state = window.global::<SettingsState>();
        self.picker_speech.set(speech);
        state.set_picker_speech(speech);
        state.set_picker_query("".into());
        state.set_picker_error("".into());
        state.set_picker_open(true);
        let slot = speech as usize;
        if self.catalogue.borrow()[slot].is_some() {
            self.show_models();
            return;
        }
        state.set_picker_loading(true);
        let weak = Rc::downgrade(self);
        crate::tasks::background(
            "model-catalogue",
            move || openrouter::list_models(speech),
            move |result| {
                let Some(me) = weak.upgrade() else { return };
                let Some(window) = me.context.window() else { return };
                let state = window.global::<SettingsState>();
                state.set_picker_loading(false);
                match result {
                    Ok(models) => me.catalogue.borrow_mut()[slot] = Some(models),
                    Err(error) => state.set_picker_error(error.to_string().into()),
                }
                me.show_models();
            },
        );
    }

    fn show_models(&self) {
        let Some(window) = self.context.window() else { return };
        let state = window.global::<SettingsState>();
        let speech = self.picker_speech.get();
        let catalogue = self.catalogue.borrow();
        let Some(models) = catalogue[speech as usize].as_ref() else {
            state.set_picker_rows(ModelRc::default());
            return;
        };
        let query = state.get_picker_query().to_lowercase();
        let words: Vec<&str> = query.split_whitespace().collect();
        let (vision, free) = (state.get_picker_vision() && !speech, state.get_picker_free() && !speech);
        let mut list: Vec<&ModelSummary> = models
            .iter()
            .filter(|m| words.iter().all(|w| m.id.to_lowercase().contains(w) || m.name.to_lowercase().contains(w)))
            .filter(|m| !vision || m.vision)
            .filter(|m| !free || (m.prompt_price == 0.0 && m.completion_price == 0.0))
            .collect();
        match state.get_picker_sort().as_str() {
            "cheapest" if !speech => list.sort_by(|a, b| (a.prompt_price + a.completion_price).total_cmp(&(b.prompt_price + b.completion_price))),
            "name" => list.sort_by_key(|m| m.name.to_lowercase()),
            _ => {}
        }
        let price = |value: f64| {
            if value == 0.0 {
                "free".to_string()
            } else if value < 0.01 {
                format!("${value:.4}")
            } else {
                format!("${value:.2}")
            }
        };
        let rows: Vec<ModelRow> = list
            .into_iter()
            .take(300)
            .map(|m| ModelRow {
                id: m.id.clone().into(),
                name: if m.name.is_empty() { m.id.clone().into() } else { m.name.clone().into() },
                prices: if speech { "".into() } else { format!("{} / {}", price(m.prompt_price), price(m.completion_price)).into() },
                vision: m.vision,
            })
            .collect();
        state.set_picker_rows(ModelRc::new(VecModel::from(rows)));
    }
}
