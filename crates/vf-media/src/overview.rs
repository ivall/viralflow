//! Timeline overviews: filmstrip frames and waveform peaks.

use std::path::Path;

use vf_core::Result;

use crate::audio::{AudioDecoder, CHANNELS, SAMPLE_RATE};
use crate::video::VideoDecoder;
use crate::RgbaImage;

/// `count` frames spread over `duration` seconds, each `height` pixels tall:
/// the keyframe before each moment, so a frame costs one decoded picture
/// however long the video's keyframe interval is.
pub fn filmstrip(path: &Path, duration: f64, count: usize, height: u32) -> Result<Vec<RgbaImage>> {
    let mut decoder = VideoDecoder::open_keyframes(path, height * 4, height)?;
    let mut frames = Vec::with_capacity(count);
    for index in 0..count {
        let time = if count <= 1 { 0.0 } else { duration * (index as f64 + 0.5) / count as f64 };
        if let Some(frame) = decoder.keyframe_at(time)? {
            frames.push(frame);
        } else if let Some(last) = frames.last().cloned() {
            frames.push(last);
        }
    }
    Ok(frames)
}

/// Peak level (0..1) of every 1/`per_second` of a second of the file's sound.
pub fn waveform(path: &Path, per_second: u32, cancelled: &dyn Fn() -> bool) -> Result<Vec<f32>> {
    let mut decoder = AudioDecoder::open(path)?;
    let window = (SAMPLE_RATE / per_second.max(1)) as usize * CHANNELS;
    let mut chunk = vec![0.0f32; window];
    let mut peaks = Vec::new();
    loop {
        if cancelled() {
            return Err(vf_core::Error::Cancelled);
        }
        let read = decoder.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        peaks.push(chunk[..read].iter().fold(0.0f32, |peak, sample| peak.max(sample.abs())).min(1.0));
        if read < window {
            break;
        }
    }
    Ok(peaks)
}
