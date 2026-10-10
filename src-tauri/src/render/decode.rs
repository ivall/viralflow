use std::io::Read;
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::Arc;

use tiny_skia::{IntSize, Pixmap};

use crate::engine::MediaLayout;
use crate::error::{AppError, AppResult};
use crate::ffmpeg::{command, stderr_tail, Tools};
use crate::render::pixels::premultiply;

/// Source crop + target size, both in pixels.
#[derive(Debug, Clone, Copy)]
pub struct Geometry {
    pub crop_x: u32,
    pub crop_y: u32,
    pub crop_w: u32,
    pub crop_h: u32,
    pub width: u32,
    pub height: u32,
}

impl Geometry {
    pub fn new(layout: &MediaLayout, target_w: f64, target_h: f64) -> Self {
        let even = |value: f64| ((value.round() as u32).max(2) + 1) & !1;
        Geometry {
            crop_x: layout.sx.round().max(0.0) as u32,
            crop_y: layout.sy.round().max(0.0) as u32,
            crop_w: (layout.sw.round() as u32).max(1),
            crop_h: (layout.sh.round() as u32).max(1),
            width: even(target_w).min(8192),
            height: even(target_h).min(8192),
        }
    }

    fn filter(&self) -> String {
        format!("crop={}:{}:{}:{},scale={}:{}:flags=bicubic,format=rgba", self.crop_w, self.crop_h, self.crop_x, self.crop_y, self.width, self.height)
    }
}

fn frame_to_pixmap(mut data: Vec<u8>, width: u32, height: u32) -> AppResult<Pixmap> {
    premultiply(&mut data);
    Pixmap::from_vec(data, IntSize::from_wh(width, height).ok_or_else(|| AppError::msg("Invalid frame size"))?)
        .ok_or_else(|| AppError::msg("Invalid frame buffer"))
}

pub fn load_image(tools: &Tools, path: &Path, geometry: Geometry) -> AppResult<Pixmap> {
    let output = command(&tools.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-vf", &geometry.filter(), "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    let expected = (geometry.width * geometry.height * 4) as usize;
    if !output.status.success() || output.stdout.len() < expected {
        return Err(AppError::msg(format!("Cannot decode image {}: {}", path.display(), stderr_tail(&output.stderr))));
    }
    let mut data = output.stdout;
    data.truncate(expected);
    frame_to_pixmap(data, geometry.width, geometry.height)
}

/// Streams frames of one clip at the export frame rate. Frame `n` shows source
/// time `source_in + n · speed / fps`, matching `mediaTimeAt` in the preview.
pub struct VideoDecoder {
    child: Child,
    frames: Receiver<Vec<u8>>,
    geometry: Geometry,
    source_in: f64,
    speed: f64,
    fps: f64,
    index: i64,
    current: Option<Arc<Pixmap>>,
    ended: bool,
}

impl VideoDecoder {
    pub fn spawn(tools: &Tools, path: &Path, geometry: Geometry, source_in: f64, source_duration: f64, speed: f64, fps: u32) -> AppResult<Self> {
        let rate = fps as f64 / speed;
        let filter = format!("fps={rate:.6}:round=near,{}", geometry.filter());
        let mut child = command(&tools.ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-ss", &format!("{:.4}", source_in.max(0.0)), "-i"])
            .arg(path)
            .args(["-t", &format!("{:.4}", source_duration + 1.0), "-an", "-sn", "-vf", &filter, "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut stdout = child.stdout.take().expect("stdout");
        let frame_size = (geometry.width * geometry.height * 4) as usize;
        let (sender, frames) = sync_channel::<Vec<u8>>(4);
        std::thread::spawn(move || loop {
            let mut buffer = vec![0u8; frame_size];
            if stdout.read_exact(&mut buffer).is_err() {
                break;
            }
            if sender.send(buffer).is_err() {
                break;
            }
        });
        Ok(VideoDecoder { child, frames, geometry, source_in, speed, fps: fps as f64, index: -1, current: None, ended: false })
    }

    /// Frame showing `media_time`. Never seeks backwards; holds the last frame at the end.
    pub fn frame_at(&mut self, media_time: f64) -> AppResult<Option<Arc<Pixmap>>> {
        let target = (((media_time - self.source_in) * self.fps / self.speed).round() as i64).max(0);
        while self.index < target && !self.ended {
            match self.frames.recv() {
                Ok(data) => {
                    self.index += 1;
                    if self.index >= target || self.current.is_none() {
                        self.current = Some(Arc::new(frame_to_pixmap(data, self.geometry.width, self.geometry.height)?));
                    }
                }
                Err(_) => self.ended = true,
            }
        }
        Ok(self.current.clone())
    }
}

impl Drop for VideoDecoder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Decodes a single frame (used for still previews and tests).
#[allow(dead_code)]
pub fn decode_frame(tools: &Tools, path: &Path, time: f64, geometry: Geometry) -> AppResult<Pixmap> {
    let output = command(&tools.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-ss", &format!("{time:.4}"), "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-vf", &geometry.filter(), "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    let expected = (geometry.width * geometry.height * 4) as usize;
    if output.stdout.len() < expected {
        return Err(AppError::msg(stderr_tail(&output.stderr)));
    }
    let mut data = output.stdout;
    data.truncate(expected);
    frame_to_pixmap(data, geometry.width, geometry.height)
}
