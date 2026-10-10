pub mod captions;
pub mod openrouter;
pub mod whisper;

use std::path::Path;
use std::process::Stdio;

use crate::error::{AppError, AppResult};
use crate::ffmpeg::{command, stderr_tail, Tools};

/// Mono 16 kHz WAV of a source range - the input cloud transcription expects.
pub fn extract_wav(tools: &Tools, source: &Path, offset: f64, duration: f64) -> AppResult<Vec<u8>> {
    let output = command(&tools.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-ss", &format!("{:.3}", offset.max(0.0)), "-t", &format!("{:.3}", duration.max(0.1)), "-i"])
        .arg(source)
        .args(["-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le", "-f", "wav", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    if !output.status.success() || output.stdout.is_empty() {
        return Err(AppError::msg(format!("Cannot read audio: {}", stderr_tail(&output.stderr))));
    }
    if output.stdout.len() > 24 * 1024 * 1024 {
        return Err(AppError::msg("The audio clip is too long for cloud transcription. Use local captions or a shorter clip."));
    }
    Ok(output.stdout)
}

/// Small JPEG of an image for multimodal requests.
pub fn small_jpeg(tools: &Tools, source: &Path) -> AppResult<Vec<u8>> {
    let output = command(&tools.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(source)
        .args([
            "-vf",
            "scale='min(768,iw)':'min(768,ih)':force_original_aspect_ratio=decrease",
            "-frames:v",
            "1",
            "-q:v",
            "5",
            "-f",
            "image2pipe",
            "-vcodec",
            "mjpeg",
            "-",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    if !output.status.success() || output.stdout.is_empty() {
        return Err(AppError::msg(format!("Cannot read image: {}", stderr_tail(&output.stderr))));
    }
    Ok(output.stdout)
}
