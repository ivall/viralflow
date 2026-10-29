//! The AI editor: one set of tools that language models use to edit the open
//! project, offered in two places. The chat in the editor talks to a model on
//! OpenRouter; the MCP server lets AI apps (Claude, Cursor, VS Code…) drive
//! ViralFlow. Both run the same tools on the UI thread (`bridge`), so every
//! change shows up live, autosaves and can be undone.

pub mod args;
pub mod bridge;
pub mod chat;
pub mod describe;
pub mod edits;
pub mod host;
pub mod jobs;
pub mod mcp;
pub mod prompt;
pub mod specs;

use serde_json::{json, Value};

/// Tool results longer than this are cut (they go back into the model's context).
const MAX_RESULT: usize = 60_000;

/// A picture a tool shows the model (a rendered frame, a frame of a file).
#[derive(Debug, Clone)]
pub struct Picture {
    pub jpeg: Vec<u8>,
    pub caption: String,
}

/// What a tool returns: JSON for the model, maybe pictures.
#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub value: Value,
    pub pictures: Vec<Picture>,
    pub error: bool,
}

impl ToolOutput {
    pub fn ok(value: Value) -> Self {
        ToolOutput { value, pictures: vec![], error: false }
    }

    pub fn error(message: impl Into<String>) -> Self {
        ToolOutput { value: json!({ "error": message.into() }), pictures: vec![], error: true }
    }

    pub fn from_result(result: Result<Value, String>) -> Self {
        match result {
            Ok(value) => ToolOutput::ok(value),
            Err(message) => ToolOutput::error(message),
        }
    }

    /// The JSON as text for the model, cut when it is very long.
    pub fn text(&self) -> String {
        let text = serde_json::to_string(&self.value).unwrap_or_default();
        if text.len() <= MAX_RESULT {
            return text;
        }
        let mut end = MAX_RESULT;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}… [cut: the result was {} characters; ask for less, e.g. a time window or fewer ids]", &text[..end], text.len())
    }

    /// One line for people: the tool's own summary, or its error.
    pub fn summary(&self) -> String {
        if let Some(error) = self.value.get("error").and_then(Value::as_str) {
            return error.to_string();
        }
        self.value.get("summary").and_then(Value::as_str).unwrap_or_default().to_string()
    }
}

/// What the UI thread answers for a tool call.
pub enum Reply {
    Now(ToolOutput),
    /// Work that finishes off the UI thread (rendering, waiting for a job).
    /// It gets how long it may wait before answering "still running".
    Later(Box<dyn FnOnce(std::time::Duration) -> ToolOutput + Send>),
}

impl Reply {
    pub fn ok(value: Value) -> Self {
        Reply::Now(ToolOutput::ok(value))
    }

    pub fn error(message: impl Into<String>) -> Self {
        Reply::Now(ToolOutput::error(message))
    }
}

/// Who asked: the chat of a project (its id) or an AI app over MCP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    Chat(String),
    Mcp,
}

/// RGBA pixels to a JPEG.
pub fn jpeg(width: u32, height: u32, rgba: &[u8], quality: u8) -> Option<Vec<u8>> {
    let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, quality).encode(&rgb, u16::try_from(width).ok()?, u16::try_from(height).ok()?, jpeg_encoder::ColorType::Rgb).ok()?;
    Some(out)
}

/// Seconds rounded to milliseconds (shorter JSON for the model).
pub fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}
