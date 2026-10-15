//! Local speech recognition through FFmpeg's `whisper` filter (whisper.cpp).

use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use serde::Serialize;

use crate::ai::captions::{align_tokens, TimedSegment};
use crate::error::{AppError, AppResult};
use crate::ffmpeg::{filter_path, Tools};
use crate::media::run_with_progress;

pub const MODELS: [(&str, &str, u64); 5] = [
    ("tiny", "Tiny - fastest, rough", 77_691_713),
    ("base", "Base - fast, good for clear speech", 147_951_465),
    ("small", "Small - accurate, slower", 487_601_967),
    ("medium", "Medium - very accurate, slow", 1_533_763_059),
    ("large-v3-turbo", "Large v3 Turbo - best quality", 1_624_555_275),
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
    pub size: u64,
    pub installed: bool,
}

pub fn model_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("ggml-{id}.bin"))
}

pub fn list_models(dir: &Path) -> Vec<ModelInfo> {
    MODELS
        .iter()
        .map(|(id, label, size)| ModelInfo { id: id.to_string(), label: label.to_string(), size: *size, installed: model_path(dir, id).is_file() })
        .collect()
}

pub async fn download_model(dir: &Path, id: &str, mut on_progress: impl FnMut(u64, u64)) -> AppResult<PathBuf> {
    let Some((_, _, expected)) = MODELS.iter().find(|(model, _, _)| *model == id) else {
        return Err(AppError::msg("Unknown speech model"));
    };
    let url = format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{id}.bin");
    let response = reqwest::Client::new().get(url).send().await?.error_for_status()?;
    let total = response.content_length().unwrap_or(*expected);
    let target = model_path(dir, id);
    let partial = target.with_extension("download");
    let mut file = tokio::fs::File::create(&partial).await?;
    let mut stream = response.bytes_stream();
    let mut received = 0u64;
    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        received += chunk.len() as u64;
        on_progress(received, total);
    }
    file.flush().await?;
    drop(file);
    if received < total / 2 {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err(AppError::msg("The model download was incomplete. Try again."));
    }
    tokio::fs::rename(&partial, &target).await?;
    Ok(target)
}

fn language_code(language: &str) -> String {
    let value = language.trim().to_lowercase();
    if value.is_empty() || value == "auto" {
        return "auto".into();
    }
    if value.len() == 2 {
        return value;
    }
    match value.as_str() {
        "polish" | "polski" => "pl",
        "english" => "en",
        "german" | "deutsch" => "de",
        "spanish" | "español" => "es",
        "french" | "français" => "fr",
        "italian" => "it",
        "portuguese" => "pt",
        "ukrainian" => "uk",
        "russian" => "ru",
        _ => "auto",
    }
    .into()
}

/// Progress of a transcription: pass 0 finds the sentences, pass 1 times the words.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Progress {
    pub pass: u8,
    pub progress: f64,
}

struct Pass<'a> {
    tools: &'a Tools,
    model: &'a Path,
    source: &'a Path,
    offset: f64,
    duration: f64,
    language: &'a str,
    work_dir: &'a Path,
}

fn run_pass(pass: &Pass, tokens: bool, on_progress: &mut dyn FnMut(f64)) -> AppResult<Vec<TimedSegment>> {
    let Pass { tools, model, source, offset, duration, language, work_dir } = *pass;
    let output = work_dir.join(if tokens { "tokens.json" } else { "segments.json" });
    let _ = std::fs::remove_file(&output);
    let filter = format!(
        "aresample=16000,whisper=model={}:language={}:queue=20:use_gpu=false:destination={}:format=json{}",
        filter_path(model),
        language_code(language),
        filter_path(&output),
        if tokens { ":max_len=1" } else { "" }
    );
    let duration = duration.max(0.1);
    let args: Vec<String> = vec![
        "-ss".into(),
        format!("{:.3}", offset.max(0.0)),
        "-t".into(),
        format!("{duration:.3}"),
        "-i".into(),
        source.display().to_string(),
        "-vn".into(),
        "-af".into(),
        filter,
        "-f".into(),
        "null".into(),
        "-".into(),
    ];
    // Whisper works on 20 s chunks, so progress arrives in jumps; the UI smooths it.
    run_with_progress(tools, &args, duration, &mut *on_progress, &|| false).map_err(|error| match error {
        AppError::Message(message) => AppError::msg(message.replacen("FFmpeg failed:", "Speech recognition failed:", 1)),
        other => other,
    })?;
    let text = std::fs::read_to_string(&output).unwrap_or_default();
    let mut segments = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        #[derive(serde::Deserialize)]
        struct Line {
            start: f64,
            end: f64,
            text: String,
        }
        if let Ok(entry) = serde_json::from_str::<Line>(line) {
            let text = entry.text.trim().to_string();
            if !text.is_empty() && !text.starts_with('[') {
                segments.push(TimedSegment { text, start: entry.start / 1000.0, end: entry.end / 1000.0 });
            }
        }
    }
    Ok(segments)
}

/// Returns (segments, word timings when available) relative to `offset`.
#[allow(clippy::too_many_arguments)]
pub fn transcribe(
    tools: &Tools,
    model: &Path,
    source: &Path,
    offset: f64,
    duration: f64,
    language: &str,
    work_dir: &Path,
    on_progress: &mut dyn FnMut(Progress),
) -> AppResult<(Vec<TimedSegment>, Option<Vec<crate::ai::captions::TimedWord>>)> {
    if !model.is_file() {
        return Err(AppError::msg("The speech model is not installed. Download it in Settings → Captions."));
    }
    std::fs::create_dir_all(work_dir)?;
    let pass = Pass { tools, model, source, offset, duration, language, work_dir };
    let segments = run_pass(&pass, false, &mut |value| on_progress(Progress { pass: 0, progress: value }))?;
    if segments.is_empty() {
        return Ok((segments, None));
    }
    let tokens = run_pass(&pass, true, &mut |value| on_progress(Progress { pass: 1, progress: value })).unwrap_or_default();
    let words = align_tokens(&segments, &tokens);
    Ok((segments, words))
}
