//! Per-pixel operations on premultiplied RGBA buffers (tiny-skia layout).

use rayon::prelude::*;

use crate::engine::{hash01, ColorMatrix};

/// Applies a 4×5 colour matrix (feColorMatrix semantics, unpremultiplied, sRGB).
pub fn apply_color_matrix(data: &mut [u8], m: &ColorMatrix) {
    let m: [f32; 20] = std::array::from_fn(|i| m[i] as f32);
    data.par_chunks_exact_mut(4 * 256).for_each(|chunk| process_matrix(chunk, &m));
    let tail = data.len() / (4 * 256) * (4 * 256);
    process_matrix(&mut data[tail..], &m);
}

fn process_matrix(chunk: &mut [u8], m: &[f32; 20]) {
    for px in chunk.chunks_exact_mut(4) {
        let a = px[3] as f32 / 255.0;
        if a <= 0.0 {
            continue;
        }
        let r = px[0] as f32 / 255.0 / a;
        let g = px[1] as f32 / 255.0 / a;
        let b = px[2] as f32 / 255.0 / a;
        let nr = (m[0] * r + m[1] * g + m[2] * b + m[3] * a + m[4]).clamp(0.0, 1.0);
        let ng = (m[5] * r + m[6] * g + m[7] * b + m[8] * a + m[9]).clamp(0.0, 1.0);
        let nb = (m[10] * r + m[11] * g + m[12] * b + m[13] * a + m[14]).clamp(0.0, 1.0);
        let na = (m[15] * r + m[16] * g + m[17] * b + m[18] * a + m[19]).clamp(0.0, 1.0);
        px[0] = (nr * na * 255.0 + 0.5) as u8;
        px[1] = (ng * na * 255.0 + 0.5) as u8;
        px[2] = (nb * na * 255.0 + 0.5) as u8;
        px[3] = (na * 255.0 + 0.5) as u8;
    }
}

/// Darkens towards the edges of the buffer (elliptical falloff), like the preview vignette.
pub fn apply_vignette(data: &mut [u8], width: usize, height: usize, strength: f32) {
    let cx = width as f32 / 2.0;
    let cy = height as f32 / 2.0;
    data.par_chunks_exact_mut(width * 4).enumerate().for_each(|(y, row)| {
        let dy = (y as f32 + 0.5 - cy) / cy;
        for x in 0..width {
            let dx = (x as f32 + 0.5 - cx) / cx;
            let distance = (dx * dx + dy * dy).sqrt() / std::f32::consts::SQRT_2;
            let t = ((distance - 0.35) / 0.65).clamp(0.0, 1.0);
            let falloff = 1.0 - strength * 0.85 * (t * t * (3.0 - 2.0 * t));
            let px = &mut row[x * 4..x * 4 + 3];
            for channel in px {
                *channel = (*channel as f32 * falloff) as u8;
            }
        }
    });
}

/// Film grain and optional scanlines.
pub fn apply_grain(data: &mut [u8], width: usize, frame: i64, grain: f32, scanlines: bool) {
    data.par_chunks_exact_mut(width * 4).enumerate().for_each(|(y, row)| {
        let line = if scanlines && y % 4 == 0 { 0.85 } else { 1.0 };
        for x in 0..width {
            let px = &mut row[x * 4..x * 4 + 4];
            if px[3] == 0 {
                continue;
            }
            let alpha = px[3] as f32;
            let noise = (hash01(frame * 7_919 + (y * width + x) as i64) as f32 - 0.5) * grain * 2.0 * alpha;
            for channel in &mut px[..3] {
                *channel = ((*channel as f32 + noise) * line).clamp(0.0, alpha) as u8;
            }
        }
    });
}

/// Three box-blur passes approximate a gaussian with the given sigma.
pub fn gaussian_blur(data: &mut [u8], width: usize, height: usize, sigma: f32) {
    if sigma < 0.5 || width == 0 || height == 0 {
        return;
    }
    let radii = box_radii(sigma);
    let mut buffer = vec![0u8; data.len()];
    for radius in radii {
        blur_horizontal(data, &mut buffer, width, radius);
        blur_vertical(&buffer, data, width, height, radius);
    }
}

fn box_radii(sigma: f32) -> [usize; 3] {
    let n = 3.0f32;
    let ideal = (12.0 * sigma * sigma / n + 1.0).sqrt();
    let mut lower = ideal.floor() as i32;
    if lower % 2 == 0 {
        lower -= 1;
    }
    let upper = lower + 2;
    let ideal_m = (12.0 * sigma * sigma - n * (lower * lower) as f32 - 4.0 * n * lower as f32 - 3.0 * n) / (-4.0 * lower as f32 - 4.0);
    let m = ideal_m.round() as i32;
    let size = |i: i32| if i < m { lower } else { upper };
    [((size(0) - 1) / 2).max(0) as usize, ((size(1) - 1) / 2).max(0) as usize, ((size(2) - 1) / 2).max(0) as usize]
}

fn blur_horizontal(src: &[u8], dst: &mut [u8], width: usize, radius: usize) {
    if radius == 0 {
        dst.copy_from_slice(src);
        return;
    }
    let window = (2 * radius + 1) as u32;
    dst.par_chunks_exact_mut(width * 4).zip(src.par_chunks_exact(width * 4)).for_each(|(out, row)| {
        let mut sum = [0u32; 4];
        let at = |x: isize| -> usize { x.clamp(0, width as isize - 1) as usize * 4 };
        for i in -(radius as isize)..=(radius as isize) {
            let p = at(i);
            for c in 0..4 {
                sum[c] += row[p + c] as u32;
            }
        }
        for x in 0..width {
            for c in 0..4 {
                out[x * 4 + c] = (sum[c] / window) as u8;
            }
            let add = at(x as isize + radius as isize + 1);
            let remove = at(x as isize - radius as isize);
            for c in 0..4 {
                sum[c] = sum[c] + row[add + c] as u32 - row[remove + c] as u32;
            }
        }
    });
}

fn blur_vertical(src: &[u8], dst: &mut [u8], width: usize, height: usize, radius: usize) {
    if radius == 0 {
        dst.copy_from_slice(src);
        return;
    }
    let window = (2 * radius + 1) as u32;
    // Process column strips in parallel; each strip writes disjoint columns.
    let strip = 16usize;
    let columns: Vec<usize> = (0..width).step_by(strip).collect();
    let dst_ptr = dst.as_mut_ptr() as usize;
    columns.par_iter().for_each(|&start| {
        let end = (start + strip).min(width);
        let dst = dst_ptr as *mut u8;
        for x in start..end {
            let at = |y: isize| -> usize { (y.clamp(0, height as isize - 1) as usize * width + x) * 4 };
            let mut sum = [0u32; 4];
            for i in -(radius as isize)..=(radius as isize) {
                let p = at(i);
                for (c, total) in sum.iter_mut().enumerate() {
                    *total += src[p + c] as u32;
                }
            }
            for y in 0..height {
                let o = (y * width + x) * 4;
                for (c, total) in sum.iter().enumerate() {
                    // SAFETY: every task writes only to its own columns [start, end).
                    unsafe { *dst.add(o + c) = (total / window) as u8 };
                }
                let add = at(y as isize + radius as isize + 1);
                let remove = at(y as isize - radius as isize);
                for (c, total) in sum.iter_mut().enumerate() {
                    *total = *total + src[add + c] as u32 - src[remove + c] as u32;
                }
            }
        }
    });
}

/// Converts straight RGBA to premultiplied in place.
pub fn premultiply(data: &mut [u8]) {
    data.par_chunks_exact_mut(4).for_each(|px| {
        let a = px[3] as u32;
        if a < 255 {
            px[0] = ((px[0] as u32 * a + 127) / 255) as u8;
            px[1] = ((px[1] as u32 * a + 127) / 255) as u8;
            px[2] = ((px[2] as u32 * a + 127) / 255) as u8;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blur_keeps_flat_colour() {
        let mut data = vec![200u8; 32 * 16 * 4];
        gaussian_blur(&mut data, 32, 16, 4.0);
        assert!(data.iter().all(|value| *value >= 199));
    }

    #[test]
    fn identity_matrix_is_lossless() {
        let mut data = vec![10, 20, 30, 255, 5, 6, 7, 128];
        let original = data.clone();
        apply_color_matrix(&mut data, &crate::engine::IDENTITY);
        assert_eq!(data, original);
    }
}
