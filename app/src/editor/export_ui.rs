//! The export dialog: settings live in the project (`project.export`), the
//! job runs in `crate::export` and reports progress through a channel.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, VecModel};
use vf_core::model::{ExportCodec, ExportFormat, ExportQuality};
use vf_media::encode::{encoder_for, VideoCodec};

use super::EditorController;
use crate::export::{self, Event};
use crate::{format, Choice, ExportPhase, ExportState};

impl EditorController {
    pub(super) fn bind_export(self: &Rc<Self>, state: &ExportState) {
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
        on!(on_choose, |this, field, value| this.export_choose(&field, &value));
        on!(on_toggle_hardware, |this| {
            this.edit(|e| e.update_export(|x| x.hardware = !x.hardware));
            this.sync_export();
        });
        on!(on_choose_path, |this| this.export_choose_path());
        on!(on_start, |this| this.export_start());
        on!(on_cancel, |this| {
            if let Some(job) = this.export_job.borrow().as_ref() {
                job.cancel();
            }
        });
        on!(on_close, |this| {
            if this.export_job.borrow().is_none() {
                if let Some(window) = this.context.window() {
                    window.global::<ExportState>().set_open(false);
                }
            }
        });
        on!(on_reveal, |this| crate::system::reveal(std::path::Path::new(&this.export_result.borrow().clone())));
        on!(on_play, |this| crate::system::open_path(std::path::Path::new(&this.export_result.borrow().clone())));
        // Which GPU encoders work is probed once, off the UI thread.
        std::thread::Builder::new()
            .name("encoder-probe".into())
            .spawn(|| {
                encoder_for(VideoCodec::H264, true);
                encoder_for(VideoCodec::H265, true);
            })
            .ok();
    }

    pub fn open_export(&self) {
        let Some(window) = self.context.window() else { return };
        if self.export_job.borrow().is_some() {
            window.global::<ExportState>().set_open(true);
            return;
        }
        self.flush();
        let settings = self.context.settings.get();
        let path = self.with(|s| {
            if settings.hardware_encoding && !s.editor.project.export.hardware && hardware_name(s.editor.project.export.codec).is_some() {
                s.editor.update_export(|x| x.hardware = true);
            }
            export::default_path(&settings.export_folder, &s.editor.project.name, s.editor.project.export.format)
        });
        let Some(path) = path else { return };
        *self.export_path.borrow_mut() = path;
        let state = window.global::<ExportState>();
        state.set_phase(ExportPhase::Setup);
        state.set_error("".into());
        self.sync_export();
        state.set_open(true);
    }

    fn sync_export(&self) {
        let Some(window) = self.context.window() else { return };
        let state = window.global::<ExportState>();
        let path = self.export_path.borrow().display().to_string();
        self.with(|s| {
            let project = &s.editor.project;
            let x = &project.export;
            let duration = s.editor.playback_end();
            state.set_description(format!("{} · {}", project.name, format::duration(duration)).into());
            state.set_path(path.clone().into());
            state.set_format(x.format.as_str().into());
            state.set_codec(x.codec.as_str().into());
            let codecs: Vec<Choice> = if x.format == ExportFormat::Webm {
                vec![choice("vp9", "VP9")]
            } else {
                vec![choice("h264", "H.264 - plays everywhere"), choice("h265", "H.265 - smaller file")]
            };
            state.set_codecs(ModelRc::new(VecModel::from(codecs)));
            let short = project.width.min(project.height);
            let resolutions: Vec<Choice> = [480u32, 720, 1080, 1440, 2160]
                .into_iter()
                .filter(|value| *value <= short.max(1080) * 2)
                .map(|value| Choice {
                    value: value.to_string().into(),
                    label: if value == 2160 { "4K".into() } else { format!("{value}p").into() },
                    hint: if value == short { "Original".into() } else { "".into() },
                    ..Default::default()
                })
                .collect();
            state.set_resolutions(ModelRc::new(VecModel::from(resolutions)));
            state.set_resolution(x.resolution.to_string().into());
            state.set_fps(x.fps.to_string().into());
            state.set_quality(x.quality.as_str().into());
            let hardware = hardware_name(x.codec);
            state.set_hardware_available(hardware.is_some());
            state.set_hardware(x.hardware);
            state.set_hardware_label(match hardware {
                Some(name) => format!("Faster, slightly larger files ({name})").into(),
                None => "Not available on this computer".into(),
            });
            state.set_loudness(x.loudness.filter(|v| *v < 0.0).map(|v| format!("{}", v.round() as i64)).unwrap_or_else(|| "0".into()).into());
            let (width, height) = export::output_size(project);
            state.set_summary(format!("{width}×{height} · {} fps", x.fps).into());
            state.set_estimate(format!("about {} MB", export::estimate_megabytes(project, duration)).into());
        });
    }

    fn export_choose(&self, field: &str, value: &str) {
        let mut path = self.export_path.borrow().clone();
        self.edit(|e| {
            e.update_export(|x| match field {
                "format" => {
                    let format = ExportFormat::parse(value).unwrap_or_default();
                    x.format = format;
                    x.codec = if format == ExportFormat::Webm {
                        ExportCodec::Vp9
                    } else if x.codec == ExportCodec::Vp9 {
                        ExportCodec::H264
                    } else {
                        x.codec
                    };
                    path.set_extension(format.as_str());
                }
                "codec" => x.codec = ExportCodec::parse(value).unwrap_or_default(),
                "resolution" => x.resolution = value.parse().unwrap_or(x.resolution),
                "fps" => x.fps = value.parse().unwrap_or(x.fps),
                "quality" => x.quality = ExportQuality::parse(value).unwrap_or_default(),
                "loudness" => x.loudness = value.parse::<f64>().ok().filter(|v| *v < 0.0),
                _ => {}
            })
        });
        *self.export_path.borrow_mut() = path;
        self.sync_export();
    }

    fn export_choose_path(&self) {
        let current = self.export_path.borrow().clone();
        let format = self.with(|s| s.editor.project.export.format).unwrap_or_default();
        let mut dialog = rfd::FileDialog::new().set_title("Export video").add_filter(format.as_str().to_uppercase(), &[format.as_str()]);
        if let Some(folder) = current.parent() {
            dialog = dialog.set_directory(folder);
        }
        if let Some(name) = current.file_name() {
            dialog = dialog.set_file_name(name.to_string_lossy());
        }
        if let Some(mut path) = dialog.save_file() {
            if path.extension().is_none() {
                path.set_extension(format.as_str());
            }
            *self.export_path.borrow_mut() = path;
            self.sync_export();
        }
    }

    /// Where the export goes (tests; the dialog uses the file picker).
    #[cfg(test)]
    pub fn set_export_path(&self, path: PathBuf) {
        *self.export_path.borrow_mut() = path;
        self.sync_export();
    }

    /// Development aid (`VF_EXPORT=<file>`): exports the open project and quits.
    pub fn dev_export(self: &Rc<Self>, path: PathBuf) {
        self.open_export();
        *self.export_path.borrow_mut() = path;
        self.export_start();
        *self.quit_after_export.borrow_mut() = true;
    }

    fn export_start(self: &Rc<Self>) {
        let Some(window) = self.context.window() else { return };
        let state = window.global::<ExportState>();
        self.flush();
        let Some(project) = self.with(|s| s.editor.project.clone()) else { return };
        if vf_core::engine::scene::content_end(&project) < 0.05 {
            state.set_error("The timeline is empty. Add at least one clip.".into());
            state.set_phase(ExportPhase::Failed);
            return;
        }
        let hardware = project.export.hardware && hardware_name(project.export.codec).is_some();
        let path = self.export_path.borrow().clone();
        if let Some(parent) = path.parent() {
            if !parent.is_dir() {
                state.set_error(format!("The folder {} does not exist.", parent.display()).into());
                state.set_phase(ExportPhase::Failed);
                return;
            }
        }
        // Stop playback so preview decoding doesn't compete with the export.
        self.with(|s| {
            if s.playing() {
                s.audio.stop();
            }
        });
        *self.export_job.borrow_mut() = Some(export::start(project, self.text.clone(), path, hardware));
        state.set_stage("Preparing".into());
        state.set_fraction(0.0);
        state.set_frames_label("".into());
        state.set_eta_label("".into());
        state.set_phase(ExportPhase::Working);
        let weak = Rc::downgrade(self);
        self.export_timer.start(slint::TimerMode::Repeated, Duration::from_millis(100), move || {
            if let Some(this) = weak.upgrade() {
                this.export_poll();
            }
        });
    }

    fn export_poll(&self) {
        let Some(window) = self.context.window() else { return };
        let state = window.global::<ExportState>();
        let mut finished = None;
        if let Some(job) = self.export_job.borrow().as_ref() {
            while let Ok(event) = job.events.try_recv() {
                match event {
                    Event::Progress(p) => {
                        state.set_stage(p.stage.into());
                        state.set_fraction(p.fraction as f32);
                        let frames = if p.frame > 0 {
                            let speed = if p.speed > 0.0 { format!(" · {:.1}× real time", p.speed) } else { String::new() };
                            format!("Frame {} of {}{speed}", p.frame, p.frames)
                        } else {
                            String::new()
                        };
                        state.set_frames_label(frames.into());
                        state.set_eta_label(p.eta.map(|eta| format!("{} left", format::time(eta, None))).unwrap_or_default().into());
                    }
                    other => finished = Some(other),
                }
            }
        }
        let Some(event) = finished else { return };
        self.export_timer.stop();
        if *self.quit_after_export.borrow() {
            log::info!("export finished: {event:?}");
            let _ = slint::quit_event_loop();
        }
        *self.export_job.borrow_mut() = None;
        match event {
            Event::Done(path) => {
                *self.export_result.borrow_mut() = path.display().to_string();
                state.set_result(path.display().to_string().into());
                state.set_phase(ExportPhase::Done);
                if !state.get_open() {
                    let file = path.clone();
                    self.context.toasts.with_action(
                        crate::ToastTone::Success,
                        "Your video is ready",
                        &path.display().to_string(),
                        "Show in folder",
                        move || crate::system::reveal(&file),
                    );
                }
            }
            Event::Failed(error) => {
                state.set_error(error.clone().into());
                state.set_phase(ExportPhase::Failed);
                if !state.get_open() {
                    self.context.toasts.error("The export failed", &error);
                }
            }
            Event::Cancelled => state.set_phase(ExportPhase::Setup),
            Event::Progress(_) => {}
        }
    }
}

/// The GPU encoder an export with this codec would use, if any.
fn hardware_name(codec: ExportCodec) -> Option<&'static str> {
    let codec = match codec {
        ExportCodec::H264 => VideoCodec::H264,
        ExportCodec::H265 => VideoCodec::H265,
        ExportCodec::Vp9 => return None,
    };
    let name = encoder_for(codec, true);
    (!name.starts_with("lib")).then_some(name)
}

fn choice(value: &str, label: &str) -> Choice {
    Choice { value: value.into(), label: label.into(), ..Default::default() }
}
