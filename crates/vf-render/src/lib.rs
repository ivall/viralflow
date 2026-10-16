pub mod compositor;
mod frames;
pub mod gpu;
pub mod renderer;
pub mod text;

pub use gpu::Gpu;
pub use renderer::{FrameRenderer, Mode};

/// Bytes the preview's video frame cache holds now.
pub fn preview_frame_bytes() -> usize {
    frames::FrameCache::shared().lock().map(|cache| cache.bytes()).unwrap_or(0)
}

/// Drops every cached preview frame (when the editor closes).
pub fn release_preview_frames() {
    if let Ok(mut cache) = frames::FrameCache::shared().lock() {
        cache.clear();
    }
}
pub use text::TextEngine;
