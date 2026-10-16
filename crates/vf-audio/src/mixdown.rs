//! The export's soundtrack: the same mix the preview plays (see [`crate::mixer`]),
//! rendered from start to end, normalised to the loudness target and limited.

use vf_core::model::Project;
use vf_core::Result;
use vf_media::audio::{CHANNELS, SAMPLE_RATE};

use crate::loudness::{self, Master};
use crate::mixer::{audible_clips, render_all, Mix};

/// Interleaved 48 kHz stereo of the first `total` seconds, or None when silent.
/// `progress` gets 0..1; `cancelled` stops early with an error.
pub fn mixdown(project: &Project, total: f64, progress: &dyn Fn(f64), cancelled: &dyn Fn() -> bool) -> Result<Option<Vec<f32>>> {
    let frames = (total.max(0.0) * SAMPLE_RATE as f64).round() as usize;
    let last = frames as i64;
    let clips: Vec<_> = audible_clips(project).into_iter().filter(|clip| clip.first < last && clip.path.is_file()).collect();
    if clips.is_empty() {
        return Ok(None);
    }
    let mut mix = Mix::new(clips);
    let mut out = vec![0.0f32; frames * CHANNELS];
    let finished = render_all(
        &mut mix,
        frames,
        |at, block| {
            out[at * CHANNELS..at * CHANNELS + block.len()].copy_from_slice(block);
            progress(at as f64 / frames.max(1) as f64 * 0.95);
        },
        cancelled,
    );
    if !finished {
        return Err(vf_core::Error::msg("Export cancelled."));
    }
    debug_assert!(!mix.is_empty());
    let target = loudness::target(project.export.loudness);
    let measured = target.and_then(|_| loudness::integrated(&out));
    Master::new(target.is_some(), loudness::normalising_gain(target, measured)).process(&mut out);
    progress(1.0);
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silent_projects_have_no_soundtrack() {
        let project = Project::default();
        assert!(mixdown(&project, 3.0, &|_| {}, &|| false).unwrap().is_none());
    }
}
