//! Exporting a project to a video file on a background thread: the soundtrack
//! is mixed first, then every frame is rendered by the same compositor as the
//! preview (on its own GPU device) and encoded in process.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use vf_core::engine::scene::content_end;
use vf_core::model::{ExportCodec, ExportFormat, ExportQuality, Project};
use vf_core::{Error, Result};
use vf_media::audio::{CHANNELS, SAMPLE_RATE};
use vf_media::encode::{partial_path, Container, EncodeSettings, Encoder, VideoCodec};
use vf_render::{compositor, FrameRenderer, Gpu, Mode, TextEngine};

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Progress(Progress),
    Done(PathBuf),
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Progress {
    pub stage: String,
    pub fraction: f64,
    pub frame: u64,
    pub frames: u64,
    /// Seconds of video encoded per second of work.
    pub speed: f64,
    pub eta: Option<f64>,
}

pub struct Job {
    cancel: Arc<AtomicBool>,
    pub events: Receiver<Event>,
}

impl Job {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}

/// Output pixels per project pixel for the chosen resolution (short side).
pub fn export_scale(project: &Project) -> f64 {
    project.export.resolution.max(2) as f64 / project.width.min(project.height).max(1) as f64
}

/// Frame size of the export (even, as the encoders need).
pub fn output_size(project: &Project) -> (u32, u32) {
    let k = export_scale(project);
    let even = |value: f64| ((value / 2.0).round() * 2.0).max(2.0) as u32;
    (even(project.width as f64 * k), even(project.height as f64 * k))
}

pub fn estimate_megabytes(project: &Project, duration: f64) -> u64 {
    let (width, height) = output_size(project);
    let bits =
        vf_media::encode::target_bitrate(width, height, project.export.fps.max(1), video_codec(project.export.codec), quality_index(project.export.quality));
    ((bits as f64 * duration) / 8.0 / 1024.0 / 1024.0 + duration * 0.024).round().max(1.0) as u64
}

fn video_codec(codec: ExportCodec) -> VideoCodec {
    match codec {
        ExportCodec::H264 => VideoCodec::H264,
        ExportCodec::H265 => VideoCodec::H265,
        ExportCodec::Vp9 => VideoCodec::Vp9,
    }
}

fn quality_index(quality: ExportQuality) -> usize {
    match quality {
        ExportQuality::Draft => 0,
        ExportQuality::Standard => 1,
        ExportQuality::High => 2,
        ExportQuality::Ultra => 3,
    }
}

/// A free file name in the export folder (or Videos), like "My project (2).mp4".
pub fn default_path(folder: &str, name: &str, format: ExportFormat) -> PathBuf {
    let folder = if !folder.trim().is_empty() && Path::new(folder).is_dir() {
        PathBuf::from(folder)
    } else {
        dirs::video_dir().or_else(dirs::download_dir).unwrap_or_else(std::env::temp_dir)
    };
    let clean: String = name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' }).collect();
    let stem = if clean.trim().is_empty() { "video".to_string() } else { clean.trim().to_string() };
    let extension = format.as_str();
    let mut candidate = folder.join(format!("{stem}.{extension}"));
    let mut index = 2;
    while candidate.exists() {
        candidate = folder.join(format!("{stem} ({index}).{extension}"));
        index += 1;
    }
    candidate
}

pub fn start(project: Project, text: Arc<TextEngine>, output: PathBuf, hardware: bool) -> Job {
    let cancel = Arc::new(AtomicBool::new(false));
    let (sender, events) = channel();
    let flag = cancel.clone();
    std::thread::Builder::new()
        .name("export".into())
        .spawn(move || {
            let partial = partial_path(&output);
            let result = run(&project, text, &partial, &output, hardware, &sender, &flag);
            let event = match result {
                Ok(()) => Event::Done(output),
                Err(_) if flag.load(Ordering::Acquire) => Event::Cancelled,
                Err(error) => Event::Failed(error.to_string()),
            };
            if !matches!(event, Event::Done(_)) {
                let _ = std::fs::remove_file(&partial);
            }
            let _ = sender.send(event);
        })
        .expect("export thread");
    Job { cancel, events }
}

fn run(project: &Project, text: Arc<TextEngine>, partial: &Path, output: &Path, hardware: bool, events: &Sender<Event>, cancel: &AtomicBool) -> Result<()> {
    let cancelled = || cancel.load(Ordering::Acquire);
    let total = content_end(project);
    if total < 0.05 {
        return Err(Error::msg("The timeline is empty. Add at least one clip."));
    }
    let fps = project.export.fps.max(1);
    let frames = (total * fps as f64).ceil().max(1.0) as u64;
    let send = |stage: &str, fraction: f64, frame: u64, speed: f64, eta: Option<f64>| {
        let _ = events.send(Event::Progress(Progress { stage: stage.into(), fraction, frame, frames, speed, eta }));
    };

    // The soundtrack takes about a tenth of the work.
    send("Mixing the soundtrack", 0.0, 0, 0.0, None);
    let audio = vf_audio::mixdown::mixdown(project, total, &|f| send("Mixing the soundtrack", f * 0.1, 0, 0.0, None), &cancelled)?;
    if cancelled() {
        return Err(Error::msg("Export cancelled."));
    }

    let (width, height) = output_size(project);
    let settings = EncodeSettings {
        path: partial.to_path_buf(),
        width,
        height,
        fps,
        codec: video_codec(project.export.codec),
        container: match project.export.format {
            ExportFormat::Mp4 => Container::Mp4,
            ExportFormat::Mov => Container::Mov,
            ExportFormat::Webm => Container::Webm,
        },
        quality: quality_index(project.export.quality),
        hardware,
        audio: audio.is_some(),
    };
    let mut encoder = Encoder::create(&settings)?;
    log::info!("export {width}×{height} at {fps} fps with {}", encoder.encoder_name);
    let gpu = Gpu::new()?;
    let mut renderer = FrameRenderer::new(&gpu, text, Mode::Export);
    let target = compositor::render_target(&gpu, width, height);
    let k = width as f64 / project.width.max(1) as f64;

    let started = Instant::now();
    let mut last_report = Instant::now() - Duration::from_secs(1);
    let mut audio_written = 0usize;
    for frame in 0..frames {
        if cancelled() {
            return Err(Error::msg("Export cancelled."));
        }
        let time = frame as f64 / fps as f64;
        renderer.render(project, time, &target, k);
        let pixels = compositor::read(&gpu, &target);
        encoder.push_video(&pixels)?;
        // Audio up to the end of this frame keeps the file interleaved.
        if let Some(samples) = &audio {
            let until = ((((frame + 1) as f64 / fps as f64) * SAMPLE_RATE as f64).round() as usize * CHANNELS).min(samples.len());
            if until > audio_written {
                encoder.push_audio(&samples[audio_written..until])?;
                audio_written = until;
            }
        }
        if last_report.elapsed() >= Duration::from_millis(200) || frame + 1 == frames {
            last_report = Instant::now();
            let done = (frame + 1) as f64 / frames as f64;
            let elapsed = started.elapsed().as_secs_f64();
            let speed = if elapsed > 0.0 { (frame + 1) as f64 / fps as f64 / elapsed } else { 0.0 };
            let eta = (frame > fps as u64).then(|| elapsed / done * (1.0 - done));
            send("Rendering video", 0.1 + done * 0.88, frame + 1, speed, eta);
        }
    }
    if let Some(samples) = &audio {
        if audio_written < samples.len() {
            encoder.push_audio(&samples[audio_written..])?;
        }
    }
    send("Finishing the file", 0.99, frames, 0.0, None);
    encoder.finish()?;
    if output.exists() {
        std::fs::remove_file(output).map_err(|error| Error::msg(format!("{} cannot be replaced: {error}", output.display())))?;
    }
    std::fs::rename(partial, output).map_err(|error| Error::msg(format!("The video could not be saved: {error}")))?;
    Ok(())
}
