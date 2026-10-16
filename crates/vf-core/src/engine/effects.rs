use super::color::{brightness_matrix, contrast_and_saturation, ColorMatrix, IDENTITY};
use super::math::{clamp, hash01};
use crate::model::{EffectKind, EffectRange, TimelineItem};

pub const GLITCH_BANDS: usize = 6;
pub const MIN_EFFECT: f64 = 0.1;

#[derive(Debug, Clone, PartialEq)]
pub struct EffectState {
    pub dx: f64,
    pub dy: f64,
    pub scale: f64,
    pub blur: f64,
    pub matrix: ColorMatrix,
    /// Film grain strength 0..1.
    pub grain: f64,
    pub scanlines: bool,
    /// Horizontal band displacement in % of the layer width, one per band (empty when idle).
    pub glitch: Vec<f64>,
}

impl Default for EffectState {
    fn default() -> Self {
        EffectState { dx: 0.0, dy: 0.0, scale: 1.0, blur: 0.0, matrix: IDENTITY, grain: 0.0, scanlines: false, glitch: vec![] }
    }
}

pub struct EffectPreset {
    pub kind: EffectKind,
    pub label: &'static str,
    pub description: &'static str,
}

pub const EFFECTS: &[EffectPreset] = &[
    EffectPreset { kind: EffectKind::None, label: "None", description: "No effect" },
    EffectPreset { kind: EffectKind::Punch, label: "Punch", description: "Quick scale hit at the start" },
    EffectPreset { kind: EffectKind::Shake, label: "Shake", description: "Decaying camera shake" },
    EffectPreset { kind: EffectKind::ZoomIn, label: "Push in", description: "Slow zoom in across the clip" },
    EffectPreset { kind: EffectKind::ZoomOut, label: "Pull out", description: "Slow zoom out across the clip" },
    EffectPreset { kind: EffectKind::Flash, label: "Flash", description: "Bright flash at the start" },
    EffectPreset { kind: EffectKind::Blur, label: "Blur", description: "Soft blur over the whole clip" },
    EffectPreset { kind: EffectKind::Vhs, label: "VHS", description: "Grain, scanlines and faded color" },
    EffectPreset { kind: EffectKind::Glitch, label: "Glitch", description: "Digital band displacement" },
];

pub fn resolve_effect(effect: EffectKind, elapsed: f64, duration: f64) -> EffectState {
    let idle = EffectState::default();
    let t = elapsed.max(0.0);
    match effect {
        EffectKind::None => idle,
        EffectKind::Punch => {
            let p = clamp(t / 0.32, 0.0, 1.0);
            if p >= 1.0 {
                return idle;
            }
            EffectState { scale: 1.0 + 0.12 * (1.0 - p).powi(2) * (p * std::f64::consts::PI * 2.0).cos(), ..idle }
        }
        EffectKind::Shake => {
            let p = clamp(t / 0.45, 0.0, 1.0);
            if p >= 1.0 {
                return idle;
            }
            let envelope = (1.0 - p).powi(2);
            EffectState { dx: (t * 78.0).sin() * 1.2 * envelope, dy: (t * 62.0).cos() * 0.7 * envelope, ..idle }
        }
        EffectKind::ZoomIn => EffectState { scale: 1.0 + 0.15 * clamp(t / duration.max(0.01), 0.0, 1.0), ..idle },
        EffectKind::ZoomOut => EffectState { scale: 1.15 - 0.15 * clamp(t / duration.max(0.01), 0.0, 1.0), ..idle },
        EffectKind::Flash => {
            let p = clamp(t / 0.35, 0.0, 1.0);
            if p >= 1.0 {
                return idle;
            }
            EffectState { matrix: brightness_matrix(0.6 * (1.0 - p).powi(2)), ..idle }
        }
        EffectKind::Blur => EffectState { blur: 10.0, ..idle },
        EffectKind::Vhs => EffectState { matrix: contrast_and_saturation(1.18, 0.78), grain: 0.09, scanlines: true, ..idle },
        EffectKind::Glitch => {
            let slot = (t * 8.0).floor() as i64;
            if hash01(slot) <= 0.6 {
                return idle;
            }
            let glitch = (0..GLITCH_BANDS).map(|band| (hash01(slot * 31 + band as i64) - 0.5) * 6.0).collect();
            EffectState { dx: (hash01(slot + 7) - 0.5) * 1.5, glitch, ..idle }
        }
    }
}

/// The span of the clip an effect covers, clamped to the clip.
pub fn effect_window(duration: f64, range: Option<EffectRange>) -> EffectRange {
    let Some(range) = range else { return EffectRange { start: 0.0, duration } };
    let start = clamp(if range.start.is_finite() { range.start } else { 0.0 }, 0.0, (duration - MIN_EFFECT).max(0.0));
    let length = clamp(if range.duration.is_finite() { range.duration } else { duration }, MIN_EFFECT.min(duration - start), duration - start);
    EffectRange { start, duration: length }
}

/// Effect state of a clip `elapsed` seconds after its start. A ranged effect is
/// idle outside its window, except where the window touches a clip edge.
pub fn resolve_item_effect(item: &TimelineItem, elapsed: f64) -> EffectState {
    let effect = item.effect();
    if effect == EffectKind::None {
        return EffectState::default();
    }
    if item.effect_range.is_none() {
        return resolve_effect(effect, elapsed, item.duration);
    }
    let window = effect_window(item.duration, item.effect_range);
    let local = elapsed - window.start;
    let touches_start = window.start <= 1e-6;
    let touches_end = window.start + window.duration >= item.duration - 1e-6;
    if (local < 0.0 && !touches_start) || (local > window.duration && !touches_end) {
        return EffectState::default();
    }
    resolve_effect(effect, local, window.duration)
}
