//! Media I/O through libav (FFmpeg 8) linked into the process: probing files,
//! decoding video frames and audio samples, thumbnails and waveforms.

pub use ffmpeg_next as ffmpeg;

pub mod audio;
pub mod encode;
pub mod filter;
pub mod jobs;
pub mod overview;
pub mod probe;
pub mod video;

use std::sync::Once;

use vf_core::Error;

/// Must run before any other call; cheap to call again.
pub fn init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        if let Err(error) = ffmpeg::init() {
            log::error!("FFmpeg could not be initialised: {error}");
        }
        ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Error);
    });
}

/// Versions of the linked libraries, e.g. "libavcodec 62.11".
pub fn version() -> String {
    let v = ffmpeg::codec::version();
    format!("libavcodec {}.{}", v >> 16, (v >> 8) & 0xff)
}

pub(crate) fn av(error: ffmpeg::Error) -> Error {
    Error::msg(error.to_string())
}

/// Tightly packed 8-bit RGBA pixels.
#[derive(Clone, PartialEq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl std::fmt::Debug for RgbaImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RgbaImage({}×{})", self.width, self.height)
    }
}

impl RgbaImage {
    pub fn new(width: u32, height: u32) -> Self {
        RgbaImage { width, height, data: vec![0; width as usize * height as usize * 4] }
    }

    /// Rotates clockwise by 90, 180 or 270 degrees.
    pub fn rotated(&self, degrees: i32) -> RgbaImage {
        let (w, h) = (self.width as usize, self.height as usize);
        let turn = degrees.rem_euclid(360);
        if turn == 0 {
            return self.clone();
        }
        let (nw, nh) = if turn == 180 { (w, h) } else { (h, w) };
        let mut out = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let (nx, ny) = match turn {
                    90 => (h - 1 - y, x),
                    180 => (w - 1 - x, h - 1 - y),
                    _ => (y, w - 1 - x),
                };
                let src = (y * w + x) * 4;
                let dst = (ny * nw + nx) * 4;
                out[dst..dst + 4].copy_from_slice(&self.data[src..src + 4]);
            }
        }
        RgbaImage { width: nw as u32, height: nh as u32, data: out }
    }
}

/// Largest size with the aspect of `w`×`h` that fits in `max_w`×`max_h` (never upscaled, even).
pub fn fit_size(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let (w, h) = (w.max(1) as f64, h.max(1) as f64);
    let scale = (max_w as f64 / w).min(max_h as f64 / h).min(1.0);
    let even = |v: f64| ((v * scale / 2.0).round() as u32 * 2).max(2);
    (even(w), even(h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_moves_pixels_clockwise() {
        // 2×1: red, green.
        let image = RgbaImage { width: 2, height: 1, data: vec![255, 0, 0, 255, 0, 255, 0, 255] };
        let turned = image.rotated(90);
        assert_eq!((turned.width, turned.height), (1, 2));
        assert_eq!(&turned.data[0..4], &[255, 0, 0, 255]);
        assert_eq!(&turned.data[4..8], &[0, 255, 0, 255]);
        assert_eq!(image.rotated(180).data[0..4], [0, 255, 0, 255]);
    }

    #[test]
    fn fit_keeps_aspect_and_never_upscales() {
        assert_eq!(fit_size(1920, 1080, 960, 960), (960, 540));
        assert_eq!(fit_size(640, 360, 1920, 1920), (640, 360));
    }
}
