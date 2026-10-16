//! The sound of a clip as a small WAV for cloud speech recognition
//! (16 kHz mono, 16-bit: what speech models use, a third of the size).

use std::path::Path;

use vf_core::Result;
use vf_media::audio::{AudioDecoder, CHANNELS, SAMPLE_RATE};
use vf_media::filter;

const RATE: u32 = 16_000;

/// `duration` seconds of `path` from `offset`.
pub fn speech_wav(path: &Path, offset: f64, duration: f64) -> Result<Vec<u8>> {
    let mut decoder = AudioDecoder::open(path)?;
    decoder.seek(offset.max(0.0))?;
    let frames = (duration.max(0.1) * SAMPLE_RATE as f64) as usize;
    let mut stereo = vec![0.0f32; frames * CHANNELS];
    decoder.read(&mut stereo)?;
    // Low-pass below the new Nyquist frequency, then keep every third sample (48 → 16 kHz).
    let filtered = filter::apply("lowpass=f=7600,lowpass=f=7600", &stereo)?;
    let step = (SAMPLE_RATE / RATE) as usize;
    let samples: Vec<i16> =
        filtered.chunks_exact(CHANNELS * step).map(|chunk| (((chunk[0] + chunk[1]) * 0.5).clamp(-1.0, 1.0) * i16::MAX as f32) as i16).collect();
    Ok(wav(&samples, RATE))
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
}
