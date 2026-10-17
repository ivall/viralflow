//! 4×5 row-major colour matrices in the layout of SVG feColorMatrix:
//! R' = m0·R + m1·G + m2·B + m3·A + m4 (channels and offsets in 0..1).

use super::math::clamp;
use crate::model::Adjust;

pub type ColorMatrix = [f64; 20];

pub const IDENTITY: ColorMatrix = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];

/// Returns a ∘ b: b is applied first, then a.
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

pub fn contrast_matrix(k: f64) -> ColorMatrix {
    let o = 0.5 * (1.0 - k);
    [k, 0.0, 0.0, 0.0, o, 0.0, k, 0.0, 0.0, o, 0.0, 0.0, k, 0.0, o, 0.0, 0.0, 0.0, 1.0, 0.0]
}

/// CSS saturate() matrix.
pub fn saturate_matrix(s: f64) -> ColorMatrix {
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

/// CSS hue-rotate() matrix.
pub fn hue_matrix(degrees: f64) -> ColorMatrix {
    let (s, c) = degrees.to_radians().sin_cos();
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

/// Mixes RGB towards a solid colour: out = in·(1-a) + color·a.
pub fn mix_matrix(color: [f64; 3], amount: f64) -> ColorMatrix {
    let a = clamp(amount, 0.0, 1.0);
    let k = 1.0 - a;
    [k, 0.0, 0.0, 0.0, color[0] * a, 0.0, k, 0.0, 0.0, color[1] * a, 0.0, 0.0, k, 0.0, color[2] * a, 0.0, 0.0, 0.0, 1.0, 0.0]
}

/// CSS sepia() matrix, mixed in by `amount` 0..1.
pub fn sepia_matrix(amount: f64) -> ColorMatrix {
    let a = clamp(amount, 0.0, 1.0);
    let k = 1.0 - a;
    [
        k + 0.393 * a,
        0.769 * a,
        0.189 * a,
        0.0,
        0.0,
        0.349 * a,
        k + 0.686 * a,
        0.168 * a,
        0.0,
        0.0,
        0.272 * a,
        0.534 * a,
        k + 0.131 * a,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
    ]
}

/// CSS invert() matrix: `amount` 1 is the negative.
pub fn invert_matrix(amount: f64) -> ColorMatrix {
    let a = clamp(amount, 0.0, 1.0);
    let k = 1.0 - 2.0 * a;
    [k, 0.0, 0.0, 0.0, a, 0.0, k, 0.0, 0.0, a, 0.0, 0.0, k, 0.0, a, 0.0, 0.0, 0.0, 1.0, 0.0]
}

pub fn brightness_matrix(offset: f64) -> ColorMatrix {
    offset_rgb(offset)
}

pub fn contrast_and_saturation(contrast: f64, saturation: f64) -> ColorMatrix {
    multiply(&saturate_matrix(saturation), &contrast_matrix(contrast))
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

/// "#rrggbb" to 0..1 channels; anything else is white.
pub fn hex_to_rgb(hex: &str) -> [f64; 3] {
    let value = hex.trim().trim_start_matches('#');
    if value.len() != 6 || !value.chars().all(|c| c.is_ascii_hexdigit()) {
        return [1.0, 1.0, 1.0];
    }
    let int = u32::from_str_radix(value, 16).unwrap_or(0xffffff);
    [((int >> 16) & 255) as f64 / 255.0, ((int >> 8) & 255) as f64 / 255.0, (int & 255) as f64 / 255.0]
}

pub fn is_hex_color(value: &str) -> bool {
    let value = value.trim();
    value.len() == 7 && value.starts_with('#') && value[1..].chars().all(|c| c.is_ascii_hexdigit())
}
