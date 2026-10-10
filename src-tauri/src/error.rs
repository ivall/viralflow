use serde::{Serialize, Serializer};

/// Error type returned by every command. It serialises to a plain message so the
/// frontend can show it directly.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Message(String),
    #[error("File error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid data: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("{0}")]
    Tauri(#[from] tauri::Error),
    #[error("Operation cancelled")]
    Cancelled,
}

impl AppError {
    pub fn msg(message: impl Into<String>) -> Self {
        AppError::Message(message.into())
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
