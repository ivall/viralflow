//! ViralFlow core: the project document, the engine that turns it into frames
//! and the editing operations. No I/O and no GPU, so everything here is tested
//! directly.

pub mod engine;
pub mod error;
pub mod model;

pub use error::{Error, Result};
pub mod captions;
pub mod edit;
pub mod generators;
pub mod migrate;
pub mod pace;
pub mod presets;
pub mod srt;
pub mod time;
pub mod timeline;
