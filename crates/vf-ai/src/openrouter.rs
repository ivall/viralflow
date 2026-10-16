//! Optional cloud AI through OpenRouter (only used when the user adds an API
//! key): text rewriting, cloud speech recognition and matching photos to
//! speech. Calls block, so they run on background threads.

use std::time::Duration;

use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Value};
use vf_core::{Error, Result};

use crate::transcript::{TimedSegment, TimedWord};

const CHAT_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
const TRANSCRIPTION_URL: &str = "https://openrouter.ai/api/v1/audio/transcriptions";
pub const DEFAULT_MODEL: &str = "openai/gpt-6-luna";
pub const DEFAULT_TRANSCRIPTION_MODEL: &str = "openai/whisper-large-v3";

pub struct Client {
    key: String,
    model: String,
    agent: ureq::Agent,
}

fn agent(timeout: u64) -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(timeout))).http_status_as_error(false).build().into()
}

fn read_json(mut response: ureq::http::Response<ureq::Body>) -> Result<Value> {
    let status = response.status();
    let text = response.body_mut().read_to_string().map_err(|error| Error::msg(format!("OpenRouter could not be read: {error}")))?;
    if !status.is_success() {
        let message = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|value| value.pointer("/error/message").and_then(Value::as_str).map(String::from))
            .unwrap_or_else(|| text.chars().take(300).collect());
        return Err(Error::msg(format!("OpenRouter returned {}: {message}", status.as_u16())));
    }
    serde_json::from_str(&text).map_err(|error| Error::msg(format!("OpenRouter sent an unreadable answer: {error}")))
}

impl Client {
    pub fn new(key: &str, model: &str) -> Result<Self> {
        if key.trim().is_empty() {
            return Err(Error::msg("Add an OpenRouter API key in Settings → AI to use this feature."));
        }
        let model = if model.trim().is_empty() { DEFAULT_MODEL } else { model.trim() };
        Ok(Client { key: key.trim().to_string(), model: model.to_string(), agent: agent(300) })
    }

    fn post(&self, url: &str, body: Value) -> Result<Value> {
        let response = self
            .agent
            .post(url)
            .header("Authorization", &format!("Bearer {}", self.key))
            .header("HTTP-Referer", "https://github.com/ivall/viralflow")
            .header("X-Title", "ViralFlow")
            .send_json(&body)
            .map_err(|error| Error::msg(format!("OpenRouter could not be reached: {error}")))?;
        read_json(response)
    }

    pub fn chat(&self, messages: Value, temperature: f64, json_mode: bool) -> Result<String> {
        let mut body = json!({ "model": self.model, "messages": messages, "temperature": temperature, "max_tokens": 16000 });
        if json_mode {
            body["response_format"] = json!({ "type": "json_object" });
        }
        let value = self.post(CHAT_URL, body)?;
        let content = value.pointer("/choices/0/message/content").and_then(Value::as_str).unwrap_or_default().trim().to_string();
        if content.is_empty() {
            return Err(Error::msg("The AI model returned an empty answer."));
        }
        Ok(content)
    }

    pub fn chat_json<T: for<'de> Deserialize<'de>>(&self, system: &str, user: Value, temperature: f64) -> Result<T> {
        let content = self.chat(json!([{ "role": "system", "content": system }, { "role": "user", "content": user }]), temperature, true)?;
        let trimmed = content.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
        serde_json::from_str(trimmed).map_err(|error| Error::msg(format!("The AI answer could not be read: {error}")))
    }

    /// Speech recognition of a WAV file; segments and word timings in seconds.
    pub fn transcribe(&self, model: &str, wav: &[u8], language: &str) -> Result<(Vec<TimedSegment>, Option<Vec<TimedWord>>)> {
        let language = language.trim().to_lowercase();
        let mut body = json!({
            "model": if model.trim().is_empty() { DEFAULT_TRANSCRIPTION_MODEL } else { model.trim() },
            "input_audio": { "data": base64::engine::general_purpose::STANDARD.encode(wav), "format": "wav" },
            "temperature": 0,
            "response_format": "verbose_json",
            "timestamp_granularities": ["word", "segment"]
        });
        if language.len() == 2 {
            body["language"] = json!(language);
        }
        let value = self.post(TRANSCRIPTION_URL, body)?;
        Ok(parse_transcription(&value))
    }
}

fn parse_transcription(value: &Value) -> (Vec<TimedSegment>, Option<Vec<TimedWord>>) {
    let number = |entry: &Value, key: &str| entry.get(key).and_then(Value::as_f64).unwrap_or(0.0);
    let text_of = |entry: &Value, key: &str| entry.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let list = |key: &str| value.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
    let mut segments: Vec<TimedSegment> = list("segments")
        .iter()
        .map(|s| TimedSegment { text: text_of(s, "text"), start: number(s, "start"), end: number(s, "end") })
        .filter(|s| !s.text.is_empty())
        .collect();
    if segments.is_empty() {
        let text = text_of(value, "text");
        if !text.is_empty() {
            segments.push(TimedSegment { text, start: 0.0, end: 0.0 });
        }
    }
    let words: Vec<TimedWord> = list("words")
        .iter()
        .map(|w| TimedWord { word: text_of(w, "word"), start: number(w, "start"), end: number(w, "end") })
        .filter(|w| !w.word.is_empty())
        .collect();
    (segments, (!words.is_empty()).then_some(words))
}

/// Rewrites a short piece of video copy, keeping its language.
pub fn rewrite(client: &Client, text: &str, instruction: &str) -> Result<String> {
    let instruction = if instruction.trim().is_empty() { "Make it tighter and more engaging without changing the meaning." } else { instruction.trim() };
    let answer = client.chat(
        json!([
            { "role": "system", "content": "You rewrite short-form video copy. Keep the language of the source. Return only the rewritten text, without quotes or explanations." },
            { "role": "user", "content": format!("Instruction: {instruction}\nText: {text}") }
        ]),
        0.55,
        false,
    )?;
    Ok(answer.trim_matches(|c| c == '"' || c == '\'' || c == '`').trim().to_string())
}

/// A photo placed where the narration talks about it.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageEvent {
    pub image_index: usize,
    pub start: f64,
    pub end: f64,
}

/// Picks moments of a narrated video where each photo (JPEG bytes) fits the speech.
pub fn match_images(client: &Client, transcript: &str, cues: &Value, duration: f64, images: &[(String, Vec<u8>)]) -> Result<Vec<ImageEvent>> {
    let names: Vec<String> = images.iter().enumerate().map(|(index, (name, _))| format!("{index}: {name}")).collect();
    let prompt = format!(
        "Arrange the supplied images over this narrated short video.\nVideo duration: {duration:.2} s\nTranscript: {transcript}\nTimed captions: {cues}\nImages in the attached order:\n{}\n\nReturn ONLY JSON: {{\"events\":[{{\"imageIndex\":0,\"start\":2.4,\"end\":5.8}}]}}\nRules: use an image only where its content matches the words spoken at that time; events are chronological, non-overlapping, at least 0.25 s apart; keep the first second clear; prefer 2.8-4.5 s placements; use each image at most once; it is fine to skip irrelevant images.",
        names.join("\n")
    );
    let mut content = vec![json!({ "type": "text", "text": prompt })];
    for (_, data) in images {
        content.push(json!({ "type": "image_url", "image_url": { "url": format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(data)) } }));
    }
    #[derive(Deserialize)]
    struct Answer {
        #[serde(default)]
        events: Vec<ImageEvent>,
    }
    let answer: Answer = client.chat_json("You are a precise picture editor for short-form video.", Value::Array(content), 0.2)?;
    Ok(answer.events)
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelSummary {
    pub id: String,
    pub name: String,
    /// USD per one million input tokens.
    pub prompt_price: f64,
    /// USD per one million output tokens.
    pub completion_price: f64,
    pub vision: bool,
}

/// The public model catalogue (no key needed), newest first. `transcription`
/// lists speech-to-text models, which the default catalogue leaves out.
pub fn list_models(transcription: bool) -> Result<Vec<ModelSummary>> {
    let url = if transcription { "https://openrouter.ai/api/v1/models?output_modalities=transcription" } else { "https://openrouter.ai/api/v1/models" };
    let response = agent(30).get(url).call().map_err(|error| Error::msg(format!("OpenRouter could not be reached: {error}")))?;
    let value = read_json(response)?;
    Ok(parse_models(&value, transcription))
}

fn parse_models(value: &Value, transcription: bool) -> Vec<ModelSummary> {
    let output = if transcription { "transcription" } else { "text" };
    let price = |entry: &Value, key: &str| -> f64 {
        entry.pointer(&format!("/pricing/{key}")).and_then(Value::as_str).and_then(|t| t.parse::<f64>().ok()).map(|p| p * 1_000_000.0).unwrap_or(0.0)
    };
    let has = |entry: &Value, pointer: &str, needle: &str| {
        entry.pointer(pointer).and_then(Value::as_array).is_some_and(|items| items.iter().any(|item| item.as_str() == Some(needle)))
    };
    let mut models: Vec<(i64, ModelSummary)> = value
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|entry| has(entry, "/architecture/output_modalities", output))
        .filter_map(|entry| {
            let summary = ModelSummary {
                id: entry.get("id")?.as_str()?.to_string(),
                name: entry.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                prompt_price: price(entry, "prompt"),
                completion_price: price(entry, "completion"),
                vision: has(entry, "/architecture/input_modalities", "image"),
            };
            Some((entry.get("created").and_then(Value::as_i64).unwrap_or(0), summary))
        })
        .collect();
    models.sort_by_key(|(created, _)| std::cmp::Reverse(*created));
    models.into_iter().map(|(_, model)| model).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_verbose_transcriptions() {
        let value = json!({
            "text": "Hello there",
            "segments": [{ "text": " Hello there ", "start": 0.0, "end": 1.2 }],
            "words": [{ "word": "Hello", "start": 0.0, "end": 0.5 }, { "word": "there", "start": 0.6, "end": 1.1 }]
        });
        let (segments, words) = parse_transcription(&value);
        assert_eq!(segments, vec![TimedSegment { text: "Hello there".into(), start: 0.0, end: 1.2 }]);
        assert_eq!(words.unwrap().len(), 2);
        let (segments, words) = parse_transcription(&json!({ "text": "Only text" }));
        assert_eq!(segments[0].text, "Only text");
        assert!(words.is_none());
    }

    #[test]
    fn lists_newest_models_with_prices() {
        let value = json!({ "data": [
            { "id": "a/old", "name": "Old", "created": 1, "pricing": { "prompt": "0.000001", "completion": "0.000002" }, "architecture": { "output_modalities": ["text"], "input_modalities": ["text"] } },
            { "id": "b/new", "name": "New", "created": 5, "pricing": { "prompt": "0.000003" }, "architecture": { "output_modalities": ["text"], "input_modalities": ["text", "image"] } },
            { "id": "c/speech", "created": 9, "architecture": { "output_modalities": ["transcription"] } }
        ]});
        let models = parse_models(&value, false);
        assert_eq!(models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["b/new", "a/old"]);
        assert!(models[0].vision && (models[0].prompt_price - 3.0).abs() < 1e-9);
        assert_eq!(parse_models(&value, true)[0].id, "c/speech");
    }

    #[test]
    fn needs_a_key() {
        assert!(Client::new("  ", "").is_err());
    }
}

#[cfg(test)]
mod live {
    /// `cargo test -p vf-ai -- --ignored`: reads the public catalogue (needs internet).
    #[test]
    #[ignore]
    fn public_catalogue() {
        let models = super::list_models(false).unwrap();
        assert!(models.len() > 20, "{}", models.len());
        let speech = super::list_models(true).unwrap();
        eprintln!("{} text models, {} speech models, newest: {}", models.len(), speech.len(), models[0].id);
    }
}
