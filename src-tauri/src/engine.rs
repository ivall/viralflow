//! Rust mirror of src/lib/engine/*.ts. Every formula here must match its
//! TypeScript twin exactly so the exported file looks like the preview.

use crate::model::{Adjust, Crop, Keyframe, MediaAsset, Point, Project, TimelineItem, Track, Transform};

pub const EPSILON: f64 = 1e-4;

// ---------------------------------------------------------------- math.ts

pub fn clamp(value: f64, min: f64, max: f64) -> f64 {
    value.max(min).min(max)
}

pub fn lerp(from: f64, to: f64, progress: f64) -> f64 {
    from + (to - from) * progress
}

pub fn ease_out_cubic(value: f64) -> f64 {
    1.0 - (1.0 - clamp(value, 0.0, 1.0)).powi(3)
}

pub fn ease_in_cubic(value: f64) -> f64 {
    clamp(value, 0.0, 1.0).powi(3)
}

pub fn ease_in_out_cubic(value: f64) -> f64 {
    let v = clamp(value, 0.0, 1.0);
    if v < 0.5 {
        4.0 * v * v * v
    } else {
        1.0 - (-2.0 * v + 2.0).powi(3) / 2.0
    }
}

pub fn apply_easing(progress: f64, easing: &str) -> f64 {
    let v = clamp(progress, 0.0, 1.0);
    match easing {
        "ease-in" => v * v,
        "ease-out" => 1.0 - (1.0 - v) * (1.0 - v),
        "ease-in-out" => {
            if v < 0.5 {
                2.0 * v * v
            } else {
                1.0 - (-2.0 * v + 2.0).powi(2) / 2.0
            }
        }
        _ => v,
    }
}

/// Mirror of `hash01` in math.ts (Math.imul based integer hash).
pub fn hash01(seed: i64) -> f64 {
    let seed = seed as i32 as u32;
    let mut x = (seed ^ 0x9e37_79b9).wrapping_mul(0x85eb_ca6b);
    x = (x ^ (x >> 13)).wrapping_mul(0xc2b2_ae35);
    x ^= x >> 16;
    x as f64 / 4_294_967_296.0
}

// ----------------------------------------------------------- keyframes.ts

pub fn normalize_transform(value: Option<&Transform>) -> Transform {
    let v = value.copied().unwrap_or_default();
    let finite = |value: f64, fallback: f64| if value.is_finite() { value } else { fallback };
    let scale = if v.scale > 0.0 { v.scale } else { 1.0 };
    Transform {
        x: clamp(finite(v.x, 50.0), -100.0, 200.0),
        y: clamp(finite(v.y, 50.0), -100.0, 200.0),
        scale: clamp(scale, 0.05, 10.0),
        rotation: finite(v.rotation, 0.0),
        opacity: clamp(finite(v.opacity, 1.0), 0.0, 1.0),
    }
}

fn sorted_keyframes(item: &TimelineItem) -> Vec<Keyframe> {
    let duration = item.duration.max(0.0);
    let mut list: Vec<Keyframe> = item
        .keyframes
        .iter()
        .filter(|keyframe| keyframe.time.is_finite())
        .map(|keyframe| Keyframe { time: clamp(keyframe.time, 0.0, duration), ..keyframe.clone() })
        .collect();
    list.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.id.cmp(&b.id)));
    let mut result: Vec<Keyframe> = Vec::with_capacity(list.len());
    for index in 0..list.len() {
        if index == list.len() - 1 || (list[index + 1].time - list[index].time).abs() > EPSILON {
            result.push(list[index].clone());
        }
    }
    result
}

fn interpolate(left: Transform, right: Transform, progress: f64, easing: &str) -> Transform {
    let t = apply_easing(progress, easing);
    Transform {
        x: lerp(left.x, right.x, t),
        y: lerp(left.y, right.y, t),
        scale: lerp(left.scale, right.scale, t),
        rotation: lerp(left.rotation, right.rotation, t),
        opacity: lerp(left.opacity, right.opacity, t),
    }
}

pub fn resolve_transform(item: &TimelineItem, elapsed: f64) -> Transform {
    let base = normalize_transform(item.transform.as_ref());
    let keyframes = sorted_keyframes(item);
    if keyframes.is_empty() {
        return base;
    }
    let time = clamp(elapsed, 0.0, item.duration.max(0.0));
    let first = &keyframes[0];
    if time <= first.time {
        return normalize_transform(Some(&first.transform));
    }
    for index in 1..keyframes.len() {
        let right = &keyframes[index];
        if time > right.time {
            continue;
        }
        let left = &keyframes[index - 1];
        let span = (right.time - left.time).max(EPSILON);
        return interpolate(normalize_transform(Some(&left.transform)), normalize_transform(Some(&right.transform)), (time - left.time) / span, &right.easing);
    }
    normalize_transform(Some(&keyframes[keyframes.len() - 1].transform))
}

// ----------------------------------------------------------- animation.ts

#[derive(Debug, Clone, Copy)]
pub struct Motion {
    pub dx: f64,
    pub dy: f64,
    pub scale: f64,
    pub rotation: f64,
    pub opacity: f64,
    pub blur: f64,
    pub reveal: f64,
}

pub const IDLE_MOTION: Motion = Motion { dx: 0.0, dy: 0.0, scale: 1.0, rotation: 0.0, opacity: 1.0, blur: 0.0, reveal: 1.0 };

fn combine(a: Motion, b: Motion) -> Motion {
    Motion {
        dx: a.dx + b.dx,
        dy: a.dy + b.dy,
        scale: a.scale * b.scale,
        rotation: a.rotation + b.rotation,
        opacity: a.opacity * b.opacity,
        blur: a.blur + b.blur,
        reveal: a.reveal.min(b.reveal),
    }
}

fn in_state(kind: &str, progress: f64) -> Motion {
    let p = clamp(progress, 0.0, 1.0);
    let e = ease_out_cubic(p);
    let appear = clamp(p / 0.25, 0.0, 1.0);
    let mut s = IDLE_MOTION;
    match kind {
        "fade" => s.opacity = e,
        "pop" => {
            s.opacity = appear;
            s.scale = if p < 0.6 { lerp(0.6, 1.08, ease_out_cubic(p / 0.6)) } else { lerp(1.08, 1.0, ease_out_cubic((p - 0.6) / 0.4)) };
        }
        "rise" => {
            s.opacity = appear;
            s.dy = (1.0 - e) * 10.0;
        }
        "drop" => {
            s.opacity = appear;
            s.dy = -(1.0 - e) * 10.0;
        }
        "slide-left" => {
            s.opacity = appear;
            s.dx = (1.0 - e) * 35.0;
        }
        "slide-right" => {
            s.opacity = appear;
            s.dx = -(1.0 - e) * 35.0;
        }
        "zoom-in" => {
            s.opacity = e;
            s.scale = lerp(0.4, 1.0, e);
        }
        "zoom-out" => {
            s.opacity = e;
            s.scale = lerp(1.8, 1.0, e);
        }
        "bounce" => {
            s.opacity = appear;
            s.scale = if p < 0.45 {
                lerp(0.5, 1.16, ease_out_cubic(p / 0.45))
            } else if p < 0.75 {
                lerp(1.16, 0.95, ease_out_cubic((p - 0.45) / 0.3))
            } else {
                lerp(0.95, 1.0, ease_out_cubic((p - 0.75) / 0.25))
            };
        }
        "spin" => {
            s.opacity = e;
            s.scale = lerp(0.3, 1.0, e);
            s.rotation = (1.0 - e) * -180.0;
        }
        "blur" => {
            s.opacity = e;
            s.blur = (1.0 - e) * 24.0;
        }
        "typewriter" => s.reveal = p,
        _ => {}
    }
    s
}

fn out_state(kind: &str, progress: f64) -> Motion {
    let q = ease_in_cubic(progress);
    let fade = 1.0 - clamp((progress - 0.6) / 0.4, 0.0, 1.0);
    let mut s = IDLE_MOTION;
    match kind {
        "fade" => s.opacity = 1.0 - q,
        "shrink" => {
            s.opacity = 1.0 - q;
            s.scale = lerp(1.0, 0.3, q);
        }
        "grow" => {
            s.opacity = 1.0 - q;
            s.scale = lerp(1.0, 1.8, q);
        }
        "sink" => {
            s.opacity = fade;
            s.dy = q * 10.0;
        }
        "lift" => {
            s.opacity = fade;
            s.dy = -q * 10.0;
        }
        "slide-left" => {
            s.opacity = fade;
            s.dx = -q * 35.0;
        }
        "slide-right" => {
            s.opacity = fade;
            s.dx = q * 35.0;
        }
        "spin" => {
            s.opacity = 1.0 - q;
            s.scale = lerp(1.0, 0.3, q);
            s.rotation = q * 180.0;
        }
        "blur" => {
            s.opacity = 1.0 - q;
            s.blur = q * 24.0;
        }
        _ => {}
    }
    s
}

pub fn resolve_motion(item: &TimelineItem, elapsed: f64) -> Motion {
    let in_kind = item.anim_in.as_ref().map(|a| a.kind.as_str()).unwrap_or("none");
    let out_kind = item.anim_out.as_ref().map(|a| a.kind.as_str()).unwrap_or("none");
    let mut in_duration = if in_kind == "none" { 0.0 } else { item.anim_in.as_ref().map(|a| a.duration).unwrap_or(0.0).max(0.05) };
    let mut out_duration = if out_kind == "none" { 0.0 } else { item.anim_out.as_ref().map(|a| a.duration).unwrap_or(0.0).max(0.05) };
    let total = in_duration + out_duration;
    if total > item.duration && total > 0.0 {
        let ratio = item.duration / total;
        in_duration *= ratio;
        out_duration *= ratio;
    }
    let mut state = IDLE_MOTION;
    if in_kind != "none" && elapsed < in_duration {
        state = combine(state, in_state(in_kind, elapsed / in_duration));
    }
    let out_start = item.duration - out_duration;
    if out_kind != "none" && elapsed > out_start {
        state = combine(state, out_state(out_kind, (elapsed - out_start) / out_duration));
    }
    state
}

// --------------------------------------------------------------- color.ts

pub type ColorMatrix = [f64; 20];

pub const IDENTITY: ColorMatrix = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];

pub fn multiply(a: &ColorMatrix, b: &ColorMatrix) -> ColorMatrix {
    let mut out = [0.0; 20];
    for row in 0..4 {
        for col in 0..5 {
            let mut value = if col == 4 { a[row * 5 + 4] } else { 0.0 };
            for k in 0..4 {
                value += a[row * 5 + k] * b[k * 5 + col];
            }
            out[row * 5 + col] = value;
        }
    }
    out
}

pub fn is_identity(matrix: &ColorMatrix) -> bool {
    matrix.iter().zip(IDENTITY.iter()).all(|(a, b)| (a - b).abs() < 1e-6)
}

fn scale_rgb(r: f64, g: f64, b: f64) -> ColorMatrix {
    [r, 0.0, 0.0, 0.0, 0.0, 0.0, g, 0.0, 0.0, 0.0, 0.0, 0.0, b, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]
}

fn offset_rgb(v: f64) -> ColorMatrix {
    [1.0, 0.0, 0.0, 0.0, v, 0.0, 1.0, 0.0, 0.0, v, 0.0, 0.0, 1.0, 0.0, v, 0.0, 0.0, 0.0, 1.0, 0.0]
}

fn contrast_matrix(k: f64) -> ColorMatrix {
    let o = 0.5 * (1.0 - k);
    [k, 0.0, 0.0, 0.0, o, 0.0, k, 0.0, 0.0, o, 0.0, 0.0, k, 0.0, o, 0.0, 0.0, 0.0, 1.0, 0.0]
}

fn saturate_matrix(s: f64) -> ColorMatrix {
    [
        0.213 + 0.787 * s,
        0.715 - 0.715 * s,
        0.072 - 0.072 * s,
        0.0,
        0.0,
        0.213 - 0.213 * s,
        0.715 + 0.285 * s,
        0.072 - 0.072 * s,
        0.0,
        0.0,
        0.213 - 0.213 * s,
        0.715 - 0.715 * s,
        0.072 + 0.928 * s,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
    ]
}

fn hue_matrix(degrees: f64) -> ColorMatrix {
    let angle = degrees.to_radians();
    let (s, c) = angle.sin_cos();
    [
        0.213 + c * 0.787 - s * 0.213,
        0.715 - c * 0.715 - s * 0.715,
        0.072 - c * 0.072 + s * 0.928,
        0.0,
        0.0,
        0.213 - c * 0.213 + s * 0.143,
        0.715 + c * 0.285 + s * 0.14,
        0.072 - c * 0.072 - s * 0.283,
        0.0,
        0.0,
        0.213 - c * 0.213 - s * 0.787,
        0.715 - c * 0.715 + s * 0.715,
        0.072 + c * 0.928 + s * 0.072,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
    ]
}

pub fn mix_matrix(color: [f64; 3], amount: f64) -> ColorMatrix {
    let a = clamp(amount, 0.0, 1.0);
    let k = 1.0 - a;
    [k, 0.0, 0.0, 0.0, color[0] * a, 0.0, k, 0.0, 0.0, color[1] * a, 0.0, 0.0, k, 0.0, color[2] * a, 0.0, 0.0, 0.0, 1.0, 0.0]
}

pub fn brightness_matrix(offset: f64) -> ColorMatrix {
    offset_rgb(offset)
}

pub fn adjust_matrix(adjust: &Adjust) -> ColorMatrix {
    let a = adjust;
    let mut m = IDENTITY;
    if a.exposure != 0.0 {
        let k = 2f64.powf(a.exposure * 1.5);
        m = multiply(&scale_rgb(k, k, k), &m);
    }
    if a.brightness != 0.0 {
        m = multiply(&offset_rgb(a.brightness * 0.3), &m);
    }
    if a.contrast != 0.0 {
        let k = if a.contrast >= 0.0 { 1.0 + a.contrast * 1.5 } else { 1.0 + a.contrast };
        m = multiply(&contrast_matrix(k), &m);
    }
    if a.saturation != 0.0 {
        m = multiply(&saturate_matrix((1.0 + a.saturation).max(0.0)), &m);
    }
    if a.hue != 0.0 {
        m = multiply(&hue_matrix(a.hue * 180.0), &m);
    }
    if a.temperature != 0.0 {
        m = multiply(&scale_rgb(1.0 + 0.15 * a.temperature, 1.0 + 0.03 * a.temperature, 1.0 - 0.15 * a.temperature), &m);
    }
    if a.tint != 0.0 {
        m = multiply(&scale_rgb(1.0 + 0.05 * a.tint, 1.0 - 0.12 * a.tint, 1.0 + 0.05 * a.tint), &m);
    }
    m
}

pub fn hex_to_rgb(hex: &str) -> [f64; 3] {
    let value = hex.trim().trim_start_matches('#');
    if value.len() != 6 || !value.chars().all(|c| c.is_ascii_hexdigit()) {
        return [1.0, 1.0, 1.0];
    }
    let int = u32::from_str_radix(value, 16).unwrap_or(0xffffff);
    [((int >> 16) & 255) as f64 / 255.0, ((int >> 8) & 255) as f64 / 255.0, (int & 255) as f64 / 255.0]
}

// ------------------------------------------------------------- effects.ts

pub const GLITCH_BANDS: usize = 6;

#[derive(Debug, Clone)]
pub struct EffectState {
    pub dx: f64,
    pub dy: f64,
    pub scale: f64,
    pub blur: f64,
    pub matrix: ColorMatrix,
    pub grain: f64,
    pub scanlines: bool,
    pub glitch: Vec<f64>,
}

impl Default for EffectState {
    fn default() -> Self {
        EffectState { dx: 0.0, dy: 0.0, scale: 1.0, blur: 0.0, matrix: IDENTITY, grain: 0.0, scanlines: false, glitch: vec![] }
    }
}

pub fn resolve_effect(effect: Option<&str>, elapsed: f64, duration: f64) -> EffectState {
    let idle = EffectState::default();
    let t = elapsed.max(0.0);
    match effect.unwrap_or("none") {
        "punch" => {
            let p = clamp(t / 0.32, 0.0, 1.0);
            if p >= 1.0 {
                return idle;
            }
            EffectState { scale: 1.0 + 0.12 * (1.0 - p).powi(2) * (p * std::f64::consts::PI * 2.0).cos(), ..idle }
        }
        "shake" => {
            let p = clamp(t / 0.45, 0.0, 1.0);
            if p >= 1.0 {
                return idle;
            }
            let envelope = (1.0 - p).powi(2);
            EffectState { dx: (t * 78.0).sin() * 1.2 * envelope, dy: (t * 62.0).cos() * 0.7 * envelope, ..idle }
        }
        "zoom-in" => EffectState { scale: 1.0 + 0.15 * clamp(t / duration.max(0.01), 0.0, 1.0), ..idle },
        "zoom-out" => EffectState { scale: 1.15 - 0.15 * clamp(t / duration.max(0.01), 0.0, 1.0), ..idle },
        "flash" => {
            let p = clamp(t / 0.35, 0.0, 1.0);
            if p >= 1.0 {
                return idle;
            }
            EffectState { matrix: brightness_matrix(0.6 * (1.0 - p).powi(2)), ..idle }
        }
        "blur" => EffectState { blur: 10.0, ..idle },
        "vhs" => EffectState { matrix: multiply(&saturate_matrix(0.78), &contrast_matrix(1.18)), grain: 0.09, scanlines: true, ..idle },
        "glitch" => {
            let slot = (t * 8.0).floor() as i64;
            if hash01(slot) <= 0.6 {
                return idle;
            }
            let glitch = (0..GLITCH_BANDS).map(|band| (hash01(slot * 31 + band as i64) - 0.5) * 6.0).collect();
            EffectState { dx: (hash01(slot + 7) - 0.5) * 1.5, glitch, ..idle }
        }
        _ => idle,
    }
}

// --------------------------------------------------------- transitions.ts

pub const MIN_TRANSITION: f64 = 0.1;
pub const MAX_TRANSITION: f64 = 3.0;
pub const DEFAULT_TRANSITION: f64 = 0.5;

#[derive(Debug, Clone)]
pub struct Connection {
    pub from: String,
    pub to: String,
    pub kind: String,
    pub start: f64,
    pub end: f64,
    pub duration: f64,
}

#[derive(Debug, Clone)]
pub struct TransitionState {
    pub dx: f64,
    pub dy: f64,
    pub scale: f64,
    pub rotation: f64,
    pub opacity: f64,
    pub blur: f64,
    pub matrix: ColorMatrix,
    pub clip_left: f64,
    pub clip_right: f64,
}

impl Default for TransitionState {
    fn default() -> Self {
        TransitionState { dx: 0.0, dy: 0.0, scale: 1.0, rotation: 0.0, opacity: 1.0, blur: 0.0, matrix: IDENTITY, clip_left: 0.0, clip_right: 1.0 }
    }
}

fn transitionable(item: &TimelineItem) -> bool {
    (item.kind == "video" || item.kind == "image") && !item.hidden
}

pub fn track_connections(track: &Track, fps: u32) -> Vec<Connection> {
    if track.kind != "video" || track.hidden {
        return vec![];
    }
    let tolerance = (0.5 / fps.max(1) as f64).max(0.002);
    let mut items: Vec<&TimelineItem> = track.items.iter().filter(|item| transitionable(item)).collect();
    items.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.id.cmp(&b.id)));
    let mut result = Vec::new();
    for pair in items.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        if (from.end() - to.start).abs() > tolerance {
            continue;
        }
        let Some(transition) = &to.transition else { continue };
        if transition.kind.is_empty() || transition.kind == "none" {
            continue;
        }
        let requested = if transition.duration > 0.0 { transition.duration } else { DEFAULT_TRANSITION };
        let duration = clamp(requested.min(from.duration).min(to.duration), MIN_TRANSITION, MAX_TRANSITION);
        result.push(Connection {
            from: from.id.clone(),
            to: to.id.clone(),
            kind: transition.kind.clone(),
            start: to.start - duration / 2.0,
            end: to.start + duration / 2.0,
            duration,
        });
    }
    result
}

pub fn resolve_transition(connection: Option<&Connection>, item_id: &str, time: f64) -> TransitionState {
    let mut state = TransitionState::default();
    let Some(c) = connection else { return state };
    if time < c.start || time > c.end {
        return state;
    }
    let incoming = c.to == item_id;
    if !incoming && c.from != item_id {
        return state;
    }
    let p = clamp((time - c.start) / c.duration, 0.0, 1.0);
    let e = ease_in_out_cubic(p);
    let s = ease_out_cubic(p);
    match c.kind.as_str() {
        "fade" => {
            if incoming {
                state.opacity = e;
            }
        }
        "black" | "white" => {
            let color = if c.kind == "black" { [0.0, 0.0, 0.0] } else { [1.0, 1.0, 1.0] };
            if incoming {
                if p < 0.5 {
                    state.opacity = 0.0;
                } else {
                    state.matrix = mix_matrix(color, 1.0 - (p - 0.5) / 0.5);
                }
            } else if p < 0.5 {
                state.matrix = mix_matrix(color, p / 0.5);
            } else {
                state.opacity = 0.0;
            }
        }
        "slide-left" => state.dx = if incoming { (1.0 - e) * 100.0 } else { -e * 100.0 },
        "slide-right" => state.dx = if incoming { -(1.0 - e) * 100.0 } else { e * 100.0 },
        "slide-up" => state.dy = if incoming { (1.0 - e) * 100.0 } else { -e * 100.0 },
        "slide-down" => state.dy = if incoming { -(1.0 - e) * 100.0 } else { e * 100.0 },
        "zoom" => {
            if incoming {
                state.opacity = s;
                state.scale = 1.3 - 0.3 * s;
                state.blur = (1.0 - s) * 8.0;
            } else {
                state.scale = 1.0 + 0.2 * e;
            }
        }
        "blur" => {
            if incoming {
                state.opacity = e;
                state.blur = (1.0 - s) * 20.0;
            } else {
                state.blur = e * 14.0;
            }
        }
        "wipe-left" => {
            if incoming {
                state.clip_left = 1.0 - e;
            }
        }
        "wipe-right" => {
            if incoming {
                state.clip_right = e;
            }
        }
        "flash" => {
            if incoming {
                state.opacity = e;
            }
            state.matrix = brightness_matrix((e * std::f64::consts::PI).sin() * if incoming { 0.8 } else { 0.6 });
        }
        "spin" => {
            if incoming {
                state.opacity = s;
                state.scale = 0.6 + 0.4 * s;
                state.rotation = (1.0 - s) * 30.0;
            } else {
                state.scale = 1.0 + 0.2 * e;
                state.rotation = -e * 20.0;
            }
        }
        _ => {}
    }
    state
}

// --------------------------------------------------------------- scene.ts

#[derive(Debug, Clone, Copy)]
pub struct MediaLayout {
    pub sx: f64,
    pub sy: f64,
    pub sw: f64,
    pub sh: f64,
    pub box_w: f64,
    pub box_h: f64,
}

pub fn media_layout(source_w: f64, source_h: f64, crop: Option<&Crop>, fit: &str, focus: Option<&Point>, canvas_w: f64, canvas_h: f64) -> MediaLayout {
    let c = crop.copied().unwrap_or_default();
    let sw = source_w.max(1.0);
    let sh = source_h.max(1.0);
    let cx = clamp(c.x, 0.0, 1.0) * sw;
    let cy = clamp(c.y, 0.0, 1.0) * sh;
    let cw = (sw - cx).min(clamp(c.w, 0.01, 1.0) * sw).max(1.0);
    let ch = (sh - cy).min(clamp(c.h, 0.01, 1.0) * sh).max(1.0);
    match fit {
        "fill" => MediaLayout { sx: cx, sy: cy, sw: cw, sh: ch, box_w: canvas_w, box_h: canvas_h },
        "cover" => {
            let k = (canvas_w / cw).max(canvas_h / ch);
            let vw = cw.min(canvas_w / k);
            let vh = ch.min(canvas_h / k);
            let focus = focus.copied().unwrap_or_default();
            let fx = clamp(focus.x, 0.0, 100.0) / 100.0;
            let fy = clamp(focus.y, 0.0, 100.0) / 100.0;
            MediaLayout { sx: cx + (cw - vw) * fx, sy: cy + (ch - vh) * fy, sw: vw, sh: vh, box_w: canvas_w, box_h: canvas_h }
        }
        _ => {
            let k = (canvas_w / cw).min(canvas_h / ch);
            MediaLayout { sx: cx, sy: cy, sw: cw, sh: ch, box_w: cw * k, box_h: ch * k }
        }
    }
}

pub fn shape_box(item: &TimelineItem, canvas_w: f64, canvas_h: f64) -> (f64, f64) {
    let shape = item.shape.clone().unwrap_or_default();
    let width = clamp(shape.width, 0.2, 400.0) / 100.0 * canvas_w;
    let height = if shape.kind == "circle" && shape.width == shape.height { width } else { clamp(shape.height, 0.1, 400.0) / 100.0 * canvas_h };
    (width, height)
}

pub fn media_time_at(item: &TimelineItem, time: f64, fps: u32, asset_duration: Option<f64>) -> f64 {
    let elapsed = clamp(time - item.start, 0.0, (item.duration - 1.0 / fps.max(1) as f64).max(0.0));
    let mut value = item.source_in + elapsed * item.speed();
    if let Some(duration) = asset_duration.filter(|d| *d > 0.0) {
        value = value.min((duration - 0.5 / fps.max(1) as f64).max(0.0));
    }
    value.max(0.0)
}

#[derive(Debug, Clone)]
pub struct Layer {
    pub item_index: (usize, usize),
    pub box_w: f64,
    pub box_h: f64,
    pub media_time: Option<f64>,
    pub cx: f64,
    pub cy: f64,
    pub scale: f64,
    pub rotation: f64,
    pub flip_x: bool,
    pub flip_y: bool,
    pub opacity: f64,
    pub blur: f64,
    pub blend: String,
    pub radius: f64,
    pub matrix: Option<ColorMatrix>,
    pub vignette: f64,
    pub clip_left: f64,
    pub clip_right: f64,
    pub grain: f64,
    pub scanlines: bool,
    pub glitch: Vec<f64>,
}

/// Size of a layer box that is not derived from the project document (text rasters).
pub trait BoxProvider {
    fn text_box(&self, item: &TimelineItem) -> Option<(f64, f64)>;
}

fn item_box(project: &Project, item: &TimelineItem, boxes: &dyn BoxProvider) -> Option<(f64, f64)> {
    let (w, h) = (project.width as f64, project.height as f64);
    match item.kind.as_str() {
        "video" | "image" => {
            let asset = project.asset(item.asset_id.as_deref())?;
            let layout = media_layout(asset.width?, asset.height?, item.crop.as_ref(), item.fit.as_deref().unwrap_or("contain"), item.focus.as_ref(), w, h);
            Some((layout.box_w, layout.box_h))
        }
        "shape" => Some(shape_box(item, w, h)),
        "text" => boxes.text_box(item),
        _ => None,
    }
}

pub fn build_layer(project: &Project, index: (usize, usize), time: f64, boxes: &dyn BoxProvider, connection: Option<&Connection>) -> Option<Layer> {
    let item = &project.tracks[index.0].items[index.1];
    let (box_w, box_h) = item_box(project, item, boxes)?;
    let elapsed = time - item.start;
    let transform = resolve_transform(item, elapsed);
    let motion = resolve_motion(item, elapsed);
    let effect = if item.kind == "shape" { EffectState::default() } else { resolve_effect(item.effect.as_deref(), elapsed, item.duration) };
    let transition = resolve_transition(connection, &item.id, time);

    let mut matrix = IDENTITY;
    if item.kind == "video" || item.kind == "image" {
        matrix = adjust_matrix(&item.adjust.unwrap_or_default());
    }
    matrix = multiply(&effect.matrix, &matrix);
    matrix = multiply(&transition.matrix, &matrix);

    let asset_duration = project.asset(item.asset_id.as_deref()).and_then(|asset| asset.duration);
    let (w, h) = (project.width as f64, project.height as f64);
    Some(Layer {
        item_index: index,
        box_w,
        box_h,
        media_time: (item.kind == "video").then(|| media_time_at(item, time, project.fps, asset_duration)),
        cx: (transform.x + motion.dx + effect.dx + transition.dx) / 100.0 * w,
        cy: (transform.y + motion.dy + effect.dy + transition.dy) / 100.0 * h,
        scale: transform.scale * motion.scale * effect.scale * transition.scale,
        rotation: transform.rotation + motion.rotation + transition.rotation,
        flip_x: item.flip_x,
        flip_y: item.flip_y,
        opacity: clamp(transform.opacity * motion.opacity * transition.opacity, 0.0, 1.0),
        blur: motion.blur.max(effect.blur).max(transition.blur),
        blend: item.blend.clone().unwrap_or_else(|| "normal".into()),
        radius: if item.kind == "shape" { 0.0 } else { item.radius.max(0.0) },
        matrix: (!is_identity(&matrix)).then_some(matrix),
        vignette: if item.kind == "video" || item.kind == "image" { clamp(item.adjust.map(|a| a.vignette).unwrap_or(0.0), 0.0, 1.0) } else { 0.0 },
        clip_left: transition.clip_left,
        clip_right: transition.clip_right.min(motion.reveal),
        grain: effect.grain,
        scanlines: effect.scanlines,
        glitch: effect.glitch,
    })
}

fn active_at(item: &TimelineItem, time: f64) -> bool {
    !item.hidden && time >= item.start && time < item.end()
}

/// Visual layers at `time`, bottom first. Mirror of `resolveScene`.
pub fn resolve_scene(project: &Project, connections: &[Vec<Connection>], time: f64, boxes: &dyn BoxProvider) -> Vec<Layer> {
    let mut layers = Vec::new();
    for track_index in (0..project.tracks.len()).rev() {
        let track = &project.tracks[track_index];
        if track.hidden || track.kind == "audio" {
            continue;
        }
        let active = connections[track_index].iter().find(|c| time >= c.start && time <= c.end);
        let mut entries: Vec<(usize, Option<&Connection>)> = Vec::new();
        if let Some(connection) = active {
            for id in [&connection.from, &connection.to] {
                if let Some(position) = track.items.iter().position(|item| &item.id == id) {
                    entries.push((position, Some(connection)));
                }
            }
        }
        let mut others: Vec<usize> = track
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.is_visual() && active_at(item, time))
            .filter(|(_, item)| active.map(|c| item.id != c.from && item.id != c.to).unwrap_or(true))
            .map(|(index, _)| index)
            .collect();
        others.sort_by(|a, b| track.items[*a].start.partial_cmp(&track.items[*b].start).unwrap_or(std::cmp::Ordering::Equal));
        entries.extend(others.into_iter().map(|index| (index, None)));
        for (item_index, connection) in entries {
            if let Some(layer) = build_layer(project, (track_index, item_index), time, boxes, connection) {
                if layer.opacity > 0.001 {
                    layers.push(layer);
                }
            }
        }
    }
    layers
}

pub fn asset_has_audio(asset: Option<&MediaAsset>) -> bool {
    asset.and_then(|asset| asset.has_audio).unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_matches_typescript() {
        // Values produced by hash01() in src/lib/engine/math.ts.
        let expected = [(0, 0.11478774505667388), (1, 0.24678996880538762), (42, 0.6052086122799665), (-5, 0.6545013557188213)];
        for (seed, value) in expected {
            assert!((hash01(seed) - value).abs() < 1e-12, "seed {seed}: {}", hash01(seed));
        }
    }

    #[test]
    fn cover_layout_respects_focus() {
        let layout = media_layout(1920.0, 1080.0, None, "cover", Some(&Point { x: 0.0, y: 50.0 }), 1080.0, 1920.0);
        assert!((layout.sx - 0.0).abs() < 1e-9);
        assert!((layout.sh - 1080.0).abs() < 1e-9);
        assert!((layout.sw - 607.5).abs() < 1e-9);
        assert_eq!((layout.box_w, layout.box_h), (1080.0, 1920.0));
    }

    struct FixedText;
    impl BoxProvider for FixedText {
        fn text_box(&self, _item: &TimelineItem) -> Option<(f64, f64)> {
            Some((100.0, 50.0))
        }
    }

    /// Compares against values computed by the TypeScript preview engine
    /// (src/lib/engine/parity.test.ts writes tests/fixtures/parity-expected.json).
    #[test]
    fn scene_matches_typescript_preview() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let project: Project = serde_json::from_slice(&std::fs::read(root.join("parity-project.json")).unwrap()).unwrap();
        let expected: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("parity-expected.json")).unwrap()).unwrap();
        let connections: Vec<Vec<Connection>> = project.tracks.iter().map(|track| track_connections(track, project.fps)).collect();
        let close = |a: f64, b: &serde_json::Value, what: &str, time: f64| {
            let b = b.as_f64().unwrap_or(f64::NAN);
            assert!((a - b).abs() < 1e-6, "{what} at {time}: rust {a} vs ts {b}");
        };
        for sample in expected.as_array().unwrap() {
            let time = sample["time"].as_f64().unwrap();
            let layers = resolve_scene(&project, &connections, time, &FixedText);
            let reference = sample["layers"].as_array().unwrap();
            let ids: Vec<&str> = layers.iter().map(|layer| project.tracks[layer.item_index.0].items[layer.item_index.1].id.as_str()).collect();
            let reference_ids: Vec<&str> = reference.iter().map(|layer| layer["id"].as_str().unwrap()).collect();
            assert_eq!(ids, reference_ids, "layer order at {time}");
            for (layer, want) in layers.iter().zip(reference) {
                close(layer.box_w, &want["boxW"], "boxW", time);
                close(layer.box_h, &want["boxH"], "boxH", time);
                close(layer.cx, &want["cx"], "cx", time);
                close(layer.cy, &want["cy"], "cy", time);
                close(layer.scale, &want["scale"], "scale", time);
                close(layer.rotation, &want["rotation"], "rotation", time);
                close(layer.opacity, &want["opacity"], "opacity", time);
                close(layer.blur, &want["blur"], "blur", time);
                close(layer.clip_left, &want["clipLeft"], "clipLeft", time);
                close(layer.clip_right, &want["clipRight"], "clipRight", time);
                close(layer.vignette, &want["vignette"], "vignette", time);
                close(layer.grain, &want["grain"], "grain", time);
                match layer.media_time {
                    Some(value) => close(value, &want["mediaTime"], "mediaTime", time),
                    None => assert!(want["mediaTime"].is_null(), "mediaTime at {time}"),
                }
                let glitch: Vec<f64> = want["glitch"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
                assert_eq!(layer.glitch.len(), glitch.len(), "glitch bands at {time}");
                for (a, b) in layer.glitch.iter().zip(&glitch) {
                    assert!((a - b).abs() < 1e-9, "glitch at {time}");
                }
                match (&layer.matrix, want["matrix"].as_array()) {
                    (None, None) => {}
                    (Some(matrix), Some(values)) => {
                        for (index, value) in values.iter().enumerate() {
                            close(matrix[index], value, &format!("matrix[{index}]"), time);
                        }
                    }
                    (rust, ts) => panic!("matrix presence differs at {time}: rust {:?} ts {:?}", rust.is_some(), ts.is_some()),
                }
            }
        }
    }

    #[test]
    fn contain_layout_letterboxes() {
        let layout = media_layout(1920.0, 1080.0, None, "contain", None, 1080.0, 1920.0);
        assert!((layout.box_w - 1080.0).abs() < 1e-9);
        assert!((layout.box_h - 607.5).abs() < 1e-9);
    }
}
