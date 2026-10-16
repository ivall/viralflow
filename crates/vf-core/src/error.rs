/// Error shared by every ViralFlow crate. Messages are written for the person
/// using the app: the UI shows them as they are.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error("File error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid data: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Operation cancelled")]
    Cancelled,
}

impl Error {
    pub fn msg(message: impl Into<String>) -> Self {
        Error::Message(message.into())
    }

    /// Wraps any foreign error as a message (crates such as reqwest or libav).
    pub fn from_display(error: impl std::fmt::Display) -> Self {
        Error::Message(error.to_string())
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
