//! Clip effects: what each one does to a layer at a moment, at a strength.
//! A clip carries a stack of them; each covers its own part of the clip and
//! their results add up (`EffectState::with`).

use std::f64::consts::{PI, TAU};

use super::color::{
    brightness_matrix, contrast_and_saturation, contrast_matrix, hue_matrix, invert_matrix, multiply, saturate_matrix, sepia_matrix, ColorMatrix, IDENTITY,
};
use super::fx::LayerFx;
use super::math::{clamp, hash01};
use crate::model::{ClipEffect, EffectKind, EffectRange, TimelineItem, DEFAULT_EFFECT_AMOUNT};

pub const GLITCH_BANDS: usize = 6;
pub const MIN_EFFECT: f64 = 0.1;

#[derive(Debug, Clone, PartialEq)]
pub struct EffectState {
    pub dx: f64,
    pub dy: f64,
    pub scale: f64,
    /// Degrees, clockwise.
    pub rotation: f64,
    pub blur: f64,
    pub matrix: ColorMatrix,
    /// Film grain strength 0..1.
    pub grain: f64,
    pub scanlines: bool,
    /// Horizontal band displacement in % of the layer width, one per band (empty when idle).
    pub glitch: Vec<f64>,
    pub vignette: f64,
    pub fx: LayerFx,
}

impl Default for EffectState {
    fn default() -> Self {
        EffectState {
            dx: 0.0,
            dy: 0.0,
            scale: 1.0,
            rotation: 0.0,
            blur: 0.0,
            matrix: IDENTITY,
            grain: 0.0,
            scanlines: false,
            glitch: vec![],
            vignette: 0.0,
            fx: LayerFx::default(),
        }
    }
}

impl EffectState {
    /// `self`, then `next` on top of it.
    pub fn with(self, next: EffectState) -> EffectState {
        EffectState {
            dx: self.dx + next.dx,
            dy: self.dy + next.dy,
            scale: self.scale * next.scale,
            rotation: self.rotation + next.rotation,
            blur: self.blur.max(next.blur),
            matrix: multiply(&next.matrix, &self.matrix),
            grain: (self.grain + next.grain).min(1.0),
            scanlines: self.scanlines || next.scanlines,
            glitch: if next.glitch.is_empty() { self.glitch } else { next.glitch },
            vignette: self.vignette.max(next.vignette),
            fx: self.fx.with(next.fx),
        }
    }
}

pub struct EffectPreset {
    pub kind: EffectKind,
    pub label: &'static str,
    pub description: &'static str,
    /// Whether the strength changes anything (Mirror is on or off).
    pub adjustable: bool,
}

const fn preset(kind: EffectKind, label: &'static str, description: &'static str) -> EffectPreset {
    EffectPreset { kind, label, description, adjustable: true }
}

/// Every effect, in the order the effects panel shows them: motion, then
/// distortions, then looks, then light.
pub const EFFECTS: &[EffectPreset] = &[
    preset(EffectKind::Punch, "Punch", "Quick scale hit at the start"),
    preset(EffectKind::Shake, "Shake", "Decaying camera shake"),
    preset(EffectKind::ZoomIn, "Push in", "Slow zoom in across the clip"),
    preset(EffectKind::ZoomOut, "Pull out", "Slow zoom out across the clip"),
    preset(EffectKind::Pulse, "Pulse", "Beats in and out like a heartbeat"),
    preset(EffectKind::Sway, "Sway", "Gentle rocking camera"),
    preset(EffectKind::Glitch, "Glitch", "Digital band displacement"),
    preset(EffectKind::RgbSplit, "RGB split", "Red and blue drift apart"),
    preset(EffectKind::Pixelate, "Pixelate", "Big square pixels"),
    preset(EffectKind::ZoomBlur, "Zoom blur", "Speed blur towards the centre"),
    preset(EffectKind::Fisheye, "Fisheye", "Bulging wide-angle lens"),
    EffectPreset { kind: EffectKind::Mirror, label: "Mirror", description: "Left half mirrored onto the right", adjustable: false },
    preset(EffectKind::Blur, "Blur", "Soft blur over the whole clip"),
    preset(EffectKind::Sharpen, "Sharpen", "Crisper edges and detail"),
    preset(EffectKind::Vhs, "VHS", "Grain, scanlines and faded color"),
    preset(EffectKind::Film, "Film", "Grain and a soft filmic fade"),
    preset(EffectKind::Vintage, "Vintage", "Warm sepia with dark corners"),
    preset(EffectKind::Mono, "Black & white", "Color drained to grey"),
    preset(EffectKind::Negative, "Negative", "Inverted colors"),
    preset(EffectKind::Rainbow, "Rainbow", "Colors cycle through the spectrum"),
    preset(EffectKind::Posterize, "Posterize", "Flat bands of color"),
    preset(EffectKind::Vignette, "Vignette", "Darker corners that frame the subject"),
    preset(EffectKind::Letterbox, "Cinema bars", "Black bars top and bottom"),
    preset(EffectKind::Flash, "Flash", "Bright flash at the start"),
    preset(EffectKind::Strobe, "Strobe", "Rapid flashes of light"),
    preset(EffectKind::LightLeak, "Light leak", "Warm light drifting across"),
];

pub fn effect_preset(kind: EffectKind) -> Option<&'static EffectPreset> {
    EFFECTS.iter().find(|p| p.kind == kind)
}

pub fn effect_label(kind: EffectKind) -> &'static str {
    effect_preset(kind).map(|p| p.label).unwrap_or("Effect")
}

/// The effect `elapsed` seconds into a window of `duration` seconds, at
/// strength `amount` (0..1; `DEFAULT_EFFECT_AMOUNT` is each effect's designed look).
pub fn resolve_effect(effect: EffectKind, elapsed: f64, duration: f64, amount: f64) -> EffectState {
    let idle = EffectState::default();
    let t = elapsed.max(0.0);
    let a = clamp(amount, 0.0, 1.0);
    // 1 at the default strength, 2 at full.
    let f = a / DEFAULT_EFFECT_AMOUNT;
    let progress = clamp(t / duration.max(0.01), 0.0, 1.0);
    match effect {
        EffectKind::None => idle,
        EffectKind::Punch => {
            let p = clamp(t / 0.32, 0.0, 1.0);
            if p >= 1.0 {
                return idle;
            }
            EffectState { scale: 1.0 + 0.12 * f * (1.0 - p).powi(2) * (p * TAU).cos(), ..idle }
        }
        EffectKind::Shake => {
            let p = clamp(t / 0.45, 0.0, 1.0);
            if p >= 1.0 {
                return idle;
            }
            let envelope = (1.0 - p).powi(2) * f;
            EffectState { dx: (t * 78.0).sin() * 1.2 * envelope, dy: (t * 62.0).cos() * 0.7 * envelope, ..idle }
        }
        EffectKind::ZoomIn => EffectState { scale: 1.0 + 0.15 * f * progress, ..idle },
        EffectKind::ZoomOut => EffectState { scale: 1.0 + 0.15 * f * (1.0 - progress), ..idle },
        EffectKind::Flash => {
            let p = clamp(t / 0.35, 0.0, 1.0);
            if p >= 1.0 {
                return idle;
            }
            EffectState { matrix: brightness_matrix(0.6 * f * (1.0 - p).powi(2)), ..idle }
        }
        EffectKind::Blur => EffectState { blur: 10.0 * f, ..idle },
        EffectKind::Vhs => EffectState {
            matrix: contrast_and_saturation(1.0 + 0.18 * f, (1.0 - 0.22 * f).max(0.0)),
            grain: 0.09 * f,
            scanlines: true,
            fx: LayerFx { chroma: 0.002 * f, ..Default::default() },
            ..idle
        },
        EffectKind::Glitch => {
            let slot = (t * 8.0).floor() as i64;
            if hash01(slot) <= 0.6 {
                return idle;
            }
            let glitch = (0..GLITCH_BANDS).map(|band| (hash01(slot * 31 + band as i64) - 0.5) * 6.0 * f).collect();
            EffectState { dx: (hash01(slot + 7) - 0.5) * 1.5 * f, glitch, fx: LayerFx { chroma: 0.012 * f, ..Default::default() }, ..idle }
        }
        EffectKind::Pulse => {
            // Two beats a second, each a quick swell that settles.
            let beat = (t % 0.5) / 0.5;
            EffectState { scale: 1.0 + 0.07 * f * (-beat * 9.0).exp(), ..idle }
        }
        EffectKind::Sway => EffectState {
            rotation: 3.0 * f * (t * TAU / 1.8).sin(),
            dx: 0.8 * f * (t * TAU / 3.6).sin(),
            // Enough zoom that the rocking never shows the corners.
            scale: 1.0 + 0.07 * f,
            ..idle
        },
        EffectKind::Strobe => {
            let on = (t * 8.0).fract() < 0.35;
            if on {
                EffectState { matrix: brightness_matrix(0.45 * f), ..idle }
            } else {
                idle
            }
        }
        EffectKind::RgbSplit => EffectState { fx: LayerFx { chroma: 0.006 * f * (1.0 + 0.3 * (t * 9.0).sin()), ..Default::default() }, ..idle },
        EffectKind::Pixelate => EffectState { fx: LayerFx { pixelate: 4.0 + 44.0 * a, ..Default::default() }, ..idle },
        EffectKind::ZoomBlur => EffectState { fx: LayerFx { zoom_blur: 0.35 * f, ..Default::default() }, ..idle },
        EffectKind::Fisheye => EffectState { fx: LayerFx { bulge: 0.45 * f, ..Default::default() }, ..idle },
        EffectKind::Mirror => EffectState { fx: LayerFx { mirror: true, ..Default::default() }, ..idle },
        EffectKind::Mono => EffectState { matrix: multiply(&contrast_matrix(1.0 + 0.08 * f), &saturate_matrix((1.0 - f).max(0.0))), ..idle },
        EffectKind::Vintage => EffectState {
            matrix: multiply(&contrast_matrix(1.0 - 0.06 * f), &sepia_matrix(0.8 * f)),
            vignette: clamp(0.35 * f, 0.0, 1.0),
            grain: 0.04 * f,
            ..idle
        },
        EffectKind::Film => EffectState {
            matrix: multiply(&brightness_matrix(0.02 * f), &contrast_and_saturation(1.0 - 0.06 * f, (1.0 - 0.1 * f).max(0.0))),
            grain: 0.1 * f,
            vignette: clamp(0.2 * f, 0.0, 1.0),
            ..idle
        },
        EffectKind::Negative => EffectState { matrix: invert_matrix(f), ..idle },
        EffectKind::Rainbow => EffectState { matrix: hue_matrix(t * 180.0 * f), ..idle },
        EffectKind::Posterize => EffectState { fx: LayerFx { posterize: (10.0 - 7.0 * a).round().max(2.0), ..Default::default() }, ..idle },
        EffectKind::Sharpen => EffectState { fx: LayerFx { sharpen: 0.75 * f, ..Default::default() }, ..idle },
        EffectKind::Vignette => EffectState { vignette: clamp(0.5 * f, 0.0, 1.0), ..idle },
        EffectKind::Letterbox => {
            let bar = 0.04 + 0.12 * a;
            EffectState { fx: LayerFx { clip_top: bar, clip_bottom: bar, ..Default::default() }, ..idle }
        }
        EffectKind::LightLeak => EffectState {
            fx: LayerFx { leak: clamp(0.75 * f * (0.65 + 0.35 * (t * 1.7).sin()), 0.0, 1.0), leak_phase: (t / 3.0).fract(), ..Default::default() },
            ..idle
        },
    }
}

/// The span of the clip an effect covers, clamped to the clip.
pub fn effect_window(duration: f64, range: Option<EffectRange>) -> EffectRange {
    let Some(range) = range else { return EffectRange { start: 0.0, duration } };
    let start = clamp(if range.start.is_finite() { range.start } else { 0.0 }, 0.0, (duration - MIN_EFFECT).max(0.0));
    let length = clamp(if range.duration.is_finite() { range.duration } else { duration }, MIN_EFFECT.min(duration - start), duration - start);
    EffectRange { start, duration: length }
}

/// One effect of a clip `elapsed` seconds after the clip's start. A ranged
/// effect is idle outside its window, except where the window touches a clip edge.
pub fn resolve_clip_effect(effect: &ClipEffect, item_duration: f64, elapsed: f64) -> EffectState {
    if effect.kind == EffectKind::None {
        return EffectState::default();
    }
    if effect.range.is_none() {
        return resolve_effect(effect.kind, elapsed, item_duration, effect.amount());
    }
    let window = effect_window(item_duration, effect.range);
    let local = elapsed - window.start;
    let touches_start = window.start <= 1e-6;
    let touches_end = window.start + window.duration >= item_duration - 1e-6;
    if (local < 0.0 && !touches_start) || (local > window.duration && !touches_end) {
        return EffectState::default();
    }
    resolve_effect(effect.kind, local, window.duration, effect.amount())
}

/// All of a clip's effects `elapsed` seconds after its start, in stack order.
pub fn resolve_item_effect(item: &TimelineItem, elapsed: f64) -> EffectState {
    item.active_effects().fold(EffectState::default(), |state, effect| state.with(resolve_clip_effect(effect, item.duration, elapsed)))
}

/// Whether an effect still moves (time matters) or is a fixed look.
pub fn is_animated(kind: EffectKind) -> bool {
    !matches!(
        kind,
        EffectKind::None
            | EffectKind::Blur
            | EffectKind::Pixelate
            | EffectKind::ZoomBlur
            | EffectKind::Fisheye
            | EffectKind::Mirror
            | EffectKind::Mono
            | EffectKind::Vintage
            | EffectKind::Negative
            | EffectKind::Posterize
            | EffectKind::Sharpen
            | EffectKind::Vignette
            | EffectKind::Letterbox
    )
}

/// Half a turn: used by transitions that brighten through their middle.
pub(crate) fn hump(p: f64) -> f64 {
    (clamp(p, 0.0, 1.0) * PI).sin()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ItemKind;

    #[test]
    fn every_effect_is_listed_once() {
        for kind in EffectKind::ALL.iter().filter(|k| **k != EffectKind::None) {
            assert_eq!(EFFECTS.iter().filter(|p| p.kind == *kind).count(), 1, "{kind}");
        }
    }

    #[test]
    fn default_strength_is_the_designed_look() {
        let state = resolve_effect(EffectKind::Blur, 0.0, 2.0, DEFAULT_EFFECT_AMOUNT);
        assert_eq!(state.blur, 10.0);
        let stronger = resolve_effect(EffectKind::Blur, 0.0, 2.0, 1.0);
        assert_eq!(stronger.blur, 20.0);
        assert_eq!(resolve_effect(EffectKind::Blur, 0.0, 2.0, 0.0).blur, 0.0);
        let zoom = resolve_effect(EffectKind::ZoomOut, 0.0, 2.0, DEFAULT_EFFECT_AMOUNT);
        assert!((zoom.scale - 1.15).abs() < 1e-12);
    }

    #[test]
    fn stacked_effects_add_up_in_order() {
        let item = TimelineItem {
            kind: ItemKind::Video,
            duration: 4.0,
            effects: vec![
                ClipEffect { id: "a".into(), kind: EffectKind::ZoomIn, ..Default::default() },
                ClipEffect { id: "b".into(), kind: EffectKind::Blur, range: Some(EffectRange { start: 1.0, duration: 1.0 }), ..Default::default() },
                ClipEffect { id: "c".into(), kind: EffectKind::Mono, ..Default::default() },
            ],
            ..Default::default()
        };
        let early = resolve_item_effect(&item, 0.5);
        assert_eq!(early.blur, 0.0, "blur waits for its window");
        assert!(early.scale > 1.0);
        let middle = resolve_item_effect(&item, 1.5);
        assert_eq!(middle.blur, 10.0);
        // Black & white drains the saturation: equal weights per row.
        assert!((middle.matrix[0] - middle.matrix[1] * (0.213 / 0.715)).abs() < 0.05);
    }

    #[test]
    fn a_ranged_effect_is_idle_outside_its_window() {
        let effect = ClipEffect { kind: EffectKind::Blur, range: Some(EffectRange { start: 1.0, duration: 1.0 }), ..Default::default() };
        assert_eq!(resolve_clip_effect(&effect, 4.0, 0.5).blur, 0.0);
        assert_eq!(resolve_clip_effect(&effect, 4.0, 1.5).blur, 10.0);
        assert_eq!(resolve_clip_effect(&effect, 4.0, 2.5).blur, 0.0);
        // A window that reaches the end of the clip stays on to the end.
        let tail = ClipEffect { range: Some(EffectRange { start: 3.0, duration: 1.0 }), ..effect };
        assert_eq!(resolve_clip_effect(&tail, 4.0, 4.0).blur, 10.0);
    }

    #[test]
    fn looks_do_not_move_and_motion_does() {
        assert!(!is_animated(EffectKind::Mono));
        assert!(is_animated(EffectKind::Pulse));
        let a = resolve_effect(EffectKind::Pixelate, 0.1, 2.0, 0.5);
        let b = resolve_effect(EffectKind::Pixelate, 1.7, 2.0, 0.5);
        assert_eq!(a, b);
    }
}
