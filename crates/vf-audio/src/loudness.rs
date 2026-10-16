//! Integrated loudness (ITU-R BS.1770 / EBU R128) of a whole mix, and the
//! master stage both the preview and the export finish with: one static gain
//! to the loudness target, then a peak limiter.

use vf_media::audio::{CHANNELS, SAMPLE_RATE};

/// Peak the limiter holds when no loudness target is set.
const CEILING: f32 = 0.98;
/// True-peak ceiling of normalised mixes (-1.5 dBFS, like streaming platforms).
const NORMALISED_CEILING: f32 = 0.841;
/// Largest boost normalisation applies, so near-silent mixes don't turn into noise.
const MAX_BOOST_DB: f64 = 24.0;

#[derive(Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
}

impl Biquad {
    const fn new(b: [f64; 3], a: [f64; 2]) -> Self {
        Biquad { b, a, x: [0.0; 2], y: [0.0; 2] }
    }

    fn process(&mut self, input: f64) -> f64 {
        let output = self.b[0] * input + self.b[1] * self.x[0] + self.b[2] * self.x[1] - self.a[0] * self.y[0] - self.a[1] * self.y[1];
        self.x = [input, self.x[0]];
        self.y = [output, self.y[0]];
        output
    }
}

/// K-weighting at 48 kHz: high-shelf "head" filter, then the RLB high-pass.
fn k_weighting() -> [Biquad; 2] {
    [
        Biquad::new([1.53512485958697, -2.69169618940638, 1.19839281085285], [-1.69065929318241, 0.73248077421585]),
        Biquad::new([1.0, -2.0, 1.0], [-1.99004745483398, 0.99007225036621]),
    ]
}

/// Integrated loudness meter for interleaved 48 kHz stereo, fed in pieces.
pub struct Meter {
    filters: [[Biquad; 2]; 2],
    /// Mean square of each finished 100 ms step (both channels summed).
    steps: Vec<f64>,
    sum: f64,
    count: usize,
}

impl Default for Meter {
    fn default() -> Self {
        Meter { filters: [k_weighting(), k_weighting()], steps: Vec::new(), sum: 0.0, count: 0 }
    }
}

impl Meter {
    pub fn push(&mut self, samples: &[f32]) {
        let step = SAMPLE_RATE as usize / 10;
        for frame in samples.chunks_exact(CHANNELS) {
            for (channel, filter) in self.filters.iter_mut().enumerate() {
                let value = filter.iter_mut().fold(frame[channel] as f64, |v, f| f.process(v));
                self.sum += value * value;
            }
            self.count += 1;
            if self.count == step {
                self.steps.push(self.sum / step as f64);
                self.sum = 0.0;
                self.count = 0;
            }
        }
    }

    /// Loudness in LUFS, or None when everything is below the absolute gate (silence).
    pub fn integrated(&self) -> Option<f64> {
        // 400 ms blocks overlapping by 75 %.
        let blocks = self.steps.windows(4).map(|w| w.iter().sum::<f64>() / 4.0);
        let lufs = |power: f64| -0.691 + 10.0 * power.log10();
        let gated: Vec<f64> = blocks.filter(|&p| p > 0.0 && lufs(p) > -70.0).collect();
        if gated.is_empty() {
            return None;
        }
        let relative = lufs(gated.iter().sum::<f64>() / gated.len() as f64) - 10.0;
        let kept: Vec<f64> = gated.into_iter().filter(|&p| lufs(p) > relative).collect();
        Some(lufs(kept.iter().sum::<f64>() / kept.len() as f64))
    }
}

/// Integrated loudness in LUFS of a whole mix (see [`Meter`]).
pub fn integrated(samples: &[f32]) -> Option<f64> {
    let mut meter = Meter::default();
    meter.push(samples);
    meter.integrated()
}

/// The target the export settings ask for, if any.
pub fn target(loudness: Option<f64>) -> Option<f64> {
    loudness.filter(|v| v.is_finite() && *v < 0.0).map(|v| v.clamp(-30.0, -10.0))
}

/// Linear gain that brings `measured` LUFS to `target`.
pub fn normalising_gain(target: Option<f64>, measured: Option<f64>) -> f32 {
    match (target, measured) {
        (Some(target), Some(measured)) => 10f64.powf((target - measured).min(MAX_BOOST_DB) / 20.0) as f32,
        _ => 1.0,
    }
}

/// Final stage of every mix: static gain, then a peak limiter with instant
/// attack (nothing passes the ceiling) and a smooth release.
#[derive(Clone)]
pub struct Master {
    gain: f32,
    ceiling: f32,
    envelope: f32,
    release: f32,
}

impl Master {
    pub fn new(normalised: bool, gain: f32) -> Self {
        // About 80 ms to recover after a peak.
        let release = 1.0 - (-1.0 / (0.08 * SAMPLE_RATE as f32)).exp();
        Master { gain, ceiling: if normalised { NORMALISED_CEILING } else { CEILING }, envelope: 1.0, release }
    }

    pub fn is_normalised(&self) -> bool {
        self.ceiling == NORMALISED_CEILING
    }

    pub fn reset(&mut self) {
        self.envelope = 1.0;
    }

    /// Processes interleaved stereo in place.
    pub fn process(&mut self, samples: &mut [f32]) {
        for frame in samples.chunks_exact_mut(CHANNELS) {
            let left = frame[0] * self.gain;
            let right = frame[1] * self.gain;
            let peak = left.abs().max(right.abs());
            let wanted = if peak > self.ceiling { self.ceiling / peak } else { 1.0 };
            self.envelope = if wanted < self.envelope { wanted } else { self.envelope + (wanted - self.envelope) * self.release };
            frame[0] = (left * self.envelope).clamp(-1.0, 1.0);
            frame[1] = (right * self.envelope).clamp(-1.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(seconds: f64, frequency: f64, amplitude: f64) -> Vec<f32> {
        let frames = (seconds * SAMPLE_RATE as f64) as usize;
        (0..frames)
            .flat_map(|i| {
                let v = ((i as f64 * frequency * std::f64::consts::TAU / SAMPLE_RATE as f64).sin() * amplitude) as f32;
                [v, v]
            })
            .collect()
    }

    #[test]
    fn full_scale_stereo_sine_reads_zero_lufs() {
        // BS.1770: a 997 Hz full-scale sine in one channel is -3.01 LUFS, so
        // in both channels it is about 0.
        let loud = integrated(&sine(3.0, 997.0, 1.0)).unwrap();
        assert!((loud - 0.0).abs() < 0.1, "{loud}");
        let quiet = integrated(&sine(3.0, 997.0, 0.1)).unwrap();
        assert!((quiet + 20.0).abs() < 0.1, "{quiet}");
    }

    #[test]
    fn silence_is_gated_and_does_not_dilute_loudness() {
        assert_eq!(integrated(&vec![0.0; 48_000 * 2 * 3]), None);
        let mut mixed = sine(3.0, 997.0, 0.1);
        mixed.extend(vec![0.0f32; 48_000 * 2 * 6]);
        // Blocks across the edge count partly, as BS.1770 says.
        let loud = integrated(&mixed).unwrap();
        assert!((loud + 20.0).abs() < 0.3, "{loud}");
    }

    #[test]
    fn gain_reaches_the_target_and_limiter_holds_the_ceiling() {
        let gain = normalising_gain(Some(-14.0), Some(-20.0));
        assert!((gain - 10f32.powf(6.0 / 20.0)).abs() < 1e-4, "{gain}");
        assert_eq!(normalising_gain(None, Some(-20.0)), 1.0);
        assert_eq!(normalising_gain(Some(-14.0), None), 1.0);
        let mut samples = sine(1.0, 200.0, 0.9);
        let mut master = Master::new(true, 2.0);
        master.process(&mut samples);
        let peak = samples.iter().fold(0.0f32, |a, b| a.max(b.abs()));
        assert!(peak <= NORMALISED_CEILING + 1e-6 && peak > 0.8, "{peak}");
    }
}
