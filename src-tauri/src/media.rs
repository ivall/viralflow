use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::ffmpeg::{command, stderr_tail, Tools};
use crate::paths::fnv64;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Video,
    Image,
    Audio,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProbeInfo {
    pub duration: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub has_audio: bool,
    pub has_video: bool,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub format_name: String,
    pub still_image: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedMedia {
    pub kind: MediaKind,
    pub name: String,
    pub path: String,
    pub duration: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub has_audio: bool,
    pub size: u64,
    /// The webview cannot play this file directly; a proxy must be created first.
    pub needs_proxy: bool,
}

fn parse_rate(value: Option<&Value>) -> Option<f64> {
    let text = value?.as_str()?;
    let (num, den) = text.split_once('/')?;
    let num: f64 = num.parse().ok()?;
    let den: f64 = den.parse().ok()?;
    (den > 0.0 && num > 0.0).then(|| num / den)
}

fn stream_rotation(stream: &Value) -> i64 {
    if let Some(list) = stream.get("side_data_list").and_then(Value::as_array) {
        for entry in list {
            if let Some(rotation) = entry.get("rotation").and_then(Value::as_f64) {
                return rotation.round() as i64;
            }
        }
    }
    stream.get("tags").and_then(|tags| tags.get("rotate")).and_then(Value::as_str).and_then(|value| value.parse::<i64>().ok()).unwrap_or(0)
}

pub fn probe(tools: &Tools, path: &Path) -> AppResult<ProbeInfo> {
    let output = command(&tools.ffprobe)
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    if !output.status.success() {
        return Err(AppError::msg(format!("Cannot read {}: {}", file_name(path), stderr_tail(&output.stderr))));
    }
    let json: Value = serde_json::from_slice(&output.stdout)?;
    let streams = json.get("streams").and_then(Value::as_array).cloned().unwrap_or_default();
    let format = json.get("format").cloned().unwrap_or(Value::Null);
    let format_name = format.get("format_name").and_then(Value::as_str).unwrap_or_default().to_string();
    let mut info = ProbeInfo { format_name: format_name.clone(), ..Default::default() };
    info.duration =
        format.get("duration").and_then(Value::as_str).and_then(|value| value.parse::<f64>().ok()).filter(|value| value.is_finite() && *value > 0.0);

    for stream in &streams {
        let codec_type = stream.get("codec_type").and_then(Value::as_str).unwrap_or_default();
        let attached = stream.get("disposition").and_then(|d| d.get("attached_pic")).and_then(Value::as_i64) == Some(1);
        if codec_type == "video" && !attached && !info.has_video {
            info.has_video = true;
            info.video_codec = stream.get("codec_name").and_then(Value::as_str).map(String::from);
            let mut width = stream.get("width").and_then(Value::as_u64).map(|v| v as u32);
            let mut height = stream.get("height").and_then(Value::as_u64).map(|v| v as u32);
            if stream_rotation(stream).rem_euclid(180) == 90 {
                std::mem::swap(&mut width, &mut height);
            }
            info.width = width;
            info.height = height;
            info.fps = parse_rate(stream.get("avg_frame_rate")).or_else(|| parse_rate(stream.get("r_frame_rate")));
            if info.duration.is_none() {
                info.duration = stream.get("duration").and_then(Value::as_str).and_then(|value| value.parse().ok());
            }
            let frames = stream.get("nb_frames").and_then(Value::as_str).and_then(|value| value.parse::<u64>().ok());
            let image_codec = matches!(info.video_codec.as_deref(), Some("png" | "mjpeg" | "webp" | "bmp" | "tiff" | "jpegxl" | "jpeg2000" | "qoi"));
            info.still_image = format_name.contains("image2") || format_name.ends_with("_pipe") || (image_codec && frames.unwrap_or(1) <= 1);
        }
        if codec_type == "audio" && !info.has_audio {
            info.has_audio = true;
            info.audio_codec = stream.get("codec_name").and_then(Value::as_str).map(String::from);
        }
    }
    if info.still_image {
        info.duration = None;
        info.has_audio = false;
    }
    Ok(info)
}

pub fn file_name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string())
}

fn extension(path: &Path) -> String {
    path.extension().map(|ext| ext.to_string_lossy().to_lowercase()).unwrap_or_default()
}

fn browser_plays_video(path: &Path, info: &ProbeInfo) -> bool {
    let container = matches!(extension(path).as_str(), "mp4" | "m4v" | "mov" | "webm");
    let video = matches!(info.video_codec.as_deref(), Some("h264" | "vp8" | "vp9" | "av1"));
    let audio = !info.has_audio || matches!(info.audio_codec.as_deref(), Some("aac" | "mp3" | "opus" | "vorbis" | "flac"));
    container && video && audio
}

fn browser_plays_audio(path: &Path, info: &ProbeInfo) -> bool {
    let container = matches!(extension(path).as_str(), "mp3" | "m4a" | "aac" | "wav" | "ogg" | "oga" | "opus" | "flac" | "webm" | "mp4");
    let codec = matches!(info.audio_codec.as_deref(), Some("aac" | "mp3" | "opus" | "vorbis" | "flac" | "pcm_s16le" | "pcm_s24le" | "pcm_f32le" | "pcm_u8"));
    container && codec
}

fn browser_shows_image(path: &Path) -> bool {
    matches!(extension(path).as_str(), "png" | "jpg" | "jpeg" | "webp" | "bmp" | "avif")
}

pub fn import(tools: &Tools, path: &Path) -> AppResult<ImportedMedia> {
    let metadata = std::fs::metadata(path).map_err(|_| AppError::msg(format!("File not found: {}", path.display())))?;
    if !metadata.is_file() {
        return Err(AppError::msg(format!("Not a file: {}", path.display())));
    }
    let info = probe(tools, path)?;
    let gif = extension(path) == "gif";
    let kind = if info.has_video && info.still_image && !gif {
        MediaKind::Image
    } else if info.has_video && (info.duration.unwrap_or(0.0) > 0.0 || gif) {
        MediaKind::Video
    } else if info.has_audio {
        MediaKind::Audio
    } else {
        return Err(AppError::msg(format!("{} is not a supported video, image or audio file", file_name(path))));
    };
    let needs_proxy = match kind {
        MediaKind::Video => !browser_plays_video(path, &info),
        MediaKind::Audio => !browser_plays_audio(path, &info),
        MediaKind::Image => !browser_shows_image(path),
    };
    Ok(ImportedMedia {
        kind,
        name: file_name(path),
        path: path.display().to_string(),
        duration: info.duration,
        width: info.width,
        height: info.height,
        fps: info.fps,
        has_audio: info.has_audio,
        size: metadata.len(),
        needs_proxy,
    })
}

/// Cache folder for derived files of one source (thumbnails, waveform, proxy).
pub fn cache_dir(cache_root: &Path, source: &Path) -> AppResult<PathBuf> {
    let metadata = std::fs::metadata(source)?;
    let modified = metadata.modified().ok().and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok()).map(|duration| duration.as_millis()).unwrap_or(0);
    let key = fnv64(&[source.to_string_lossy().as_bytes(), &metadata.len().to_le_bytes(), &modified.to_le_bytes()]);
    let dir = cache_root.join(format!("{key:016x}"));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Runs ffmpeg, reporting progress (0..1) parsed from `-progress pipe:1`.
pub fn run_with_progress(tools: &Tools, args: &[String], total_seconds: f64, mut on_progress: impl FnMut(f64), cancelled: &dyn Fn() -> bool) -> AppResult<()> {
    let mut child = command(&tools.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-progress", "pipe:1", "-nostats"])
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stderr = child.stderr.take().expect("stderr");
    let stderr_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = stderr.read_to_end(&mut buffer);
        buffer
    });
    let stdout = child.stdout.take().expect("stdout");
    let reader = std::io::BufReader::new(stdout);
    use std::io::BufRead;
    for line in reader.lines() {
        if cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(AppError::Cancelled);
        }
        let Ok(line) = line else { break };
        if let Some(value) = line.strip_prefix("out_time_us=") {
            if let Ok(micros) = value.trim().parse::<f64>() {
                if total_seconds > 0.0 {
                    on_progress((micros / 1_000_000.0 / total_seconds).clamp(0.0, 1.0));
                }
            }
        }
    }
    let status = child.wait()?;
    let stderr = stderr_reader.join().unwrap_or_default();
    if !status.success() {
        return Err(AppError::msg(format!("FFmpeg failed: {}", stderr_tail(&stderr))));
    }
    Ok(())
}

pub fn create_proxy(tools: &Tools, cache_root: &Path, source: &Path, kind: &MediaKind, duration: f64, on_progress: impl FnMut(f64)) -> AppResult<PathBuf> {
    let dir = cache_dir(cache_root, source)?;
    let (name, args): (&str, Vec<String>) = match kind {
        MediaKind::Video => (
            "proxy.mp4",
            vec![
                "-i".into(),
                source.display().to_string(),
                "-map".into(),
                "0:v:0".into(),
                "-map".into(),
                "0:a:0?".into(),
                "-vf".into(),
                "scale='if(gt(iw,ih),min(1280,iw),-2)':'if(gt(iw,ih),-2,min(1280,ih))',format=yuv420p".into(),
                "-c:v".into(),
                "libx264".into(),
                "-preset".into(),
                "veryfast".into(),
                "-crf".into(),
                "23".into(),
                "-g".into(),
                "30".into(),
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                "160k".into(),
                "-movflags".into(),
                "+faststart".into(),
            ],
        ),
        MediaKind::Audio => {
            ("proxy.m4a", vec!["-i".into(), source.display().to_string(), "-vn".into(), "-c:a".into(), "aac".into(), "-b:a".into(), "192k".into()])
        }
        MediaKind::Image => ("proxy.png", vec!["-i".into(), source.display().to_string(), "-frames:v".into(), "1".into()]),
    };
    let output = dir.join(name);
    if output.is_file() && output.metadata().map(|m| m.len() > 0).unwrap_or(false) {
        return Ok(output);
    }
    let partial = dir.join(format!("partial-{name}"));
    let mut full_args = args;
    full_args.push(partial.display().to_string());
    run_with_progress(tools, &full_args, duration, on_progress, &|| false)?;
    std::fs::rename(&partial, &output)?;
    Ok(output)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Filmstrip {
    pub path: String,
    pub count: u32,
    pub interval: f64,
    pub tile_width: u32,
    pub tile_height: u32,
}

const STRIP_HEIGHT: u32 = 72;

pub fn filmstrip(tools: &Tools, cache_root: &Path, source: &Path, duration: f64, width: u32, height: u32) -> AppResult<Filmstrip> {
    let dir = cache_dir(cache_root, source)?;
    let duration = duration.max(0.1);
    let count = ((duration / 1.0).ceil() as u32).clamp(1, 120);
    let interval = duration / count as f64;
    let tile_width = (((width.max(1) as f64 / height.max(1) as f64) * STRIP_HEIGHT as f64).round() as u32).clamp(24, 256) & !1;
    let output = dir.join(format!("strip-v2-{count}.jpg"));
    let result = Filmstrip { path: output.display().to_string(), count, interval, tile_width, tile_height: STRIP_HEIGHT };
    if output.is_file() {
        return Ok(result);
    }
    let filter =
        format!("fps=1/{interval:.4},scale={tile_width}:{STRIP_HEIGHT}:force_original_aspect_ratio=increase,crop={tile_width}:{STRIP_HEIGHT},tile={count}x1");
    let partial = dir.join(format!("partial-strip-{count}.jpg"));
    // Decoding only keyframes is much faster for long files, but short clips may
    // have too few keyframes to fill every tile.
    let skip: &[&str] = if duration > 90.0 { &["-skip_frame", "nokey"] } else { &[] };
    let output_status = command(&tools.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(skip)
        .arg("-i")
        .arg(source)
        .args(["-an", "-vf", &filter, "-frames:v", "1", "-q:v", "5"])
        .arg(&partial)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()?;
    if !output_status.status.success() || !partial.is_file() {
        // Some codecs only expose few keyframes; retry decoding every frame.
        let retry = command(&tools.ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(source)
            .args(["-an", "-vf", &filter, "-frames:v", "1", "-q:v", "5"])
            .arg(&partial)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()?;
        if !retry.status.success() {
            return Err(AppError::msg(format!("Thumbnails failed: {}", stderr_tail(&retry.stderr))));
        }
    }
    std::fs::rename(&partial, &output)?;
    Ok(result)
}

/// Peak amplitude per 10 ms (100 values per second), 0..255.
pub fn waveform(tools: &Tools, cache_root: &Path, source: &Path) -> AppResult<Vec<u8>> {
    let dir = cache_dir(cache_root, source)?;
    let output = dir.join("wave-100hz.bin");
    if let Ok(bytes) = std::fs::read(&output) {
        return Ok(bytes);
    }
    let mut child = command(&tools.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(source)
        .args(["-vn", "-ac", "1", "-ar", "8000", "-f", "s16le", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdout = child.stdout.take().expect("stdout");
    let mut peaks: Vec<f32> = Vec::new();
    let mut buffer = vec![0u8; 160 * 64];
    let mut carry: Vec<u8> = Vec::new();
    let mut current = 0f32;
    let mut count = 0usize;
    loop {
        let read = stdout.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        carry.extend_from_slice(&buffer[..read]);
        let usable = carry.len() / 2 * 2;
        for chunk in carry[..usable].chunks_exact(2) {
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]) as f32 / 32768.0;
            current = current.max(sample.abs());
            count += 1;
            if count == 80 {
                peaks.push(current);
                current = 0.0;
                count = 0;
            }
        }
        carry.drain(..usable);
    }
    if count > 0 {
        peaks.push(current);
    }
    let _ = child.wait();
    let max = peaks.iter().cloned().fold(0.0001f32, f32::max);
    let bytes: Vec<u8> = peaks.iter().map(|value| ((value / max).sqrt() * 255.0).round().clamp(0.0, 255.0) as u8).collect();
    let _ = std::fs::write(&output, &bytes);
    Ok(bytes)
}
