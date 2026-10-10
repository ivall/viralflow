use tiny_skia::{BlendMode, Color, FillRule, FilterQuality, Mask, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Shader, Transform};

use crate::engine::{hex_to_rgb, Layer, GLITCH_BANDS};
use crate::render::pixels::{apply_color_matrix, apply_grain, apply_vignette, gaussian_blur};

/// What a layer draws.
pub enum Content<'a> {
    Pixmap(&'a Pixmap),
    Shape { kind: &'a str, color: &'a str, radius: f64 },
    BlurRegion { strength: f64 },
}

pub fn blend_mode(name: &str) -> BlendMode {
    match name {
        "multiply" => BlendMode::Multiply,
        "screen" => BlendMode::Screen,
        "overlay" => BlendMode::Overlay,
        "darken" => BlendMode::Darken,
        "lighten" => BlendMode::Lighten,
        "color-dodge" => BlendMode::ColorDodge,
        "color-burn" => BlendMode::ColorBurn,
        "hard-light" => BlendMode::HardLight,
        "soft-light" => BlendMode::SoftLight,
        "difference" => BlendMode::Difference,
        "exclusion" => BlendMode::Exclusion,
        _ => BlendMode::SourceOver,
    }
}

pub fn fill_background(canvas: &mut Pixmap, hex: &str) {
    let [r, g, b] = hex_to_rgb(hex);
    canvas.fill(Color::from_rgba(r as f32, g as f32, b as f32, 1.0).unwrap_or(Color::BLACK));
}

/// Transform mapping box space (0..box_w, 0..box_h, already in output pixels) to the canvas.
fn box_transform(layer: &Layer, k: f64) -> Transform {
    let bw = (layer.box_w * k) as f32;
    let bh = (layer.box_h * k) as f32;
    let sx = (layer.scale * if layer.flip_x { -1.0 } else { 1.0 }) as f32;
    let sy = (layer.scale * if layer.flip_y { -1.0 } else { 1.0 }) as f32;
    Transform::from_translate((layer.cx * k) as f32, (layer.cy * k) as f32)
        .pre_rotate(layer.rotation as f32)
        .pre_scale(sx, sy)
        .pre_translate(-bw / 2.0, -bh / 2.0)
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Option<tiny_skia::Path> {
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    let mut pb = PathBuilder::new();
    if r <= 0.01 {
        pb.push_rect(Rect::from_xywh(x, y, w.max(0.01), h.max(0.01))?);
        return pb.finish();
    }
    // Quadratic approximations are close enough at video resolutions; use cubic arcs.
    let c = 0.552_284_7 * r;
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + c, y, x + w, y + r - c, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + c, x + w - r + c, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - c, y + h, x, y + h - r + c, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - c, x + r - c, y, x + r, y);
    pb.close();
    pb.finish()
}

fn ellipse(x: f32, y: f32, w: f32, h: f32) -> Option<tiny_skia::Path> {
    PathBuilder::from_oval(Rect::from_xywh(x, y, w.max(0.01), h.max(0.01))?)
}

/// Mask that limits drawing to the (rounded, wiped) layer box. None = no clipping needed.
fn layer_mask(layer: &Layer, k: f64, canvas_w: u32, canvas_h: u32, force: bool, band: Option<(f32, f32)>) -> Option<Mask> {
    let needs = force || layer.radius > 0.01 || layer.clip_left > 0.0001 || layer.clip_right < 0.9999 || band.is_some();
    if !needs {
        return None;
    }
    let bw = (layer.box_w * k) as f32;
    let bh = (layer.box_h * k) as f32;
    let transform = box_transform(layer, k);
    let mut mask = Mask::new(canvas_w, canvas_h)?;
    let path = rounded_rect(0.0, 0.0, bw, bh, (layer.radius * k) as f32)?;
    mask.fill_path(&path, FillRule::Winding, true, transform);
    let left = (layer.clip_left as f32 * bw).clamp(0.0, bw);
    let right = (layer.clip_right as f32 * bw).clamp(left, bw);
    let (top, bottom) = band.unwrap_or((0.0, bh));
    if left > 0.0 || right < bw || band.is_some() {
        let clip = PathBuilder::from_rect(Rect::from_ltrb(left, top, right.max(left + 0.01), bottom.max(top + 0.01))?);
        mask.intersect_path(&clip, FillRule::Winding, true, transform);
    }
    Some(mask)
}

/// Draws one layer onto the canvas. `k` converts project pixels to output pixels.
pub fn draw_layer(canvas: &mut Pixmap, layer: &Layer, content: Content, k: f64, frame: i64) {
    let (cw, ch) = (canvas.width(), canvas.height());
    match content {
        Content::BlurRegion { strength } => draw_blur_region(canvas, layer, k, strength),
        Content::Shape { kind, color, radius } => {
            let [r, g, b] = hex_to_rgb(color);
            let mut paint = Paint {
                anti_alias: true,
                shader: Shader::SolidColor(Color::from_rgba(r as f32, g as f32, b as f32, layer.opacity as f32).unwrap_or(Color::WHITE)),
                blend_mode: blend_mode(&layer.blend),
                ..Default::default()
            };
            let bw = (layer.box_w * k) as f32;
            let bh = (layer.box_h * k) as f32;
            let path = if kind == "circle" {
                ellipse(0.0, 0.0, bw, bh)
            } else {
                rounded_rect(0.0, 0.0, bw, bh, if kind == "line" { bh / 2.0 } else { (radius * k) as f32 })
            };
            let Some(path) = path else { return };
            if layer.blur > 0.3 {
                let Some(mut temp) = Pixmap::new(cw, ch) else { return };
                paint.blend_mode = BlendMode::SourceOver;
                let mask = layer_mask(layer, k, cw, ch, false, None);
                temp.fill_path(&path, &paint, FillRule::Winding, box_transform(layer, k), mask.as_ref());
                let (w, h) = (temp.width() as usize, temp.height() as usize);
                gaussian_blur(temp.data_mut(), w, h, (layer.blur * k) as f32);
                canvas.draw_pixmap(
                    0,
                    0,
                    temp.as_ref(),
                    &PixmapPaint { blend_mode: blend_mode(&layer.blend), ..Default::default() },
                    Transform::identity(),
                    None,
                );
            } else {
                let mask = layer_mask(layer, k, cw, ch, false, None);
                canvas.fill_path(&path, &paint, FillRule::Winding, box_transform(layer, k), mask.as_ref());
            }
        }
        Content::Pixmap(source) => {
            let processed = process_source(source, layer, frame);
            let image = processed.as_ref().unwrap_or(source);
            let bw = layer.box_w * k;
            let bh = layer.box_h * k;
            let to_box = Transform::from_scale((bw / image.width() as f64) as f32, (bh / image.height() as f64) as f32);
            let transform = box_transform(layer, k).pre_concat(to_box);
            let identity_like =
                (transform.sx - 1.0).abs() < 1e-4 && (transform.sy - 1.0).abs() < 1e-4 && transform.kx.abs() < 1e-6 && transform.ky.abs() < 1e-6;
            let quality = if identity_like && (transform.tx.fract().abs() < 1e-3) && (transform.ty.fract().abs() < 1e-3) {
                FilterQuality::Nearest
            } else {
                FilterQuality::Bicubic
            };
            let paint = PixmapPaint { opacity: layer.opacity as f32, blend_mode: blend_mode(&layer.blend), quality };

            if !layer.glitch.is_empty() {
                let bh_box = (layer.box_h * k) as f32;
                let band_h = bh_box / GLITCH_BANDS as f32;
                for (index, offset) in layer.glitch.iter().enumerate() {
                    let band = (index as f32 * band_h, (index as f32 + 1.0) * band_h);
                    let shifted = box_transform(layer, k).pre_translate((*offset / 100.0 * layer.box_w * k) as f32, 0.0).pre_concat(to_box);
                    let mask = layer_mask(layer, k, cw, ch, true, Some(band));
                    draw_pixmap_maybe_blurred(canvas, image, layer, k, &paint, shifted, mask.as_ref());
                }
                return;
            }
            let mask = layer_mask(layer, k, cw, ch, false, None);
            draw_pixmap_maybe_blurred(canvas, image, layer, k, &paint, transform, mask.as_ref());
        }
    }
}

fn draw_pixmap_maybe_blurred(canvas: &mut Pixmap, image: &Pixmap, layer: &Layer, k: f64, paint: &PixmapPaint, transform: Transform, mask: Option<&Mask>) {
    if layer.blur > 0.3 {
        let Some(mut temp) = Pixmap::new(canvas.width(), canvas.height()) else { return };
        temp.draw_pixmap(0, 0, image.as_ref(), &PixmapPaint { quality: FilterQuality::Bilinear, ..Default::default() }, transform, mask);
        let (w, h) = (temp.width() as usize, temp.height() as usize);
        gaussian_blur(temp.data_mut(), w, h, (layer.blur * k) as f32);
        canvas.draw_pixmap(
            0,
            0,
            temp.as_ref(),
            &PixmapPaint { opacity: paint.opacity, blend_mode: paint.blend_mode, quality: FilterQuality::Nearest },
            Transform::identity(),
            None,
        );
        return;
    }
    if paint.quality == FilterQuality::Nearest && mask.is_none() && paint.blend_mode == BlendMode::SourceOver && paint.opacity >= 0.999 && is_opaque(image) {
        copy_opaque(canvas, image, transform.tx.round() as i32, transform.ty.round() as i32);
        return;
    }
    canvas.draw_pixmap(0, 0, image.as_ref(), paint, transform, mask);
}

fn is_opaque(image: &Pixmap) -> bool {
    // Video frames are opaque; checking a sparse sample is enough to pick the fast path.
    let data = image.data();
    let step = (data.len() / 4 / 64).max(1) * 4;
    data.iter().skip(3).step_by(step).all(|alpha| *alpha == 255)
}

/// Fast path for untransformed opaque frames: plain row copies.
fn copy_opaque(canvas: &mut Pixmap, image: &Pixmap, x: i32, y: i32) {
    let (cw, ch) = (canvas.width() as i32, canvas.height() as i32);
    let (iw, ih) = (image.width() as i32, image.height() as i32);
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + iw).min(cw);
    let y1 = (y + ih).min(ch);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let src = image.data();
    let dst = canvas.data_mut();
    for row in y0..y1 {
        let s = (((row - y) * iw + (x0 - x)) * 4) as usize;
        let d = ((row * cw + x0) * 4) as usize;
        let len = ((x1 - x0) * 4) as usize;
        dst[d..d + len].copy_from_slice(&src[s..s + len]);
    }
}

/// Colour matrix, vignette and grain are applied in source space before drawing.
fn process_source(source: &Pixmap, layer: &Layer, frame: i64) -> Option<Pixmap> {
    if layer.matrix.is_none() && layer.vignette <= 0.001 && layer.grain <= 0.0 && !layer.scanlines {
        return None;
    }
    let mut copy = source.clone();
    let (w, h) = (copy.width() as usize, copy.height() as usize);
    if let Some(matrix) = &layer.matrix {
        apply_color_matrix(copy.data_mut(), matrix);
    }
    if layer.vignette > 0.001 {
        apply_vignette(copy.data_mut(), w, h, layer.vignette as f32);
    }
    if layer.grain > 0.0 || layer.scanlines {
        apply_grain(copy.data_mut(), w, frame, layer.grain as f32, layer.scanlines);
    }
    Some(copy)
}

/// Blurs whatever is already on the canvas inside the layer box.
fn draw_blur_region(canvas: &mut Pixmap, layer: &Layer, k: f64, strength: f64) {
    let (cw, ch) = (canvas.width(), canvas.height());
    let Some(mask) = layer_mask(layer, k, cw, ch, true, None) else { return };
    let transform = box_transform(layer, k);
    let bw = (layer.box_w * k) as f32;
    let bh = (layer.box_h * k) as f32;
    let corners = [(0.0, 0.0), (bw, 0.0), (bw, bh), (0.0, bh)].map(|(x, y)| {
        let mut point = tiny_skia::Point::from_xy(x, y);
        transform.map_point(&mut point);
        point
    });
    let pad = (strength * k * 3.0) as f32;
    let left = (corners.iter().map(|p| p.x).fold(f32::MAX, f32::min) - pad).max(0.0) as u32;
    let top = (corners.iter().map(|p| p.y).fold(f32::MAX, f32::min) - pad).max(0.0) as u32;
    let right = ((corners.iter().map(|p| p.x).fold(f32::MIN, f32::max) + pad).min(cw as f32)) as u32;
    let bottom = ((corners.iter().map(|p| p.y).fold(f32::MIN, f32::max) + pad).min(ch as f32)) as u32;
    if right <= left || bottom <= top {
        return;
    }
    let Some(rect) = tiny_skia::IntRect::from_ltrb(left as i32, top as i32, right as i32, bottom as i32) else { return };
    let Some(mut region) = canvas.clone_rect(rect) else { return };
    let (w, h) = (region.width() as usize, region.height() as usize);
    gaussian_blur(region.data_mut(), w, h, (strength * k) as f32);
    canvas.draw_pixmap(
        left as i32,
        top as i32,
        region.as_ref(),
        &PixmapPaint { opacity: layer.opacity as f32, ..Default::default() },
        Transform::identity(),
        Some(&mask),
    );
}
