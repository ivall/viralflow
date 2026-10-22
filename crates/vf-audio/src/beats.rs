//! Beats of the timeline's music, for effects that fire on them: the onsets of
//! kicks and snares, found in the mix of the audio clips (or of every clip
//! when there is no music) exactly as the export hears it. The music is
//! analysed once; picking beats from the analysis with other `BeatOptions` is instant.

use vf_core::model::{BeatFocus, BeatOptions, ItemKind, Project};
use vf_core::Result;
use vf_media::audio::{CHANNELS, SAMPLE_RATE};

use crate::mixer::{audible_clips, render_all, Mix};

/// Analysis step: 10 ms.
const HOP: usize = SAMPLE_RATE as usize / 100;
const HOP_SECONDS: f64 = HOP as f64 / SAMPLE_RATE as f64;
/// One-pole low-pass at about 200 Hz (at 48 kHz): kick, bass and snare body.
const LOW_COEFF: f32 = 0.0258;
/// Largest rise counted, in dB: a hit out of silence is not ten hits.
const MAX_RISE: f32 = 24.0;
/// How fast the remembered peak level falls, in dB a hop (50 dB a second).
const DECAY: f32 = 0.5;
/// Hops this far under the loudest one, or under `QUIET` dBFS, are silence, not music.
const SILENCE: f32 = 45.0;
const QUIET: f32 = -70.0;
/// Hops around a hit it is compared with (1.5 s each way).
const NEIGHBOURHOOD: usize = 150;

/// Analyses the music of the timeline. `progress` gets 0..1.
pub fn analyse(project: &Project, progress: &dyn Fn(f64), cancelled: &dyn Fn() -> bool) -> Result<Analysis> {
    let clips: Vec<_> = audible_clips(project).into_iter().filter(|clip| clip.path.is_file()).collect();
    let music: Vec<_> = clips.iter().filter(|clip| clip.item.kind == ItemKind::Audio).cloned().collect();
    let clips = if music.is_empty() { clips } else { music };
    let mut onsets = Analysis::default();
    let Some(end) = clips.iter().map(|clip| clip.end).max() else { return Ok(onsets) };
    let frames = end.max(0) as usize;
    let mut mix = Mix::new(clips);
    let finished = render_all(
        &mut mix,
        frames,
        |at, block| {
            onsets.push(block);
            progress(at as f64 / frames.max(1) as f64);
        },
        cancelled,
    );
    if !finished {
        return Err(vf_core::Error::Cancelled);
    }
    Ok(onsets)
}

/// Level per hop of two bands: the lows and a first difference that leans to the highs.
#[derive(Default)]
pub struct Analysis {
    low1: f32,
    low2: f32,
    previous: f32,
    low_sum: f64,
    high_sum: f64,
    count: usize,
    /// dB per hop.
    low: Vec<f32>,
    high: Vec<f32>,
}

fn db(energy: f64) -> f32 {
    (10.0 * (energy + 1e-10).log10()) as f32
}

/// Rise of each hop's level (dB) over the recent peak, which falls by `DECAY`
/// a hop: the wobble of a fading hit (a kick beating against the bass) is not
/// a new hit.
fn rises(levels: &[f32]) -> Vec<f32> {
    let floor = (levels.iter().copied().fold(f32::MIN, f32::max) - SILENCE).max(QUIET);
    let mut peaks = Vec::with_capacity(levels.len());
    let mut peak = f32::MIN;
    for level in levels {
        peak = level.max(peak - DECAY);
        peaks.push(peak);
    }
    (0..levels.len())
        .map(|t| {
            // Against two hops back: an attack rarely starts on a hop boundary.
            let before = if t >= 2 { peaks[t - 2] } else { levels[t] };
            if levels[t] < floor {
                0.0
            } else {
                (levels[t] - before.max(floor)).clamp(0.0, MAX_RISE)
            }
        })
        .collect()
}

impl Analysis {
    /// Interleaved stereo.
    fn push(&mut self, block: &[f32]) {
        for pair in block.chunks_exact(CHANNELS) {
            let x = (pair[0] + pair[1]) * 0.5;
            self.low1 += LOW_COEFF * (x - self.low1);
            self.low2 += LOW_COEFF * (self.low1 - self.low2);
            let high = x - self.previous;
            self.previous = x;
            self.low_sum += (self.low2 * self.low2) as f64;
            self.high_sum += (high * high) as f64;
            self.count += 1;
            if self.count == HOP {
                self.low.push(db(self.low_sum / HOP as f64));
                self.high.push(db(self.high_sum / HOP as f64));
                (self.low_sum, self.high_sum, self.count) = (0.0, 0.0, 0);
            }
        }
    }

    /// How sharply the sound rose at each hop; `high` weighs the highs (claps, hats) next to the lows.
    fn strength(&self, high: f32) -> Vec<f32> {
        let (lows, highs) = (rises(&self.low), rises(&self.high));
        lows.iter().zip(&highs).map(|(low, rise)| low + high * rise).collect()
    }

    /// Beats in timeline seconds, ascending: peaks of the strength that stand
    /// out from their neighbourhood. More `amount` lets weaker and closer hits in.
    pub fn beats(&self, options: &BeatOptions) -> Vec<f64> {
        let strength = self.strength(if options.focus == BeatFocus::All { 1.0 } else { 0.2 });
        let amount = options.amount() as f32;
        // A beat is at least this share of the strongest around it (0.5 by default)...
        let relative = 0.8 - 0.6 * amount;
        // ...and this many hops after the previous one (150 ms by default).
        let min_gap = (25.0 - 20.0 * amount).round() as usize;
        let n = strength.len();
        let mut picked: Vec<usize> = Vec::new();
        for t in 0..n {
            let value = strength[t];
            if value < 1.0 {
                continue;
            }
            let peak = (t.saturating_sub(4)..t).all(|j| strength[j] < value) && (t + 1..(t + 5).min(n)).all(|j| strength[j] <= value);
            let strongest = strength[t.saturating_sub(NEIGHBOURHOOD)..(t + NEIGHBOURHOOD).min(n)].iter().copied().fold(0.0, f32::max);
            if !peak || value < relative * strongest {
                continue;
            }
            match picked.last() {
                Some(&last) if t - last < min_gap => {
                    if value > strength[last] {
                        *picked.last_mut().expect("last") = t;
                    }
                }
                _ => picked.push(t),
            }
        }
        // The level rose across hops t-1..t: the hit starts at hop t-1.
        picked.into_iter().map(|t| (t.saturating_sub(1) as f64 * HOP_SECONDS * 1000.0).round() / 1000.0).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = SAMPLE_RATE as f64;

    /// Kicks every half second over a bass drone, hats between them, and hiss.
    fn groove(seconds: f64, first_kick: f64) -> Vec<f32> {
        let frames = (seconds * RATE) as usize;
        let mut noise = 0x2545_f491_u32;
        let mut out = Vec::with_capacity(frames * CHANNELS);
        for i in 0..frames {
            let t = i as f64 / RATE;
            noise ^= noise << 13;
            noise ^= noise >> 17;
            noise ^= noise << 5;
            let white = noise as f64 / u32::MAX as f64 * 2.0 - 1.0;
            let mut x = 0.15 * (std::f64::consts::TAU * 41.0 * t).sin() + 0.003 * white;
            let kick = (t - first_kick).rem_euclid(0.5);
            if t >= first_kick && kick < 0.3 {
                x += 0.8 * (std::f64::consts::TAU * 55.0 * kick).sin() * (-kick / 0.08).exp();
            }
            let hat = (t - first_kick - 0.25).rem_euclid(0.5);
            if t >= first_kick + 0.25 && hat < 0.04 {
                x += 0.15 * white * (-hat / 0.01).exp();
            }
            out.push(x as f32);
            out.push(x as f32);
        }
        out
    }

    fn analysis(samples: &[f32]) -> Analysis {
        let mut analysis = Analysis::default();
        // In blocks of the mixer's size.
        for block in samples.chunks(4800 * CHANNELS) {
            analysis.push(block);
        }
        analysis
    }

    #[test]
    fn finds_the_kicks_and_not_the_hats() {
        let beats = analysis(&groove(8.0, 0.3)).beats(&BeatOptions::default());
        let kicks: Vec<f64> = (0..16).map(|k| 0.3 + k as f64 * 0.5).filter(|t| *t < 8.0).collect();
        assert_eq!(beats.len(), kicks.len(), "{beats:?}");
        for (beat, kick) in beats.iter().zip(&kicks) {
            assert!((beat - kick).abs() <= 0.02, "beat {beat} for the kick at {kick}");
        }
    }

    #[test]
    fn every_hit_brings_the_hats_in() {
        let groove = analysis(&groove(8.0, 0.3));
        let all = groove.beats(&BeatOptions { focus: BeatFocus::All, amount: 0.5 });
        assert_eq!(all.len(), 31, "kicks and the hats between them: {all:?}");
        assert!(all.iter().any(|beat| (beat - 0.55).abs() <= 0.02));
        let fewer = groove.beats(&BeatOptions { focus: BeatFocus::All, amount: 0.0 });
        assert!(fewer.len() < all.len());
    }

    #[test]
    fn silence_has_no_beats() {
        assert!(analysis(&vec![0.0; 3 * SAMPLE_RATE as usize * CHANNELS]).beats(&BeatOptions::default()).is_empty());
    }
}
