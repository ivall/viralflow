//! The sound of one clip as the timeline plays it: decoded from the source,
//! sped up or slowed down without changing the pitch, and with the voice
//! cleanup applied. The preview mixer and the export soundtrack both read
//! clips through this, so they sound the same.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use vf_core::Result;
use vf_media::audio::{AudioDecoder, CHANNELS, SAMPLE_RATE};
use vf_media::filter::AudioFilter;

/// Source frames decoded per step.
const BLOCK: usize = 2048;

/// Voice cleanup for `amount` in 0..1: rumble cut, noise reduction, de-essing
/// and gentle compression.
pub fn enhance_chain(amount: f64) -> String {
    if amount <= 0.0 {
        return String::new();
    }
    let a = amount.min(1.0);
    format!(
        "highpass=f=80,afftdn=nf={:.1}:nr={:.1},deesser=i={:.2},acompressor=threshold={:.3}:ratio={:.2}:attack=15:release=120,alimiter=limit=0.95:level=false",
        -25.0 - a * 20.0,
        6.0 + a * 18.0,
        a * 0.5,
        0.5 - a * 0.3,
        2.0 + a * 2.0
    )
}

/// `atempo` only takes 0.5..=2 per instance, so larger changes are chained.
pub fn atempo_chain(speed: f64) -> String {
    let mut rate = speed.clamp(0.1, 100.0);
    let mut parts = Vec::new();
    while rate < 0.5 - 1e-9 {
        parts.push("atempo=0.5".to_string());
        rate /= 0.5;
    }
    while rate > 2.0 + 1e-9 {
        parts.push("atempo=2.0".to_string());
        rate /= 2.0;
    }
    if (rate - 1.0).abs() > 1e-6 {
        parts.push(format!("atempo={rate:.6}"));
    }
    parts.join(",")
}

/// Filters a clip's sound goes through (empty when it plays as recorded).
pub fn voice_chain(speed: f64, enhance: f64) -> String {
    [atempo_chain(speed), enhance_chain(enhance.clamp(0.0, 1.0))].into_iter().filter(|c| !c.is_empty()).collect::<Vec<_>>().join(",")
}

/// Frames the cleanup filters delay the sound by (noise removal works on
/// overlapping windows, the limiter looks ahead). Measured once per chain
/// with a tone burst, so it follows whatever libavfilter does.
pub fn latency(enhance: f64) -> usize {
    let chain = enhance_chain(enhance.clamp(0.0, 1.0));
    if chain.is_empty() {
        return 0;
    }
    static CACHE: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(&known) = cache.lock().expect("latency cache").get(&chain) {
        return known;
    }
    let rate = SAMPLE_RATE as usize;
    let onset = rate / 4;
    let burst: Vec<f32> = (0..rate)
        .flat_map(|i| {
            let v = if i >= onset { (i as f32 * 1000.0 * std::f32::consts::TAU / rate as f32).sin() * 0.5 } else { 0.0 };
            [v, v]
        })
        .collect();
    let first = |samples: &[f32]| samples.chunks_exact(CHANNELS).position(|f| f[0].abs() > 0.1);
    let measured = match (vf_media::filter::apply(&chain, &burst).ok().as_deref().and_then(first), first(&burst)) {
        (Some(out), Some(input)) => out.saturating_sub(input).min(rate / 4),
        _ => 0,
    };
    cache.lock().expect("latency cache").insert(chain, measured);
    measured
}

pub struct VoiceStream {
    decoder: AudioDecoder,
    chain: String,
    filter: Option<AudioFilter>,
    /// Processed samples (timeline rate, interleaved) not read yet.
    ready: VecDeque<f32>,
    block: Vec<f32>,
    staged: Vec<f32>,
    finished: bool,
    /// Filter delay in frames: dropped from the start, flushed with silence at the end.
    latency: usize,
    skip: usize,
}

// Used on one thread at a time (the mixer thread or the export thread).
unsafe impl Send for VoiceStream {}

impl VoiceStream {
    /// The clip's sound from `source` seconds into the file.
    pub fn open(path: &Path, source: f64, speed: f64, enhance: f64) -> Result<Self> {
        let mut decoder = AudioDecoder::open(path)?;
        decoder.seek(source.max(0.0))?;
        let chain = voice_chain(speed, enhance);
        let filter = if chain.is_empty() { None } else { Some(AudioFilter::new(&chain)?) };
        let latency = latency(enhance);
        Ok(VoiceStream {
            decoder,
            chain,
            filter,
            ready: VecDeque::new(),
            block: vec![0.0; BLOCK * CHANNELS],
            staged: Vec::new(),
            finished: false,
            latency,
            skip: latency,
        })
    }

    /// Filters in use (the mixer restarts the stream when the clip's settings change).
    pub fn chain(&self) -> &str {
        &self.chain
    }

    /// Continues from another point of the source with fresh filters.
    pub fn restart(&mut self, source: f64) -> Result<()> {
        self.decoder.seek(source.max(0.0))?;
        self.filter = if self.chain.is_empty() { None } else { Some(AudioFilter::new(&self.chain)?) };
        self.ready.clear();
        self.finished = false;
        self.skip = self.latency;
        Ok(())
    }

    /// Fills `out` (interleaved stereo) with the next samples; silence after the end.
    pub fn read(&mut self, out: &mut [f32]) -> Result<()> {
        while self.ready.len() < out.len() && !self.finished {
            let real = self.decoder.read(&mut self.block)?;
            self.staged.clear();
            match self.filter.as_mut() {
                Some(filter) if real > 0 => filter.push(&self.block[..real], &mut self.staged)?,
                Some(filter) => {
                    // Silence pushes the delayed tail out before the end.
                    filter.push(&vec![0.0; self.latency * CHANNELS], &mut self.staged)?;
                    filter.finish(&mut self.staged)?;
                    self.finished = true;
                }
                None => {
                    self.staged.extend_from_slice(&self.block[..real]);
                    self.finished = real == 0;
                }
            }
            let dropped = self.skip.min(self.staged.len() / CHANNELS);
            self.skip -= dropped;
            self.ready.extend(self.staged[dropped * CHANNELS..].iter().copied());
        }
        let take = self.ready.len().min(out.len());
        for (slot, sample) in out.iter_mut().zip(self.ready.drain(..take)) {
            *slot = sample;
        }
        out[take..].fill(0.0);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tempo_chains_extreme_speeds() {
        assert_eq!(atempo_chain(1.0), "");
        assert_eq!(atempo_chain(4.0), "atempo=2.0,atempo=2.000000");
        assert_eq!(atempo_chain(0.25), "atempo=0.5,atempo=0.500000");
    }

    #[test]
    fn voice_cleanup_scales_with_amount() {
        assert_eq!(enhance_chain(0.0), "");
        let full = enhance_chain(1.0);
        assert!(full.starts_with("highpass=f=80,afftdn=nf=-45.0:nr=24.0,deesser=i=0.50,"), "{full}");
        assert!(full.contains("acompressor=threshold=0.200:ratio=4.00"), "{full}");
        assert_eq!(voice_chain(1.0, 0.0), "");
        assert_eq!(voice_chain(2.0, 0.0), "atempo=2.000000");
        // The chain must be valid for libavfilter.
        let noise: Vec<f32> = (0..48_000).map(|i| ((i * 7919 % 1000) as f32 / 1000.0 - 0.5) * 0.2).collect();
        assert!(!vf_media::filter::apply(&voice_chain(1.5, 1.0), &noise).unwrap().is_empty());
    }
}
