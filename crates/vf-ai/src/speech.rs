//! The sound of a clip as speech models want it: 16 kHz mono. As floats for
//! local recognition, or as a small 16-bit WAV (a third of the size) for cloud
//! recognition.

use std::path::Path;

use vf_core::{Error, Result};
use vf_media::audio::{AudioDecoder, CHANNELS, SAMPLE_RATE};
use vf_media::filter::AudioFilter;

pub const RATE: u32 = 16_000;
/// Source samples per speech sample (48 → 16 kHz).
const STEP: usize = (SAMPLE_RATE / RATE) as usize;
/// Low-pass below the new Nyquist frequency before every third sample is kept.
const ANTI_ALIAS: &str = "lowpass=f=7600,lowpass=f=7600";

/// `duration` seconds of `path` from `offset`, 16 kHz mono. Decoded a second at
/// a time, so an hour of sound never sits in memory at 48 kHz stereo.
/// `progress` gets the fraction read.
pub fn speech_samples(path: &Path, offset: f64, duration: f64, progress: &dyn Fn(f64), cancelled: &dyn Fn() -> bool) -> Result<Vec<f32>> {
    let mut decoder = AudioDecoder::open(path)?;
    decoder.seek(offset.max(0.0))?;
    let mut filter = AudioFilter::new(ANTI_ALIAS)?;
    let total = (duration.max(0.1) * SAMPLE_RATE as f64) as usize;
    let mut block = vec![0.0f32; SAMPLE_RATE as usize * CHANNELS];
    let mut filtered = Vec::new();
    let mut out = Vec::with_capacity(total / STEP + 1);
    let mut done = 0usize;
    while done < total {
        if cancelled() {
            return Err(Error::msg("Cancelled."));
        }
        let frames = (total - done).min(SAMPLE_RATE as usize);
        let slice = &mut block[..frames * CHANNELS];
        decoder.read(slice)?;
        filter.push(slice, &mut filtered)?;
        decimate(&mut filtered, &mut out);
        done += frames;
        progress(done as f64 / total as f64);
    }
    filter.finish(&mut filtered)?;
    decimate(&mut filtered, &mut out);
    out.truncate(total / STEP);
    Ok(out)
}

/// Moves every whole group of `STEP` stereo frames from `filtered` to `out` as one mono sample.
fn decimate(filtered: &mut Vec<f32>, out: &mut Vec<f32>) {
    let group = CHANNELS * STEP;
    let whole = filtered.len() / group * group;
    out.extend(filtered[..whole].chunks_exact(group).map(|frames| (frames[0] + frames[1]) * 0.5));
    filtered.drain(..whole);
}

/// `duration` seconds of `path` from `offset` as a 16 kHz mono WAV file.
pub fn speech_wav(path: &Path, offset: f64, duration: f64) -> Result<Vec<u8>> {
    let samples = speech_samples(path, offset, duration, &|_| {}, &|| false)?;
    let pcm: Vec<i16> = samples.iter().map(|s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).collect();
    Ok(wav(&pcm, RATE))
}

/// A mono 16-bit PCM WAV file.
fn wav(samples: &[i16], rate: u32) -> Vec<u8> {
    let data = samples.len() as u32 * 2;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_valid_header() {
        let file = wav(&[0, 1000, -1000], 16_000);
        assert_eq!(&file[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(file[4..8].try_into().unwrap()), 36 + 6);
        assert_eq!(u32::from_le_bytes(file[24..28].try_into().unwrap()), 16_000);
        assert_eq!(file.len(), 44 + 6);
    }

    #[test]
    fn decimation_keeps_partial_groups_for_the_next_block() {
        let mut filtered: Vec<f32> = (0..14).map(|i| i as f32).collect();
        let mut out = Vec::new();
        decimate(&mut filtered, &mut out);
        // Two groups of three stereo frames; frames 0 and 6 are kept (averaged L/R).
        assert_eq!(out, vec![0.5, 6.5]);
        assert_eq!(filtered, vec![12.0, 13.0]);
    }
}
