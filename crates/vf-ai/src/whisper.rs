//! Local speech recognition: the source's sound runs through FFmpeg's `whisper`
//! filter (whisper.cpp) inside this process, in two passes: sentences, then
//! word timings.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use vf_core::{Error, Result};
use vf_media::audio::{AudioDecoder, CHANNELS, SAMPLE_RATE};
use vf_media::filter::AudioFilter;

use crate::transcript::{align_tokens, TimedSegment, TimedWord};

pub struct Model {
    pub id: &'static str,
    pub label: &'static str,
    pub size: u64,
}

pub const MODELS: [Model; 5] = [
    Model { id: "tiny", label: "Tiny - fastest, rough", size: 77_691_713 },
    Model { id: "base", label: "Base - fast, good for clear speech", size: 147_951_465 },
    Model { id: "small", label: "Small - accurate, slower", size: 487_601_967 },
    Model { id: "medium", label: "Medium - very accurate, slow", size: 1_533_763_059 },
    Model { id: "large-v3-turbo", label: "Large v3 Turbo - best quality", size: 1_624_555_275 },
];

pub fn model(id: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.id == id)
}

pub fn model_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("ggml-{id}.bin"))
}

/// Whether the linked libavfilter was built with whisper.cpp.
pub fn available() -> bool {
    vf_media::init();
    vf_media::ffmpeg::filter::find("whisper").is_some()
}

/// Downloads a model from Hugging Face. `progress` gets (received, total).
pub fn download_model(dir: &Path, id: &str, progress: &dyn Fn(u64, u64), cancelled: &dyn Fn() -> bool) -> Result<PathBuf> {
    let model = model(id).ok_or_else(|| Error::msg("Unknown speech model."))?;
    std::fs::create_dir_all(dir)?;
    let url = format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{id}.bin");
    let response = ureq::get(&url).call().map_err(|error| Error::msg(format!("The model could not be downloaded: {error}")))?;
    let total = response.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(model.size);
    let target = model_path(dir, id);
    let partial = target.with_extension("download");
    let result = (|| -> Result<u64> {
        let mut reader = response.into_body().into_reader();
        let mut file = std::fs::File::create(&partial)?;
        let mut buffer = vec![0u8; 1 << 16];
        let mut received = 0u64;
        loop {
            if cancelled() {
                return Err(Error::msg("Download cancelled."));
            }
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            file.write_all(&buffer[..read])?;
            received += read as u64;
            progress(received, total);
        }
        file.flush()?;
        Ok(received)
    })();
    match result {
        Ok(received) if received >= total / 2 => {
            std::fs::rename(&partial, &target)?;
            Ok(target)
        }
        Ok(_) => {
            let _ = std::fs::remove_file(&partial);
            Err(Error::msg("The model download was incomplete. Try again."))
        }
        Err(error) => {
            let _ = std::fs::remove_file(&partial);
            Err(error)
        }
    }
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

/// Quotes a path as a filter option value: `'C\:/dir/file'`.
fn filter_path(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/").replace(':', "\\:");
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub struct Request<'a> {
    pub model: &'a Path,
    pub source: &'a Path,
    /// Seconds into the file where the speech starts.
    pub offset: f64,
    pub duration: f64,
    pub language: &'a str,
    /// Scratch folder for whisper's output.
    pub work_dir: &'a Path,
}

/// One pass over the audio; `tokens` asks for one entry per token (word timings).
fn run_pass(request: &Request, tokens: bool, progress: &dyn Fn(f64), cancelled: &dyn Fn() -> bool) -> Result<Vec<TimedSegment>> {
    let output = request.work_dir.join(if tokens { "tokens.json" } else { "segments.json" });
    let _ = std::fs::remove_file(&output);
    let chain = format!(
        "aresample=16000,whisper=model={}:language={}:queue=20:use_gpu=false:destination={}:format=json{}",
        filter_path(request.model),
        language_code(request.language),
        filter_path(&output),
        if tokens { ":max_len=1" } else { "" }
    );
    {
        let mut filter = AudioFilter::new(&chain).map_err(|error| Error::msg(format!("Speech recognition could not start: {error}")))?;
        let mut decoder = AudioDecoder::open(request.source)?;
        decoder.seek(request.offset.max(0.0))?;
        let total = (request.duration.max(0.1) * SAMPLE_RATE as f64) as usize;
        let mut block = vec![0.0f32; SAMPLE_RATE as usize * CHANNELS];
        let mut sink = Vec::new();
        let mut done = 0usize;
        while done < total {
            if cancelled() {
                return Err(Error::msg("Cancelled."));
            }
            let frames = (total - done).min(SAMPLE_RATE as usize);
            let slice = &mut block[..frames * CHANNELS];
            decoder.read(slice)?;
            filter.push(slice, &mut sink)?;
            sink.clear();
            done += frames;
            progress(done as f64 / total as f64);
        }
        filter.finish(&mut sink)?;
        // Dropping the graph closes whisper's output file.
    }
    let text = std::fs::read_to_string(&output).unwrap_or_default();
    let mut segments = Vec::new();
    for line in text.lines() {
        if let Some(segment) = parse_line(line) {
            // Whisper marks non-speech as [Music], [BLANK_AUDIO] and similar.
            if !segment.text.is_empty() && !segment.text.starts_with('[') {
                segments.push(segment);
            }
        }
    }
    Ok(segments)
}

/// One line of whisper's JSON output. The filter does not escape quotes in
/// the text (`"text":"The word "x" …"`), so the text is taken as everything
/// between `"text":"` and the closing `"}` instead of parsing strict JSON.
fn parse_line(line: &str) -> Option<TimedSegment> {
    let line = line.trim();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
        let text = value.get("text")?.as_str()?.trim().to_string();
        return Some(TimedSegment { text, start: value.get("start")?.as_f64()? / 1000.0, end: value.get("end")?.as_f64()? / 1000.0 });
    }
    let number = |key: &str| -> Option<f64> {
        let at = line.find(&format!("\"{key}\":"))? + key.len() + 3;
        let rest = &line[at..];
        let end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).unwrap_or(rest.len());
        rest[..end].parse().ok()
    };
    let start = line.find("\"text\":\"")? + 8;
    let end = line.rfind("\"}")?;
    let text = line.get(start..end)?.replace("\\\"", "\"").trim().to_string();
    Some(TimedSegment { text, start: number("start")? / 1000.0, end: number("end")? / 1000.0 })
}

/// Segments and (when they can be aligned) word timings, relative to `offset`.
/// `progress` gets (pass, fraction of that pass).
pub fn transcribe(request: &Request, progress: &dyn Fn(u8, f64), cancelled: &dyn Fn() -> bool) -> Result<(Vec<TimedSegment>, Option<Vec<TimedWord>>)> {
    if !request.model.is_file() {
        return Err(Error::msg("The speech model is not installed."));
    }
    if !available() {
        return Err(Error::msg("This FFmpeg build has no speech recognition (whisper)."));
    }
    std::fs::create_dir_all(request.work_dir)?;
    let segments = run_pass(request, false, &|f| progress(0, f), cancelled)?;
    if segments.is_empty() {
        return Ok((segments, None));
    }
    let tokens = run_pass(request, true, &|f| progress(1, f), cancelled).unwrap_or_default();
    let words = align_tokens(&segments, &tokens);
    Ok((segments, words))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_lines_with_unescaped_quotes() {
        let good = parse_line(r#"{"start":0,"end":5240,"text":"Hello everyone."}"#).unwrap();
        assert_eq!((good.text.as_str(), good.start, good.end), ("Hello everyone.", 0.0, 5.24));
        let broken = parse_line(r#"{"start":5240,"end":8240,"text":"The word "shute" after."}"#).unwrap();
        assert_eq!(broken.text, r#"The word "shute" after."#);
        assert_eq!((broken.start, broken.end), (5.24, 8.24));
        assert!(parse_line("").is_none());
    }

    #[test]
    fn filter_paths_are_quoted() {
        assert_eq!(filter_path(Path::new(r"C:\models\ggml-tiny.bin")), r"'C\:/models/ggml-tiny.bin'");
    }
}
