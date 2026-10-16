use crate::model::{Bezier, Easing};

pub const EPSILON: f64 = 1e-4;

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

/// Bezier equivalent of each named easing (the curve editor starts from these).
/// The four classic easings keep their own polynomial formulas when evaluated.
pub fn easing_curve(easing: Easing) -> Bezier {
    match easing {
        Easing::Linear | Easing::Custom => [0.25, 0.25, 0.75, 0.75],
        Easing::EaseIn => [0.42, 0.0, 1.0, 1.0],
        Easing::EaseOut => [0.0, 0.0, 0.58, 1.0],
        Easing::EaseInOut => [0.42, 0.0, 0.58, 1.0],
        Easing::BackIn => [0.36, 0.0, 0.66, -0.56],
        Easing::BackOut => [0.34, 1.56, 0.64, 1.0],
        Easing::BackInOut => [0.68, -0.6, 0.32, 1.6],
    }
}

pub fn sanitize_bezier(curve: Bezier) -> Option<Bezier> {
    curve
        .iter()
        .all(|value| value.is_finite())
        .then(|| [clamp(curve[0], 0.0, 1.0), clamp(curve[1], -1.0, 2.0), clamp(curve[2], 0.0, 1.0), clamp(curve[3], -1.0, 2.0)])
}

/// y of a CSS cubic-bezier at x (0..1): Newton steps with a bisection fallback.
pub fn cubic_bezier(curve: Bezier, x: f64) -> f64 {
    let [x1, y1, x2, y2] = sanitize_bezier(curve).unwrap_or([0.25, 0.25, 0.75, 0.75]);
    let cx = 3.0 * x1;
    let bx = 3.0 * (x2 - x1) - cx;
    let ax = 1.0 - cx - bx;
    let cy = 3.0 * y1;
    let by = 3.0 * (y2 - y1) - cy;
    let ay = 1.0 - cy - by;
    let sample_x = |t: f64| ((ax * t + bx) * t + cx) * t;
    let target = clamp(x, 0.0, 1.0);
    let mut t = target;
    let mut solved = false;
    for _ in 0..8 {
        let error = sample_x(t) - target;
        if error.abs() < 1e-7 {
            solved = true;
            break;
        }
        let slope = (3.0 * ax * t + 2.0 * bx) * t + cx;
        if slope.abs() < 1e-6 {
            break;
        }
        t -= error / slope;
    }
    if !solved {
        let (mut low, mut high) = (0.0, 1.0);
        t = target;
        for _ in 0..60 {
            let value = sample_x(t);
            if (value - target).abs() < 1e-7 {
                break;
            }
            if target > value {
                low = t;
            } else {
                high = t;
            }
            t = (high - low) / 2.0 + low;
        }
    }
    ((ay * t + by) * t + cy) * t
}

pub fn apply_easing(progress: f64, easing: Easing, bezier: Option<Bezier>) -> f64 {
    let v = clamp(progress, 0.0, 1.0);
    match easing {
        Easing::Linear => v,
        Easing::EaseIn => v * v,
        Easing::EaseOut => 1.0 - (1.0 - v) * (1.0 - v),
        Easing::EaseInOut => {
            if v < 0.5 {
                2.0 * v * v
            } else {
                1.0 - (-2.0 * v + 2.0).powi(2) / 2.0
            }
        }
        Easing::BackIn | Easing::BackOut | Easing::BackInOut => cubic_bezier(easing_curve(easing), v),
        Easing::Custom => bezier.and_then(sanitize_bezier).map(|curve| cubic_bezier(curve, v)).unwrap_or(v),
    }
}

/// Deterministic pseudo random number in [0, 1) (Math.imul based integer hash).
pub fn hash01(seed: i64) -> f64 {
    let seed = seed as i32 as u32;
    let mut x = (seed ^ 0x9e37_79b9).wrapping_mul(0x85eb_ca6b);
    x = (x ^ (x >> 13)).wrapping_mul(0xc2b2_ae35);
    x ^= x >> 16;
    x as f64 / 4_294_967_296.0
}

/// Unique id such as `video-lx3k9a-1f2e3d4c`: prefix, time in base 36, 32 random bits.
pub fn uid(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    // splitmix64 of time and counter: unique within a process, unpredictable enough across runs.
    let mut z = nanos ^ count.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (std::process::id() as u64) << 32;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    format!("{prefix}-{}-{:08x}", base36(nanos / 1_000_000), z as u32)
}

fn base36(mut value: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if value == 0 {
        return "0".into();
    }
    let mut out = Vec::new();
    while value > 0 {
        out.push(DIGITS[(value % 36) as usize]);
        value /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

pub fn round_to(value: f64, step: f64) -> f64 {
    (value / step).round() * step
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_matches_typescript() {
        let expected = [(0, 0.11478774505667388), (1, 0.24678996880538762), (42, 0.6052086122799665), (-5, 0.6545013557188213)];
        for (seed, value) in expected {
            assert!((hash01(seed) - value).abs() < 1e-12, "seed {seed}");
        }
    }

    #[test]
    fn bezier_endpoints_and_known_values() {
        for easing in Easing::ALL {
            let curve = easing_curve(*easing);
            assert!(cubic_bezier(curve, 0.0).abs() < 1e-9);
            assert!((cubic_bezier(curve, 1.0) - 1.0).abs() < 1e-9);
        }
        assert!((cubic_bezier([0.42, 0.0, 0.58, 1.0], 0.25) - 0.129).abs() < 1e-3);
        assert!([0.5, 0.6, 0.7, 0.8].iter().any(|x| apply_easing(*x, Easing::BackOut, None) > 1.0));
        assert!(apply_easing(0.2, Easing::BackIn, None) < 0.0);
        assert_eq!(apply_easing(0.3, Easing::Custom, None), 0.3);
    }

    #[test]
    fn ids_are_unique() {
        let ids: std::collections::HashSet<String> = (0..1000).map(|_| uid("x")).collect();
        assert_eq!(ids.len(), 1000);
        assert!(uid("video").starts_with("video-"));
    }
}
