//! Real transcription; runs when VF_WHISPER_MODEL and VF_SPEECH point to a
//! model and a speech recording.

use std::path::PathBuf;

use vf_ai::transcript::{build_transcript, CaptionOptions};
use vf_ai::whisper::{transcribe, Request};

#[test]
fn transcribes_speech_in_process() {
    let (Ok(model), Ok(speech)) = (std::env::var("VF_WHISPER_MODEL"), std::env::var("VF_SPEECH")) else {
        eprintln!("skipped: set VF_WHISPER_MODEL and VF_SPEECH");
        return;
    };
    let work = std::env::temp_dir().join("vf-ai-whisper-test");
    let request = Request { model: &PathBuf::from(model), source: &PathBuf::from(speech), offset: 0.0, duration: 9.0, language: "en", work_dir: &work };
    let (segments, words) = transcribe(&request, &|pass, f| eprintln!("pass {pass}: {:.0}%", f * 100.0), &|| false).unwrap();
    eprintln!("{segments:?}");
    let text: String = segments.iter().map(|s| s.text.to_lowercase()).collect::<Vec<_>>().join(" ");
    assert!(text.contains("test") && text.contains("captions"), "{text}");
    let words = words.expect("word timings");
    assert!(words.len() >= 10, "{words:?}");
    let transcript = build_transcript(segments, Some(words), 9.0, CaptionOptions { max_words: 3 });
    eprintln!("{:?}", transcript.cues);
    assert_eq!(transcript.alignment, "word");
    assert!(transcript.cues.len() >= 4);
}

#[test]
fn speech_wav_is_16k_mono() {
    let Ok(speech) = std::env::var("VF_SPEECH") else { return };
    let wav = vf_ai::speech::speech_wav(&PathBuf::from(speech), 1.0, 2.0).unwrap();
    assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 16_000);
    assert_eq!(wav.len(), 44 + 2 * 32_000);
    let loud = wav[44..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs()).max().unwrap();
    assert!(loud > 1000, "{loud}");
}
