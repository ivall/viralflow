use super::color::{brightness_matrix, mix_matrix, ColorMatrix, IDENTITY};
use super::math::{clamp, ease_in_out_cubic, ease_out_cubic};
use crate::model::{Animation, ItemKind, TimelineItem, Track, TrackKind, TransitionKind};

pub const MIN_TRANSITION: f64 = 0.1;
pub const MAX_TRANSITION: f64 = 3.0;
pub const DEFAULT_TRANSITION: f64 = 0.5;

pub struct TransitionPreset {
    pub kind: TransitionKind,
    pub label: &'static str,
}

pub const TRANSITIONS: &[TransitionPreset] = &[
    TransitionPreset { kind: TransitionKind::Fade, label: "Cross dissolve" },
    TransitionPreset { kind: TransitionKind::Black, label: "Dip to black" },
    TransitionPreset { kind: TransitionKind::White, label: "Dip to white" },
    TransitionPreset { kind: TransitionKind::SlideLeft, label: "Push left" },
    TransitionPreset { kind: TransitionKind::SlideRight, label: "Push right" },
    TransitionPreset { kind: TransitionKind::SlideUp, label: "Push up" },
    TransitionPreset { kind: TransitionKind::SlideDown, label: "Push down" },
    TransitionPreset { kind: TransitionKind::Zoom, label: "Zoom" },
    TransitionPreset { kind: TransitionKind::Blur, label: "Blur" },
    TransitionPreset { kind: TransitionKind::WipeLeft, label: "Wipe left" },
    TransitionPreset { kind: TransitionKind::WipeRight, label: "Wipe right" },
    TransitionPreset { kind: TransitionKind::Flash, label: "Flash" },
    TransitionPreset { kind: TransitionKind::Spin, label: "Spin" },
];

/// A cut between two touching visual clips of a track, with or without a transition.
#[derive(Debug, Clone, PartialEq)]
pub struct Seam {
    pub from: String,
    pub to: String,
    pub cut: f64,
    pub kind: TransitionKind,
    pub duration: f64,
}

/// A seam that carries a transition, active over [start, end].
#[derive(Debug, Clone, PartialEq)]
pub struct Connection {
    pub from: String,
    pub to: String,
    pub kind: TransitionKind,
    pub cut: f64,
    pub start: f64,
    pub end: f64,
    pub duration: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransitionState {
    pub dx: f64,
    pub dy: f64,
    pub scale: f64,
    pub rotation: f64,
    pub opacity: f64,
    pub blur: f64,
    pub matrix: ColorMatrix,
    /// Visible horizontal span of the layer box as fractions [left, right].
    pub clip_left: f64,
    pub clip_right: f64,
}

impl Default for TransitionState {
    fn default() -> Self {
        TransitionState { dx: 0.0, dy: 0.0, scale: 1.0, rotation: 0.0, opacity: 1.0, blur: 0.0, matrix: IDENTITY, clip_left: 0.0, clip_right: 1.0 }
    }
}

pub fn is_transitionable(item: &TimelineItem) -> bool {
    matches!(item.kind, ItemKind::Video | ItemKind::Image) && !item.hidden
}

fn seam_tolerance(fps: u32) -> f64 {
    (0.5 / fps.max(1) as f64).max(0.002)
}

pub fn transition_duration(to: &TimelineItem, from: &TimelineItem) -> f64 {
    let requested = to.transition.map(|t| t.duration).filter(|d| *d > 0.0).unwrap_or(DEFAULT_TRANSITION);
    clamp(requested.min(from.duration).min(to.duration), MIN_TRANSITION, MAX_TRANSITION)
}

/// All cuts between touching visual clips of a track.
pub fn track_seams(track: &Track, fps: u32) -> Vec<Seam> {
    if track.kind != TrackKind::Video {
        return vec![];
    }
    let tolerance = seam_tolerance(fps);
    let mut items: Vec<&TimelineItem> = track.items.iter().filter(|item| is_transitionable(item)).collect();
    items.sort_by(|a, b| a.start.total_cmp(&b.start).then_with(|| a.id.cmp(&b.id)));
    items
        .windows(2)
        .filter(|pair| (pair[0].end() - pair[1].start).abs() <= tolerance)
        .map(|pair| Seam {
            from: pair[0].id.clone(),
            to: pair[1].id.clone(),
            cut: pair[1].start,
            kind: pair[1].transition.map(|t| t.kind).unwrap_or_default(),
            duration: transition_duration(pair[1], pair[0]),
        })
        .collect()
}

pub fn track_connections(track: &Track, fps: u32) -> Vec<Connection> {
    if track.hidden {
        return vec![];
    }
    track_seams(track, fps)
        .into_iter()
        .filter(|seam| seam.kind != TransitionKind::None)
        .map(|seam| Connection {
            start: seam.cut - seam.duration / 2.0,
            end: seam.cut + seam.duration / 2.0,
            from: seam.from,
            to: seam.to,
            kind: seam.kind,
            cut: seam.cut,
            duration: seam.duration,
        })
        .collect()
}

/// Drops transitions that no longer sit on a cut and clamps their length.
pub fn repair_transitions(track: &mut Track, fps: u32) {
    let seams = track_seams(track, fps);
    let lookup: std::collections::HashMap<String, f64> = seams.into_iter().map(|seam| (seam.to, seam.duration)).collect();
    for item in &mut track.items {
        let Some(transition) = item.transition else { continue };
        if transition.kind == TransitionKind::None {
            item.transition = None;
            continue;
        }
        match lookup.get(&item.id) {
            Some(duration) => item.transition = Some(Animation { kind: transition.kind, duration: *duration }),
            None => item.transition = None,
        }
    }
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
    match c.kind {
        TransitionKind::None => {}
        TransitionKind::Fade => {
            if incoming {
                state.opacity = e;
            }
        }
        TransitionKind::Black | TransitionKind::White => {
            let color = if c.kind == TransitionKind::Black { [0.0, 0.0, 0.0] } else { [1.0, 1.0, 1.0] };
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
        TransitionKind::SlideLeft => state.dx = if incoming { (1.0 - e) * 100.0 } else { -e * 100.0 },
        TransitionKind::SlideRight => state.dx = if incoming { -(1.0 - e) * 100.0 } else { e * 100.0 },
        TransitionKind::SlideUp => state.dy = if incoming { (1.0 - e) * 100.0 } else { -e * 100.0 },
        TransitionKind::SlideDown => state.dy = if incoming { -(1.0 - e) * 100.0 } else { e * 100.0 },
        TransitionKind::Zoom => {
            if incoming {
                state.opacity = s;
                state.scale = 1.3 - 0.3 * s;
                state.blur = (1.0 - s) * 8.0;
            } else {
                state.scale = 1.0 + 0.2 * e;
            }
        }
        TransitionKind::Blur => {
            if incoming {
                state.opacity = e;
                state.blur = (1.0 - s) * 20.0;
            } else {
                state.blur = e * 14.0;
            }
        }
        TransitionKind::WipeLeft => {
            if incoming {
                state.clip_left = 1.0 - e;
            }
        }
        TransitionKind::WipeRight => {
            if incoming {
                state.clip_right = e;
            }
        }
        TransitionKind::Flash => {
            if incoming {
                state.opacity = e;
            }
            state.matrix = brightness_matrix((e * std::f64::consts::PI).sin() * if incoming { 0.8 } else { 0.6 });
        }
        TransitionKind::Spin => {
            if incoming {
                state.opacity = s;
                state.scale = 0.6 + 0.4 * s;
                state.rotation = (1.0 - s) * 30.0;
            } else {
                state.scale = 1.0 + 0.2 * e;
                state.rotation = -e * 20.0;
            }
        }
    }
    state
}
