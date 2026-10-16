use super::math::{clamp, ease_in_cubic, ease_out_cubic, lerp};
use crate::model::{Animation, AnimationInKind, AnimationOutKind, ItemKind, TimelineItem};

/// Offsets in % of the canvas, blur in project pixels, rotation in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Motion {
    pub dx: f64,
    pub dy: f64,
    pub scale: f64,
    pub rotation: f64,
    pub opacity: f64,
    pub blur: f64,
    /// Fraction (0..1) of the layer revealed from the left edge; 1 = fully visible.
    pub reveal: f64,
}

pub const IDLE_MOTION: Motion = Motion { dx: 0.0, dy: 0.0, scale: 1.0, rotation: 0.0, opacity: 1.0, blur: 0.0, reveal: 1.0 };

pub struct Preset<K> {
    pub kind: K,
    pub label: &'static str,
    pub duration: f64,
}

pub const ANIMATIONS_IN: &[Preset<AnimationInKind>] = &[
    Preset { kind: AnimationInKind::None, label: "None", duration: 0.0 },
    Preset { kind: AnimationInKind::Fade, label: "Fade in", duration: 0.4 },
    Preset { kind: AnimationInKind::Pop, label: "Pop", duration: 0.3 },
    Preset { kind: AnimationInKind::Rise, label: "Rise", duration: 0.4 },
    Preset { kind: AnimationInKind::Drop, label: "Drop", duration: 0.4 },
    Preset { kind: AnimationInKind::SlideLeft, label: "Slide from right", duration: 0.4 },
    Preset { kind: AnimationInKind::SlideRight, label: "Slide from left", duration: 0.4 },
    Preset { kind: AnimationInKind::ZoomIn, label: "Zoom in", duration: 0.4 },
    Preset { kind: AnimationInKind::ZoomOut, label: "Zoom out", duration: 0.4 },
    Preset { kind: AnimationInKind::Bounce, label: "Bounce", duration: 0.45 },
    Preset { kind: AnimationInKind::Spin, label: "Spin", duration: 0.5 },
    Preset { kind: AnimationInKind::Blur, label: "Focus", duration: 0.4 },
    Preset { kind: AnimationInKind::Typewriter, label: "Typewriter", duration: 0.8 },
];

pub const ANIMATIONS_OUT: &[Preset<AnimationOutKind>] = &[
    Preset { kind: AnimationOutKind::None, label: "None", duration: 0.0 },
    Preset { kind: AnimationOutKind::Fade, label: "Fade out", duration: 0.4 },
    Preset { kind: AnimationOutKind::Shrink, label: "Shrink", duration: 0.35 },
    Preset { kind: AnimationOutKind::Grow, label: "Grow", duration: 0.35 },
    Preset { kind: AnimationOutKind::Sink, label: "Sink", duration: 0.4 },
    Preset { kind: AnimationOutKind::Lift, label: "Lift", duration: 0.4 },
    Preset { kind: AnimationOutKind::SlideLeft, label: "Slide left", duration: 0.4 },
    Preset { kind: AnimationOutKind::SlideRight, label: "Slide right", duration: 0.4 },
    Preset { kind: AnimationOutKind::Spin, label: "Spin", duration: 0.5 },
    Preset { kind: AnimationOutKind::Blur, label: "Blur", duration: 0.4 },
];

pub fn default_in_duration(kind: AnimationInKind) -> f64 {
    ANIMATIONS_IN.iter().find(|p| p.kind == kind).map(|p| p.duration).unwrap_or(0.4)
}

pub fn default_out_duration(kind: AnimationOutKind) -> f64 {
    ANIMATIONS_OUT.iter().find(|p| p.kind == kind).map(|p| p.duration).unwrap_or(0.4)
}

/// Time the in/out animations of a clip play over.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionSpan {
    pub start: f64,
    pub duration: f64,
    pub anim_in: Option<Animation<AnimationInKind>>,
    pub anim_out: Option<Animation<AnimationOutKind>>,
}

impl MotionSpan {
    pub fn of(item: &TimelineItem) -> Self {
        MotionSpan { start: item.start, duration: item.duration, anim_in: item.anim_in, anim_out: item.anim_out }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationWindows {
    pub in_kind: AnimationInKind,
    pub out_kind: AnimationOutKind,
    pub in_duration: f64,
    pub out_duration: f64,
}

/// Effective animation windows; in and out never overlap.
pub fn animation_windows(span: &MotionSpan) -> AnimationWindows {
    let in_kind = span.anim_in.map(|a| a.kind).unwrap_or_default();
    let out_kind = span.anim_out.map(|a| a.kind).unwrap_or_default();
    let mut in_duration = if in_kind == AnimationInKind::None { 0.0 } else { span.anim_in.map(|a| a.duration).unwrap_or(0.0).max(0.05) };
    let mut out_duration = if out_kind == AnimationOutKind::None { 0.0 } else { span.anim_out.map(|a| a.duration).unwrap_or(0.0).max(0.05) };
    let total = in_duration + out_duration;
    if total > span.duration && total > 0.0 {
        let ratio = span.duration / total;
        in_duration *= ratio;
        out_duration *= ratio;
    }
    AnimationWindows { in_kind, out_kind, in_duration, out_duration }
}

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

fn in_state(kind: AnimationInKind, progress: f64) -> Motion {
    let p = clamp(progress, 0.0, 1.0);
    let e = ease_out_cubic(p);
    let appear = clamp(p / 0.25, 0.0, 1.0);
    let mut s = IDLE_MOTION;
    match kind {
        AnimationInKind::None => {}
        AnimationInKind::Fade => s.opacity = e,
        AnimationInKind::Pop => {
            s.opacity = appear;
            s.scale = if p < 0.6 { lerp(0.6, 1.08, ease_out_cubic(p / 0.6)) } else { lerp(1.08, 1.0, ease_out_cubic((p - 0.6) / 0.4)) };
        }
        AnimationInKind::Rise => {
            s.opacity = appear;
            s.dy = (1.0 - e) * 10.0;
        }
        AnimationInKind::Drop => {
            s.opacity = appear;
            s.dy = -(1.0 - e) * 10.0;
        }
        AnimationInKind::SlideLeft => {
            s.opacity = appear;
            s.dx = (1.0 - e) * 35.0;
        }
        AnimationInKind::SlideRight => {
            s.opacity = appear;
            s.dx = -(1.0 - e) * 35.0;
        }
        AnimationInKind::ZoomIn => {
            s.opacity = e;
            s.scale = lerp(0.4, 1.0, e);
        }
        AnimationInKind::ZoomOut => {
            s.opacity = e;
            s.scale = lerp(1.8, 1.0, e);
        }
        AnimationInKind::Bounce => {
            s.opacity = appear;
            s.scale = if p < 0.45 {
                lerp(0.5, 1.16, ease_out_cubic(p / 0.45))
            } else if p < 0.75 {
                lerp(1.16, 0.95, ease_out_cubic((p - 0.45) / 0.3))
            } else {
                lerp(0.95, 1.0, ease_out_cubic((p - 0.75) / 0.25))
            };
        }
        AnimationInKind::Spin => {
            s.opacity = e;
            s.scale = lerp(0.3, 1.0, e);
            s.rotation = (1.0 - e) * -180.0;
        }
        AnimationInKind::Blur => {
            s.opacity = e;
            s.blur = (1.0 - e) * 24.0;
        }
        AnimationInKind::Typewriter => s.reveal = p,
    }
    s
}

fn out_state(kind: AnimationOutKind, progress: f64) -> Motion {
    let q = ease_in_cubic(progress);
    let fade = 1.0 - clamp((progress - 0.6) / 0.4, 0.0, 1.0);
    let mut s = IDLE_MOTION;
    match kind {
        AnimationOutKind::None => {}
        AnimationOutKind::Fade => s.opacity = 1.0 - q,
        AnimationOutKind::Shrink => {
            s.opacity = 1.0 - q;
            s.scale = lerp(1.0, 0.3, q);
        }
        AnimationOutKind::Grow => {
            s.opacity = 1.0 - q;
            s.scale = lerp(1.0, 1.8, q);
        }
        AnimationOutKind::Sink => {
            s.opacity = fade;
            s.dy = q * 10.0;
        }
        AnimationOutKind::Lift => {
            s.opacity = fade;
            s.dy = -q * 10.0;
        }
        AnimationOutKind::SlideLeft => {
            s.opacity = fade;
            s.dx = -q * 35.0;
        }
        AnimationOutKind::SlideRight => {
            s.opacity = fade;
            s.dx = q * 35.0;
        }
        AnimationOutKind::Spin => {
            s.opacity = 1.0 - q;
            s.scale = lerp(1.0, 0.3, q);
            s.rotation = q * 180.0;
        }
        AnimationOutKind::Blur => {
            s.opacity = 1.0 - q;
            s.blur = q * 24.0;
        }
    }
    s
}

/// Motion `elapsed` seconds after the span starts, caused by in/out animations.
pub fn resolve_motion(span: &MotionSpan, elapsed: f64) -> Motion {
    let w = animation_windows(span);
    let mut state = IDLE_MOTION;
    if w.in_kind != AnimationInKind::None && elapsed < w.in_duration {
        state = combine(state, in_state(w.in_kind, elapsed / w.in_duration));
    }
    let out_start = span.duration - w.out_duration;
    if w.out_kind != AnimationOutKind::None && elapsed > out_start {
        state = combine(state, out_state(w.out_kind, (elapsed - out_start) / w.out_duration));
    }
    state
}

/// `next` keeps the caption of `previous` on screen, only the highlighted word moved on.
fn continues_caption(previous: &TimelineItem, next: &TimelineItem) -> bool {
    previous.id != next.id
        && previous.kind == ItemKind::Text
        && next.kind == ItemKind::Text
        && next.text.as_ref().map(|t| t.highlight_index).unwrap_or(-1) > 0
        && previous.text.as_ref().map(|t| &t.content) == next.text.as_ref().map(|t| &t.content)
        && (previous.end() - next.start).abs() < 0.02
}

/// Indices (into `siblings`) of the clips that show the same word-highlight
/// caption as `siblings[index]`, in order. A lone clip is its own run.
pub fn caption_run(index: usize, siblings: &[TimelineItem]) -> Vec<usize> {
    let mut run = vec![index];
    if siblings[index].kind != ItemKind::Text {
        return run;
    }
    while let Some(previous) = siblings.iter().position(|candidate| continues_caption(candidate, &siblings[run[0]])) {
        if run.contains(&previous) {
            break;
        }
        run.insert(0, previous);
    }
    while let Some(next) = siblings.iter().position(|candidate| continues_caption(&siblings[*run.last().unwrap()], candidate)) {
        if run.contains(&next) {
            break;
        }
        run.push(next);
    }
    run
}

/// A word-highlight caption is one clip per word but animates as one text: in
/// on its first word, out on its last.
pub fn motion_span(index: usize, siblings: &[TimelineItem]) -> MotionSpan {
    let run = caption_run(index, siblings);
    let first = &siblings[run[0]];
    let last = &siblings[*run.last().unwrap()];
    MotionSpan { start: first.start, duration: last.end() - first.start, anim_in: first.anim_in, anim_out: last.anim_out }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TextStyle;

    fn word(id: &str, start: f64, duration: f64, content: &str, highlight_index: i64) -> TimelineItem {
        TimelineItem {
            id: id.into(),
            kind: ItemKind::Text,
            start,
            duration,
            text: Some(TextStyle { content: content.into(), highlight_index, ..Default::default() }),
            anim_in: Some(Animation { kind: AnimationInKind::Pop, duration: 0.3 }),
            anim_out: Some(Animation { kind: AnimationOutKind::Fade, duration: 0.3 }),
            ..Default::default()
        }
    }

    #[test]
    fn word_clips_animate_as_one_caption() {
        let items = vec![
            word("a", 0.0, 0.5, "one two three", 0),
            word("b", 0.5, 0.5, "one two three", 1),
            word("c", 1.0, 0.5, "one two three", 2),
            word("d", 1.5, 1.0, "next line", 0),
        ];
        assert_eq!(caption_run(1, &items), vec![0, 1, 2]);
        assert_eq!(caption_run(3, &items), vec![3]);
        let span = motion_span(1, &items);
        assert_eq!(span.start, 0.0);
        assert!((span.duration - 1.5).abs() < 1e-9);
        let middle = resolve_motion(&span, 0.6);
        assert_eq!(middle, IDLE_MOTION);
        assert!(resolve_motion(&motion_span(0, &items), 0.05).opacity < 1.0);
        assert!(resolve_motion(&motion_span(2, &items), 1.45).opacity < 1.0);
    }
}
