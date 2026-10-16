use super::math::{apply_easing, clamp, easing_curve, lerp, uid, EPSILON};
use crate::model::{Bezier, Easing, Keyframe, TimelineItem, Transform};

pub const MIN_SCALE: f64 = 0.05;
pub const MAX_SCALE: f64 = 10.0;

pub fn normalize_transform(value: Option<&Transform>) -> Transform {
    let v = value.copied().unwrap_or_default();
    let finite = |value: f64, fallback: f64| if value.is_finite() { value } else { fallback };
    let scale = if v.scale > 0.0 { v.scale } else { 1.0 };
    Transform {
        x: clamp(finite(v.x, 50.0), -100.0, 200.0),
        y: clamp(finite(v.y, 50.0), -100.0, 200.0),
        scale: clamp(scale, MIN_SCALE, MAX_SCALE),
        rotation: finite(v.rotation, 0.0),
        opacity: clamp(finite(v.opacity, 1.0), 0.0, 1.0),
    }
}

/// Keyframes in time order, clamped to the clip, one per instant.
pub fn sorted_keyframes(item: &TimelineItem) -> Vec<Keyframe> {
    let duration = item.duration.max(0.0);
    let mut list: Vec<Keyframe> = item
        .keyframes
        .iter()
        .filter(|keyframe| keyframe.time.is_finite())
        .map(|keyframe| Keyframe { time: clamp(keyframe.time, 0.0, duration), ..keyframe.clone() })
        .collect();
    list.sort_by(|a, b| a.time.total_cmp(&b.time).then_with(|| a.id.cmp(&b.id)));
    let mut result: Vec<Keyframe> = Vec::with_capacity(list.len());
    for index in 0..list.len() {
        if index == list.len() - 1 || (list[index + 1].time - list[index].time).abs() > EPSILON {
            result.push(list[index].clone());
        }
    }
    result
}

fn interpolate(left: Transform, right: Transform, progress: f64, easing: Easing, bezier: Option<Bezier>) -> Transform {
    let t = apply_easing(progress, easing, bezier);
    // Overshooting curves may carry scale and opacity past their keyframes.
    Transform {
        x: lerp(left.x, right.x, t),
        y: lerp(left.y, right.y, t),
        scale: lerp(left.scale, right.scale, t).max(MIN_SCALE),
        rotation: lerp(left.rotation, right.rotation, t),
        opacity: clamp(lerp(left.opacity, right.opacity, t), 0.0, 1.0),
    }
}

/// Transform of a clip `elapsed` seconds after its start, keyframes included.
pub fn resolve_transform(item: &TimelineItem, elapsed: f64) -> Transform {
    let base = normalize_transform(item.transform.as_ref());
    if item.keyframes.is_empty() {
        return base;
    }
    let keyframes = sorted_keyframes(item);
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
        return interpolate(
            normalize_transform(Some(&left.transform)),
            normalize_transform(Some(&right.transform)),
            (time - left.time) / span,
            right.easing,
            right.bezier,
        );
    }
    normalize_transform(Some(&keyframes[keyframes.len() - 1].transform))
}

pub fn keyframe_at(item: &TimelineItem, elapsed: f64, tolerance: f64) -> Option<Keyframe> {
    sorted_keyframes(item).into_iter().find(|keyframe| (keyframe.time - elapsed).abs() <= tolerance)
}

fn make_keyframe(transform: Transform, time: f64, existing: Option<&Keyframe>) -> Keyframe {
    Keyframe {
        id: existing.map(|k| k.id.clone()).unwrap_or_else(|| uid("kf")),
        time,
        transform,
        easing: existing.map(|k| k.easing).unwrap_or_default(),
        bezier: existing.and_then(|k| k.bezier),
    }
}

fn by_time(mut list: Vec<Keyframe>) -> Vec<Keyframe> {
    list.sort_by(|a, b| a.time.total_cmp(&b.time));
    list
}

pub fn toggle_keyframe(item: &TimelineItem, elapsed: f64, tolerance: f64) -> Vec<Keyframe> {
    let time = clamp(elapsed, 0.0, item.duration);
    let keyframes = sorted_keyframes(item);
    if let Some(existing) = keyframes.iter().find(|k| (k.time - time).abs() <= tolerance) {
        let id = existing.id.clone();
        return keyframes.into_iter().filter(|k| k.id != id).collect();
    }
    let mut list = keyframes;
    list.push(make_keyframe(resolve_transform(item, time), time, None));
    by_time(list)
}

/// Writes a transform change into the keyframe at the playhead, creating it when needed.
pub fn patch_keyframe_at(item: &TimelineItem, elapsed: f64, patch: impl Fn(Transform) -> Transform, tolerance: f64) -> Vec<Keyframe> {
    let time = clamp(elapsed, 0.0, item.duration);
    let keyframes = sorted_keyframes(item);
    let existing = keyframes.iter().find(|k| (k.time - time).abs() <= tolerance).cloned();
    let state = match &existing {
        Some(keyframe) => normalize_transform(Some(&keyframe.transform)),
        None => resolve_transform(item, time),
    };
    let next = make_keyframe(normalize_transform(Some(&patch(state))), existing.as_ref().map(|k| k.time).unwrap_or(time), existing.as_ref());
    let mut list: Vec<Keyframe> = keyframes.into_iter().filter(|k| Some(&k.id) != existing.as_ref().map(|e| &e.id)).collect();
    list.push(next);
    by_time(list)
}

/// A `Custom` easing keeps `bezier` (or the curve it had); presets drop the curve.
pub fn set_keyframe_easing(item: &TimelineItem, elapsed: f64, easing: Easing, tolerance: f64, bezier: Option<Bezier>) -> Vec<Keyframe> {
    sorted_keyframes(item)
        .into_iter()
        .map(|mut keyframe| {
            if (keyframe.time - elapsed).abs() > tolerance {
                return keyframe;
            }
            if easing != Easing::Custom {
                keyframe.bezier = None;
            } else {
                let previous = if keyframe.easing == Easing::Custom { keyframe.bezier } else { Some(easing_curve(keyframe.easing)) };
                keyframe.bezier = bezier.or(previous).or(Some(easing_curve(Easing::BackOut)));
            }
            keyframe.easing = easing;
            keyframe
        })
        .collect()
}

pub fn adjacent_keyframe(item: &TimelineItem, elapsed: f64, direction: i32, tolerance: f64) -> Option<Keyframe> {
    let keyframes = sorted_keyframes(item);
    if direction < 0 {
        keyframes.into_iter().rev().find(|k| k.time < elapsed - tolerance)
    } else {
        keyframes.into_iter().find(|k| k.time > elapsed + tolerance)
    }
}

/// Stretches keyframe times when a clip changes length because of a speed change.
pub fn retime_keyframes(keyframes: &[Keyframe], from_duration: f64, to_duration: f64) -> Vec<Keyframe> {
    let ratio = to_duration / from_duration.max(EPSILON);
    keyframes.iter().map(|k| Keyframe { time: clamp(k.time * ratio, 0.0, to_duration), ..k.clone() }).collect()
}

/// Cuts the keyframes of `item` down to [from, from + duration] and rebases them
/// to the new clip start. Boundary keyframes keep the motion inside the window
/// exactly the same.
pub fn trim_keyframes(item: &TimelineItem, from: f64, duration: f64) -> Vec<Keyframe> {
    let keyframes = sorted_keyframes(item);
    if keyframes.is_empty() {
        return vec![];
    }
    let safe_from = clamp(from, 0.0, item.duration);
    let safe_duration = clamp(duration, 0.0, item.duration - safe_from);
    let end = safe_from + safe_duration;
    let mut result: Vec<Keyframe> = keyframes
        .into_iter()
        .filter(|k| k.time >= safe_from - EPSILON && k.time <= end + EPSILON)
        .map(|k| Keyframe { time: clamp(k.time - safe_from, 0.0, safe_duration), ..k })
        .collect();
    if safe_from > EPSILON && !result.iter().any(|k| k.time <= EPSILON) {
        result.push(make_keyframe(resolve_transform(item, safe_from), 0.0, None));
    }
    if end < item.duration - EPSILON && !result.iter().any(|k| (k.time - safe_duration).abs() <= EPSILON) {
        result.push(make_keyframe(resolve_transform(item, end), safe_duration, None));
    }
    by_time(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn animated() -> TimelineItem {
        let frame = |id: &str, time: f64, x: f64| Keyframe { id: id.into(), time, transform: Transform { x, ..Default::default() }, ..Default::default() };
        TimelineItem { id: "a".into(), duration: 4.0, keyframes: vec![frame("k1", 0.0, 0.0), frame("k2", 4.0, 100.0)], ..Default::default() }
    }

    #[test]
    fn interpolates_between_keyframes() {
        assert!((resolve_transform(&animated(), 2.0).x - 50.0).abs() < 1e-9);
    }

    #[test]
    fn trimming_keeps_the_motion() {
        let item = animated();
        let trimmed = TimelineItem { duration: 2.0, keyframes: trim_keyframes(&item, 1.0, 2.0), ..item.clone() };
        for t in [0.0, 0.5, 1.0, 1.5, 2.0] {
            assert!((resolve_transform(&trimmed, t).x - resolve_transform(&item, t + 1.0).x).abs() < 1e-6);
        }
    }

    #[test]
    fn toggle_adds_then_removes() {
        let mut item = animated();
        item.keyframes = toggle_keyframe(&item, 2.0, 0.01);
        assert_eq!(item.keyframes.len(), 3);
        item.keyframes = toggle_keyframe(&item, 2.0, 0.01);
        assert_eq!(item.keyframes.len(), 2);
    }

    #[test]
    fn custom_easing_starts_from_previous_curve() {
        let item = animated();
        let list = set_keyframe_easing(&item, 4.0, Easing::Custom, 0.01, None);
        assert_eq!(list[1].bezier, Some(easing_curve(Easing::Linear)));
        let list = set_keyframe_easing(&TimelineItem { keyframes: list, ..item }, 4.0, Easing::EaseIn, 0.01, None);
        assert_eq!(list[1].bezier, None);
    }
}
