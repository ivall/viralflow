//! Speech recognition for captions (whisper.cpp through libavfilter, in
//! process) and turning what was said into timed caption cues.

pub mod openrouter;
pub mod speech;
pub mod transcript;
pub mod whisper;
