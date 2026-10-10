//! Optional cloud AI through OpenRouter (only used when the user adds an API key).

use std::time::Duration;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ai::captions::{TimedSegment, TimedWord};
use crate::error::{AppError, AppResult};

const CHAT_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
const TRANSCRIPTION_URL: &str = "https://openrouter.ai/api/v1/audio/transcriptions";
pub const DEFAULT_TRANSCRIPTION_MODEL: &str = "openai/whisper-large-v3";

pub struct Client {
    key: String,
    model: String,
    http: reqwest::Client,
}

impl Client {
    pub fn new(key: &str, model: &str) -> AppResult<Self> {
        if key.trim().is_empty() {
            return Err(AppError::msg("Add an OpenRouter API key in Settings → AI to use this feature."));
        }
        let http = reqwest::Client::builder().timeout(Duration::from_secs(300)).build()?;
        Ok(Client { key: key.trim().to_string(), model: if model.trim().is_empty() { "openai/gpt-6-luna".into() } else { model.trim().to_string() }, http })
    }

    fn request(&self, url: &str) -> reqwest::RequestBuilder {
        self.http.post(url).bearer_auth(&self.key).header("HTTP-Referer", "https://github.com/ivall/viralflow").header("X-Title", "ViralFlow")
    }

    async fn send(&self, url: &str, body: Value) -> AppResult<Value> {
        let response = self.request(url).json(&body).send().await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            let message = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|value| value.pointer("/error/message").and_then(Value::as_str).map(String::from))
                .unwrap_or_else(|| text.chars().take(300).collect());
            return Err(AppError::msg(format!("OpenRouter returned {}: {}", status.as_u16(), message)));
        }
        Ok(serde_json::from_str(&text)?)
    }

    pub async fn chat(&self, messages: Value, temperature: f64, json_mode: bool) -> AppResult<String> {
        let mut body = json!({ "model": self.model, "messages": messages, "temperature": temperature, "max_tokens": 16000 });
        if json_mode {
            body["response_format"] = json!({ "type": "json_object" });
        }
        let value = self.send(CHAT_URL, body).await?;
        let content = value.pointer("/choices/0/message/content").and_then(Value::as_str).unwrap_or_default().trim().to_string();
        if content.is_empty() {
            return Err(AppError::msg("The AI model returned an empty answer."));
        }
        Ok(content)
    }

    pub async fn chat_json<T: for<'de> Deserialize<'de>>(&self, system: &str, user: Value, temperature: f64) -> AppResult<T> {
        let content = self.chat(json!([{ "role": "system", "content": system }, { "role": "user", "content": user }]), temperature, true).await?;
        let trimmed = content.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
        serde_json::from_str(trimmed).map_err(|error| AppError::msg(format!("The AI answer could not be read: {error}")))
    }

    pub async fn transcribe(&self, model: &str, wav: &[u8], language: &str) -> AppResult<(Vec<TimedSegment>, Option<Vec<TimedWord>>)> {
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
        let value = self.send(TRANSCRIPTION_URL, body).await?;
        #[derive(Deserialize)]
        struct Word {
            word: String,
            start: f64,
            end: f64,
        }
        #[derive(Deserialize)]
        struct Segment {
            text: String,
            start: f64,
            end: f64,
        }
        let words: Vec<Word> = value.get("words").cloned().map(serde_json::from_value).transpose()?.unwrap_or_default();
        let mut segments: Vec<Segment> = value.get("segments").cloned().map(serde_json::from_value).transpose()?.unwrap_or_default();
        if segments.is_empty() {
            if let Some(text) = value.get("text").and_then(Value::as_str) {
                segments.push(Segment { text: text.to_string(), start: 0.0, end: 0.0 });
            }
        }
        let segments = segments.into_iter().map(|s| TimedSegment { text: s.text.trim().to_string(), start: s.start, end: s.end }).collect();
        let words = (!words.is_empty()).then(|| words.into_iter().map(|w| TimedWord { word: w.word, start: w.start, end: w.end }).collect());
        Ok((segments, words))
    }
}

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ScriptPlan {
    pub title: String,
    pub script: String,
    pub hook: String,
    pub cta: String,
    pub text_cues: Vec<PlanCue>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct PlanCue {
    pub text: String,
    pub start: f64,
    pub duration: f64,
    pub emphasis: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BriefRequest {
    pub topic: String,
    pub goal: String,
    pub platform: String,
    pub language: String,
    pub duration: f64,
    pub tone: String,
    pub cta: String,
    pub script: String,
}

const PLAN_PROMPT: &str = r#"You are the content director inside a short-form video editor.
Create a concise, high-retention vertical video plan. Return ONLY a JSON object:
{"title":"...","script":"...","hook":"...","cta":"...","textCues":[{"text":"...","start":0,"duration":1.2,"emphasis":false}]}
Rules:
- Write in the requested language and match platform, goal and tone.
- The spoken script must fit naturally inside the requested duration.
- The hook lands immediately and has at most 10 words.
- textCues are on-screen captions that follow the script in order, 1-3 words each, never overlap and stay inside the duration.
- No markdown, no explanations."#;

pub async fn plan(client: &Client, brief: &BriefRequest) -> AppResult<ScriptPlan> {
    let user = format!(
        "Topic: {}\nGoal: {}\nPlatform: {}\nLanguage: {}\nTarget duration: {:.0} seconds\nTone: {}\nCall to action: {}\nDraft to improve (may be empty): {}",
        brief.topic, brief.goal, brief.platform, brief.language, brief.duration, brief.tone, brief.cta, brief.script
    );
    let mut plan: ScriptPlan = client.chat_json(PLAN_PROMPT, Value::String(user), 0.65).await?;
    let total = brief.duration.max(3.0);
    plan.text_cues.retain(|cue| !cue.text.trim().is_empty() && cue.start < total);
    plan.text_cues.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
    let mut last_end = 0.0;
    for cue in &mut plan.text_cues {
        cue.start = cue.start.max(last_end);
        if cue.duration < 0.25 {
            cue.duration = 0.9;
        }
        cue.duration = cue.duration.min(total - cue.start).max(0.1);
        last_end = cue.start + cue.duration;
    }
    Ok(plan)
}

pub async fn rewrite(client: &Client, text: &str, instruction: &str) -> AppResult<String> {
    let instruction = if instruction.trim().is_empty() { "Make it tighter and more engaging without changing the meaning." } else { instruction.trim() };
    let answer = client
        .chat(
            json!([
                { "role": "system", "content": "You rewrite short-form video copy. Keep the language of the source. Return only the rewritten text, without quotes or explanations." },
                { "role": "user", "content": format!("Instruction: {instruction}\nText: {text}") }
            ]),
            0.55,
            false,
        )
        .await?;
    Ok(answer.trim_matches(|c| c == '"' || c == '\'' || c == '`').trim().to_string())
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ImageEvent {
    pub image_index: usize,
    pub start: f64,
    pub end: f64,
}

/// Picks moments of a narrated video where each supplied image fits the speech.
pub async fn match_images(client: &Client, transcript: &str, cues: &Value, duration: f64, images: &[(String, Vec<u8>)]) -> AppResult<Vec<ImageEvent>> {
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
    let answer: Answer = client.chat_json("You are a precise picture editor for short-form video.", Value::Array(content), 0.2).await?;
    Ok(answer.events)
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ModelSummary {
    pub id: String,
    pub name: String,
    pub context_length: u64,
    /// USD per one million input tokens.
    pub prompt_price: f64,
    /// USD per one million output tokens.
    pub completion_price: f64,
    pub vision: bool,
    pub json_mode: bool,
    pub created: i64,
}

/// Public model catalogue (no API key required). `transcription` lists speech-to-text
/// models, which the default catalogue leaves out.
pub async fn list_models(transcription: bool) -> AppResult<Vec<ModelSummary>> {
    let http = reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?;
    let url = if transcription { "https://openrouter.ai/api/v1/models?output_modalities=transcription" } else { "https://openrouter.ai/api/v1/models" };
    let output = if transcription { "transcription" } else { "text" };
    let response = http.get(url).send().await?.error_for_status()?;
    let value: Value = response.json().await?;
    let price = |entry: &Value, key: &str| -> f64 {
        entry
            .pointer(&format!("/pricing/{key}"))
            .and_then(Value::as_str)
            .and_then(|text| text.parse::<f64>().ok())
            .map(|per_token| per_token * 1_000_000.0)
            .unwrap_or(0.0)
    };
    let has = |entry: &Value, pointer: &str, needle: &str| -> bool {
        entry.pointer(pointer).and_then(Value::as_array).is_some_and(|items| items.iter().any(|item| item.as_str() == Some(needle)))
    };
    let mut models: Vec<ModelSummary> = value
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|entry| has(entry, "/architecture/output_modalities", output))
        .filter_map(|entry| {
            Some(ModelSummary {
                id: entry.get("id")?.as_str()?.to_string(),
                name: entry.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                context_length: entry.get("context_length").and_then(Value::as_u64).unwrap_or(0),
                prompt_price: price(entry, "prompt"),
                completion_price: price(entry, "completion"),
                vision: has(entry, "/architecture/input_modalities", "image"),
                json_mode: has(entry, "/supported_parameters", "response_format") || has(entry, "/supported_parameters", "structured_outputs"),
                created: entry.get("created").and_then(Value::as_i64).unwrap_or(0),
            })
        })
        .collect();
    models.sort_by_key(|model| std::cmp::Reverse(model.created));
    Ok(models)
}
