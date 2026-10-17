//! Local speech recognition: whisper.cpp runs in this process (whisper-rs) on
//! the clip's sound at 16 kHz mono. One pass gives the sentences and, from the
//! token timestamps, when each word is said.
//!
//! Models come from one pinned commit of the official conversions and are
//! checked against their SHA-256 before they are used, so a torn or altered
//! download never looks installed.

use std::ffi::{c_int, c_void};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use vf_core::{Error, Result};
use whisper_rs::whisper_rs_sys::{whisper_context, whisper_state};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperSegment};

use crate::transcript::{TimedSegment, TimedWord};

pub struct Model {
    pub id: &'static str,
    pub label: &'static str,
    pub size: u64,
    /// What the finished download must hash to.
    pub sha256: &'static str,
}

pub const MODELS: [Model; 5] = [
    Model { id: "tiny", label: "Tiny - fastest, rough", size: 77_691_713, sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21" },
    Model {
        id: "base",
        label: "Base - fast, good for clear speech",
        size: 147_951_465,
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
    },
    Model { id: "small", label: "Small - accurate, slower", size: 487_601_967, sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b" },
    Model {
        id: "medium",
        label: "Medium - very accurate, slow",
        size: 1_533_763_059,
        sha256: "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
    },
    Model {
        id: "large-v3-turbo",
        label: "Large v3 Turbo - best quality",
        size: 1_624_555_275,
        sha256: "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
    },
];

/// The commit of huggingface.co/ggerganov/whisper.cpp the models are fetched
/// from, so the bytes behind a URL cannot change under the digests above.
const REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";

pub fn model(id: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.id == id)
}

pub fn model_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("ggml-{id}.bin"))
}

/// Whether the model is on disk whole. Checks the size only (hashing a
/// gigabyte on every panel refresh is too slow): files only get there through
/// a verified download, and this catches ones cut short by older versions,
/// which kept a download from half its size up.
pub fn is_installed(dir: &Path, id: &str) -> bool {
    let Some(model) = model(id) else { return false };
    std::fs::metadata(model_path(dir, id)).is_ok_and(|meta| meta.len() == model.size)
}

fn model_url(id: &str) -> String {
    format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/{REVISION}/ggml-{id}.bin")
}

/// Downloads a model and checks its digest. An interrupted download is kept
/// and resumed by the next attempt. `progress` gets (received, total).
pub fn download_model(dir: &Path, id: &str, progress: &dyn Fn(u64, u64), cancelled: &dyn Fn() -> bool) -> Result<PathBuf> {
    let model = model(id).ok_or_else(|| Error::msg("Unknown speech model."))?;
    std::fs::create_dir_all(dir)?;
    let target = model_path(dir, id);
    let partial = target.with_extension("download");
    let mut hasher = Sha256::new();
    let mut have = resume_point(&partial, model.size, &mut hasher)?;
    let mut request = ureq::get(&model_url(id));
    if have > 0 {
        request = request.header("Range", format!("bytes={have}-"));
    }
    let response = request.call().map_err(|error| {
        // A range the server refuses starts over next time.
        if have > 0 {
            let _ = std::fs::remove_file(&partial);
        }
        Error::msg(format!("The model could not be downloaded: {error}"))
    })?;
    let mut file = if have > 0 && response.status().as_u16() == 206 {
        std::fs::OpenOptions::new().append(true).open(&partial)?
    } else {
        // The server sent the whole file.
        have = 0;
        hasher = Sha256::new();
        std::fs::File::create(&partial)?
    };
    let mut reader = response.into_body().into_reader();
    let mut buffer = vec![0u8; 1 << 16];
    progress(have, model.size);
    loop {
        if cancelled() {
            file.flush()?;
            return Err(Error::msg("Download cancelled."));
        }
        let read = reader.read(&mut buffer).map_err(|error| Error::msg(format!("The model download stopped: {error}. Try again to continue it.")))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])?;
        hasher.update(&buffer[..read]);
        have += read as u64;
        progress(have.min(model.size), model.size);
    }
    file.sync_all()?;
    drop(file);
    if have < model.size {
        return Err(Error::msg("The model download was cut short. Try again to continue it."));
    }
    let digest = hex(&hasher.finalize());
    if have != model.size || digest != model.sha256 {
        let _ = std::fs::remove_file(&partial);
        log::warn!("model {id}: {have} bytes, sha256 {digest}, expected {} bytes, {}", model.size, model.sha256);
        return Err(Error::msg("The downloaded model is damaged. Try again."));
    }
    std::fs::rename(&partial, &target)?;
    Ok(target)
}

/// How many bytes of an earlier attempt can be kept, hashed into `hasher`.
fn resume_point(partial: &Path, size: u64, hasher: &mut Sha256) -> Result<u64> {
    let Ok(meta) = std::fs::metadata(partial) else { return Ok(0) };
    if meta.len() == 0 || meta.len() >= size {
        let _ = std::fs::remove_file(partial);
        return Ok(0);
    }
    let mut file = std::fs::File::open(partial)?;
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(meta.len())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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

pub struct Request<'a> {
    pub model: &'a Path,
    pub source: &'a Path,
    /// Seconds into the file where the speech starts.
    pub offset: f64,
    pub duration: f64,
    pub language: &'a str,
}

/// Share of the progress bar for reading the sound; recognition has the rest.
const READING_SHARE: f64 = 0.05;

/// Segments and word timings, relative to `offset`. `progress` gets the
/// fraction of the whole job done.
pub fn transcribe(request: &Request, progress: &dyn Fn(f64), cancelled: &dyn Fn() -> bool) -> Result<(Vec<TimedSegment>, Option<Vec<TimedWord>>)> {
    if !request.model.is_file() {
        return Err(Error::msg("The speech model is not installed."));
    }
    whisper_rs::install_logging_hooks();
    let samples = crate::speech::speech_samples(request.source, request.offset, request.duration, &|f| progress(f * READING_SHARE), cancelled)?;
    if samples.is_empty() {
        return Ok((vec![], None));
    }
    let model = request.model.to_str().ok_or_else(|| Error::msg("The speech model's path is not valid text."))?;
    let context = WhisperContext::new_with_params(model, WhisperContextParameters::default())
        .map_err(|error| Error::msg(format!("The speech model could not be loaded: {error}")))?;
    let mut state = context.create_state().map_err(|error| Error::msg(format!("Speech recognition could not start: {error}")))?;

    let language = language_code(request.language);
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 5 });
    params.set_language(Some(&language));
    params.set_n_threads(threads());
    params.set_token_timestamps(true);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_special(false);
    params.set_print_timestamps(false);
    let report = |percent: i32| progress(READING_SHARE + (1.0 - READING_SHARE) * (percent as f64 / 100.0).clamp(0.0, 1.0));
    let report: &dyn Fn(i32) = &report;
    // SAFETY: both callbacks read a `&dyn Fn` that lives on this stack frame
    // until `full` returns; whisper.cpp only calls them from inside `full`.
    unsafe {
        params.set_progress_callback(Some(on_progress));
        params.set_progress_callback_user_data(&report as *const &dyn Fn(i32) as *mut c_void);
        params.set_abort_callback(Some(should_abort));
        params.set_abort_callback_user_data(&cancelled as *const &dyn Fn() -> bool as *mut c_void);
    }
    let outcome = state.full(params, &samples);
    if cancelled() {
        return Err(Error::msg("Cancelled."));
    }
    outcome.map_err(|error| Error::msg(format!("Speech recognition failed: {error}")))?;

    let end_of_text = context.token_eot();
    let mut segments = Vec::new();
    let mut words = Vec::new();
    for segment in state.as_iter() {
        let text = segment.to_str_lossy().map(|t| t.trim().to_string()).unwrap_or_default();
        // Whisper marks non-speech as [Music], [BLANK_AUDIO] and similar.
        if text.is_empty() || text.starts_with('[') {
            continue;
        }
        let (start, end) = (centiseconds(segment.start_timestamp()), centiseconds(segment.end_timestamp()));
        words.extend(segment_words(&segment, end_of_text, start, end));
        segments.push(TimedSegment { text, start, end });
    }
    progress(1.0);
    let words = (!words.is_empty()).then_some(words);
    Ok((segments, words))
}

fn threads() -> c_int {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8) as c_int
}

fn centiseconds(value: i64) -> f64 {
    value.max(0) as f64 / 100.0
}

/// The words of a segment, from its tokens: a token that starts with a space
/// starts a word. Token bytes are joined before decoding, since one letter can
/// be split across two tokens.
fn segment_words(segment: &WhisperSegment, end_of_text: i32, start: f64, end: f64) -> Vec<TimedWord> {
    let mut tokens = Vec::new();
    for index in 0..segment.n_tokens() {
        let Some(token) = segment.get_token(index) else { continue };
        let data = token.token_data();
        // Timestamps and the special tokens all come after end-of-text.
        if data.id >= end_of_text {
            continue;
        }
        let Ok(bytes) = token.to_bytes() else { continue };
        tokens.push((bytes.to_vec(), centiseconds(data.t0), centiseconds(data.t1)));
    }
    words_from_tokens(&tokens, start, end)
}

/// Joins (bytes, start, end) tokens into words, clamped to the segment.
fn words_from_tokens(tokens: &[(Vec<u8>, f64, f64)], start: f64, end: f64) -> Vec<TimedWord> {
    let mut words = Vec::new();
    let mut current: Option<(Vec<u8>, f64, f64)> = None;
    let finish = |word: Option<(Vec<u8>, f64, f64)>, words: &mut Vec<TimedWord>| {
        let Some((bytes, word_start, word_end)) = word else { return };
        let text = String::from_utf8_lossy(&bytes).trim().to_string();
        if text.is_empty() {
            return;
        }
        let word_start = word_start.clamp(start, end);
        words.push(TimedWord { word: text, start: word_start, end: word_end.clamp(word_start + 0.04, end.max(word_start + 0.04)) });
    };
    for (bytes, token_start, token_end) in tokens {
        let starts_word = bytes.first() == Some(&b' ');
        match current.as_mut() {
            Some((word, _, word_end)) if !starts_word => {
                word.extend_from_slice(bytes);
                *word_end = word_end.max(*token_end);
            }
            _ => {
                finish(current.take(), &mut words);
                current = Some((bytes.clone(), *token_start, *token_end));
            }
        }
    }
    finish(current.take(), &mut words);
    words
}

unsafe extern "C" fn on_progress(_: *mut whisper_context, _: *mut whisper_state, percent: c_int, data: *mut c_void) {
    // SAFETY: `data` is the `&dyn Fn(i32)` set in `transcribe`, alive during `full`.
    let report = unsafe { &*(data as *const &dyn Fn(i32)) };
    report(percent);
}

unsafe extern "C" fn should_abort(data: *mut c_void) -> bool {
    // SAFETY: `data` is the `&dyn Fn() -> bool` set in `transcribe`, alive during `full`.
    let cancelled = unsafe { &*(data as *const &dyn Fn() -> bool) };
    cancelled()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(text: &str, start: f64, end: f64) -> (Vec<u8>, f64, f64) {
        (text.as_bytes().to_vec(), start, end)
    }

    #[test]
    fn tokens_join_into_words() {
        let tokens =
            vec![token(" Jak", 0.07, 0.16), token("ie", 0.16, 0.26), token(" jest", 0.26, 0.47), token(" naj", 0.47, 0.74), token("lepszy?", 0.74, 1.1)];
        let words = words_from_tokens(&tokens, 0.0, 2.0);
        let texts: Vec<&str> = words.iter().map(|w| w.word.as_str()).collect();
        assert_eq!(texts, ["Jakie", "jest", "najlepszy?"]);
        assert!((words[0].start - 0.07).abs() < 1e-9 && (words[0].end - 0.26).abs() < 1e-9);
        assert!((words[2].end - 1.1).abs() < 1e-9);
    }

    #[test]
    fn a_letter_split_across_tokens_is_decoded_whole() {
        // "ś" is 0xC5 0x9B in UTF-8.
        let tokens = vec![(vec![b' ', b'c', b'z', b'e', 0xC5], 0.0, 0.2), (vec![0x9B, b'c'], 0.2, 0.4)];
        let words = words_from_tokens(&tokens, 0.0, 1.0);
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].word, "cześc");
    }

    #[test]
    fn word_times_stay_inside_the_segment() {
        let words = words_from_tokens(&[token(" late", 4.9, 5.6)], 1.0, 5.0);
        assert!(words[0].start >= 1.0 && words[0].end <= 5.0 && words[0].end > words[0].start);
    }

    #[test]
    fn urls_are_pinned_to_a_commit() {
        assert!(model_url("tiny").contains(REVISION));
        assert!(MODELS.iter().all(|m| m.sha256.len() == 64));
    }

    #[test]
    fn a_short_partial_file_is_resumed_and_a_full_one_dropped() {
        let dir = std::env::temp_dir().join(format!("vf-ai-resume-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let partial = dir.join("model.download");
        std::fs::write(&partial, b"hello").unwrap();
        let mut hasher = Sha256::new();
        assert_eq!(resume_point(&partial, 100, &mut hasher).unwrap(), 5);
        assert_eq!(hex(&hasher.finalize()), hex(&Sha256::digest(b"hello")));
        assert_eq!(resume_point(&partial, 5, &mut Sha256::new()).unwrap(), 0);
        assert!(!partial.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
