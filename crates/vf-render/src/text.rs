//! Text clips: layout (wrapping, alignment, line height) and rasterisation
//! (background plate, highlighted word, shadow, outline, fill). The layout and
//! the drawing order are those of the former browser renderer (text.ts).

use std::sync::Mutex;

use cosmic_text::{Attrs, Buffer, Command, Family, FontSystem, Metrics, Shaping, Style, SwashCache, SwashContent, Weight};
use tiny_skia::{FillRule, LineJoin, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke, Transform};
use vf_core::engine::color::hex_to_rgb;
use vf_core::model::{HighlightMode, TextAlign, TextStyle};

/// Fonts that ship with ViralFlow (OFL licensed, see assets/licenses).
const BUNDLED_FONTS: &[&[u8]] = &[
    include_bytes!("../../../assets/fonts/Inter.ttf"),
    include_bytes!("../../../assets/fonts/Manrope.ttf"),
    include_bytes!("../../../assets/fonts/DMSans.ttf"),
    include_bytes!("../../../assets/fonts/Montserrat.ttf"),
    include_bytes!("../../../assets/fonts/Poppins-Regular.ttf"),
    include_bytes!("../../../assets/fonts/Poppins-Bold.ttf"),
    include_bytes!("../../../assets/fonts/Rubik.ttf"),
    include_bytes!("../../../assets/fonts/RobotoCondensed.ttf"),
    include_bytes!("../../../assets/fonts/BarlowCondensed-Regular.ttf"),
    include_bytes!("../../../assets/fonts/BarlowCondensed-Bold.ttf"),
    include_bytes!("../../../assets/fonts/Oswald.ttf"),
    include_bytes!("../../../assets/fonts/Anton-Regular.ttf"),
    include_bytes!("../../../assets/fonts/BebasNeue-Regular.ttf"),
    include_bytes!("../../../assets/fonts/ArchivoBlack-Regular.ttf"),
];

pub const BUNDLED_FAMILIES: &[&str] = &[
    "Inter",
    "Manrope",
    "DM Sans",
    "Montserrat",
    "Poppins",
    "Rubik",
    "Roboto Condensed",
    "Barlow Condensed",
    "Oswald",
    "Anton",
    "Bebas Neue",
    "Archivo Black",
];

/// A shaped line: glyphs with their x positions (letter spacing applied).
struct ShapedLine {
    text: String,
    width: f32,
    glyphs: Vec<PlacedGlyph>,
}

struct PlacedGlyph {
    start: usize,
    x: f32,
    w: f32,
    cache_key: cosmic_text::CacheKey,
    y_offset: f32,
}

/// Layout of a text clip at a given scale, in output pixels.
pub struct Layout {
    lines: Vec<ShapedLine>,
    pub block_width: f32,
    pub line_height: f32,
    pub ascent: f32,
    pub descent: f32,
    pub margin: f32,
    pub padding: f32,
    pub width: u32,
    pub height: u32,
}

struct Engine {
    fonts: FontSystem,
    swash: SwashCache,
}

/// Thread-safe text engine shared by the preview and the export.
pub struct TextEngine {
    inner: Mutex<Engine>,
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TextEngine {
    /// Loads the bundled fonts and the system's fonts (for emoji and fallbacks).
    pub fn new() -> Self {
        let mut fonts = FontSystem::new();
        for data in BUNDLED_FONTS {
            fonts.db_mut().load_font_data(data.to_vec());
        }
        TextEngine { inner: Mutex::new(Engine { fonts, swash: SwashCache::new() }) }
    }

    /// Adds a font file (custom fonts chosen by the user).
    pub fn load_font_file(&self, path: &std::path::Path) -> bool {
        let Ok(data) = std::fs::read(path) else { return false };
        if let Ok(mut engine) = self.inner.lock() {
            engine.fonts.db_mut().load_font_data(data);
            return true;
        }
        false
    }

    /// Font families available on this system (bundled ones first).
    pub fn families(&self) -> Vec<String> {
        let Ok(engine) = self.inner.lock() else { return vec![] };
        let mut names: Vec<String> = engine.fonts.db().faces().filter_map(|face| face.families.first().map(|(name, _)| name.clone())).collect();
        names.sort_by_key(|name| name.to_lowercase());
        names.dedup();
        let mut result: Vec<String> = BUNDLED_FAMILIES.iter().map(|s| s.to_string()).collect();
        result.extend(names.into_iter().filter(|name| !BUNDLED_FAMILIES.contains(&name.as_str())));
        result
    }

    /// Size of the text box in project pixels.
    pub fn measure(&self, style: &TextStyle, canvas_width: f64) -> (f64, f64) {
        let Ok(mut engine) = self.inner.lock() else { return (1.0, 1.0) };
        let layout = engine.layout(style, canvas_width, 1.0);
        (layout.width as f64, layout.height as f64)
    }

    /// Premultiplied RGBA raster at `scale` × project pixels.
    pub fn rasterize(&self, style: &TextStyle, canvas_width: f64, scale: f64) -> (u32, u32, Vec<u8>) {
        let Ok(mut engine) = self.inner.lock() else { return (1, 1, vec![0; 4]) };
        engine.rasterize(style, canvas_width, scale)
    }
}

fn attrs(style: &TextStyle) -> Attrs<'_> {
    Attrs::new().family(Family::Name(&style.font)).weight(Weight(style.weight.clamp(1.0, 1000.0) as u16)).style(if style.italic {
        Style::Italic
    } else {
        Style::Normal
    })
}

impl Engine {
    fn shape(&mut self, style: &TextStyle, text: &str, size: f32, spacing: f32) -> ShapedLine {
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size.max(1.0), size.max(1.0) * 1.2));
        buffer.set_size(None, None);
        buffer.set_text(text, &attrs(style), Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        let mut glyphs = Vec::new();
        let mut width = 0.0f32;
        for run in buffer.layout_runs() {
            for (index, glyph) in run.glyphs.iter().enumerate() {
                let physical = glyph.physical((0.0, 0.0), 1.0);
                let x = glyph.x + glyph.font_size * glyph.x_offset + spacing * index as f32;
                glyphs.push(PlacedGlyph { start: glyph.start, x, w: glyph.w, cache_key: physical.cache_key, y_offset: glyph.font_size * glyph.y_offset });
            }
            width = width.max(run.line_w + spacing * run.glyphs.len() as f32);
        }
        ShapedLine { text: text.to_string(), width, glyphs }
    }

    fn measure(&mut self, style: &TextStyle, text: &str, size: f32, spacing: f32) -> f32 {
        self.shape(style, text, size, spacing).width
    }

    /// Greedy word wrap with hard breaks for words wider than the box (as text.ts).
    fn wrap(&mut self, style: &TextStyle, content: &str, max_width: f32, size: f32, spacing: f32) -> Vec<String> {
        let mut lines = Vec::new();
        for paragraph in content.split('\n') {
            let paragraph = paragraph.trim_end_matches('\r');
            let mut line = String::new();
            for word in split_keep_whitespace(paragraph) {
                let candidate = format!("{line}{word}");
                if line.is_empty() || self.measure(style, candidate.trim_end(), size, spacing) <= max_width {
                    line = candidate;
                    continue;
                }
                lines.push(line.trim_end().to_string());
                line = word.trim_start().to_string();
            }
            let mut pending = vec![line.trim_end().to_string()];
            while let Some(current) = pending.pop() {
                let chars: Vec<char> = current.chars().collect();
                if chars.len() <= 1 || self.measure(style, &current, size, spacing) <= max_width {
                    lines.push(current);
                    continue;
                }
                let mut cut = chars.len() - 1;
                while cut > 1 && self.measure(style, &chars[..cut].iter().collect::<String>(), size, spacing) > max_width {
                    cut -= 1;
                }
                pending.push(chars[cut..].iter().collect());
                lines.push(chars[..cut].iter().collect());
            }
        }
        lines
    }

    fn metrics(&mut self, style: &TextStyle, size: f32) -> (f32, f32) {
        // Ascent and descent of the font the text actually uses ("ÁgjÉ" probe).
        let probe = self.shape(style, "ÁgjÉ", size, 0.0);
        let font_id = probe.glyphs.first().map(|g| g.cache_key.font_id);
        let weight = Weight(style.weight.clamp(1.0, 1000.0) as u16);
        if let Some(font) = font_id.and_then(|id| self.fonts.get_font(id, weight)) {
            let metrics = font.as_swash().metrics(&[]).scale(size);
            if metrics.ascent > 0.0 {
                return (metrics.ascent, metrics.descent.abs());
            }
        }
        (size * 0.8, size * 0.2)
    }

    fn layout(&mut self, style: &TextStyle, canvas_width: f64, scale: f64) -> Layout {
        let scale = scale as f32;
        let size = style.size as f32 * scale;
        let spacing = style.letter_spacing as f32 * scale;
        let content = style.display_text();
        let max_width = ((style.max_width / 100.0) as f32 * canvas_width as f32 * scale).max(20.0);
        let texts = self.wrap(style, if content.is_empty() { " " } else { &content }, max_width, size, spacing);
        let lines: Vec<ShapedLine> = texts.iter().map(|text| self.shape(style, text, size, spacing)).collect();
        let (ascent, descent) = self.metrics(style, size);
        let line_height = (ascent + descent).max(size * style.line_height as f32);
        let block_width = lines.iter().map(|l| l.width).fold(1.0f32, f32::max);
        let padding = if style.background_opacity > 0.0 { style.background_padding as f32 * scale } else { 0.0 };
        let shadow = if style.shadow_opacity > 0.0 { (style.shadow_blur + style.shadow_distance) as f32 * scale } else { 0.0 };
        // A highlight box may stick out past the glyphs of the first or last word.
        let highlight_room = if style.has_highlight() && style.highlight_mode == HighlightMode::Box { size * 0.22 } else { 0.0 };
        let margin = ((style.stroke_width as f32 * scale).max(shadow).max(highlight_room) + 2.0 * scale).ceil();
        let width = (block_width + 2.0 * padding + 2.0 * margin).ceil().max(1.0) as u32;
        let height = (lines.len() as f32 * line_height + 2.0 * padding + 2.0 * margin).ceil().max(1.0) as u32;
        Layout { lines, block_width, line_height, ascent, descent, margin, padding, width, height }
    }

    fn rasterize(&mut self, style: &TextStyle, canvas_width: f64, scale: f64) -> (u32, u32, Vec<u8>) {
        let layout = self.layout(style, canvas_width, scale);
        let Some(mut canvas) = Pixmap::new(layout.width, layout.height) else { return (1, 1, vec![0; 4]) };
        let s = scale as f32;
        let size = style.size as f32 * s;
        let inset = layout.margin;
        if layout.padding > 0.0 {
            let w = layout.block_width + 2.0 * layout.padding;
            let h = layout.lines.len() as f32 * layout.line_height + 2.0 * layout.padding;
            if let Some(path) = rounded_rect(inset, inset, w, h, style.background_radius as f32 * s) {
                canvas.fill_path(&path, &paint(&style.background_color, style.background_opacity), FillRule::Winding, Transform::identity(), None);
            }
        }
        let origin_x = inset + layout.padding;
        let origin_y = inset + layout.padding;
        let highlight = style.has_highlight();
        let mut word_index = 0i64;
        let mut text_path = PathBuilder::new();
        let mut word_path = PathBuilder::new();
        let mut color_glyphs: Vec<(f32, f32, cosmic_text::CacheKey)> = Vec::new();
        let mut boxes: Vec<(f32, f32, f32, f32)> = Vec::new();
        for (index, line) in layout.lines.iter().enumerate() {
            let x = match style.align {
                TextAlign::Left => origin_x,
                TextAlign::Right => origin_x + layout.block_width - line.width,
                TextAlign::Center => origin_x + (layout.block_width - line.width) / 2.0,
            };
            let y = origin_y + index as f32 * layout.line_height + (layout.line_height - layout.ascent - layout.descent) / 2.0 + layout.ascent;
            // The highlighted word on this line (words are counted across lines).
            let mut word: Option<(usize, usize)> = None;
            if highlight {
                for (start, end) in word_ranges(&line.text) {
                    if word_index == style.highlight_index {
                        word = Some((start, end));
                    }
                    word_index += 1;
                }
            }
            if let (Some((start, end)), HighlightMode::Box) = (word, style.highlight_mode) {
                let glyphs: Vec<&PlacedGlyph> = line.glyphs.iter().filter(|g| g.start >= start && g.start < end).collect();
                if let (Some(first), Some(last)) = (glyphs.iter().map(|g| g.x).reduce(f32::min), glyphs.iter().map(|g| g.x + g.w).reduce(f32::max)) {
                    let top = y - layout.ascent + size * 0.1;
                    let bottom = y + layout.descent * 0.55;
                    boxes.push((x + first - size * 0.16, top, last - first + size * 0.32, bottom - top));
                }
            }
            for glyph in &line.glyphs {
                let gx = x + glyph.x;
                let gy = y - glyph.y_offset;
                if let Some(image) = self.swash.get_image(&mut self.fonts, glyph.cache_key) {
                    if image.content == SwashContent::Color {
                        color_glyphs.push((gx, gy, glyph.cache_key));
                        continue;
                    }
                }
                let in_word = word.is_some_and(|(start, end)| glyph.start >= start && glyph.start < end);
                if let Some(commands) = self.swash.get_outline_commands(&mut self.fonts, glyph.cache_key) {
                    append_outline(&mut text_path, commands, gx, gy);
                    if in_word {
                        append_outline(&mut word_path, commands, gx, gy);
                    }
                }
            }
        }
        let text_shape = text_path.finish();
        let word_shape = word_path.finish();

        for (x, y, w, h) in boxes {
            if let Some(path) = rounded_rect(x, y, w, h, size * 0.18) {
                canvas.fill_path(&path, &paint(&style.highlight_color, 1.0), FillRule::Winding, Transform::identity(), None);
            }
        }
        let stroke = (style.stroke_width > 0.0).then(|| Stroke {
            width: (style.stroke_width * 2.0) as f32 * s,
            line_join: LineJoin::Round,
            miter_limit: 2.0,
            ..Default::default()
        });
        if let Some(shape) = &text_shape {
            if style.shadow_opacity > 0.0 {
                // The shadow follows the outline when there is one, else the letters.
                if let Some(mut shadow) = Pixmap::new(layout.width, layout.height) {
                    let shadow_paint = paint(&style.shadow_color, style.shadow_opacity);
                    let offset = Transform::from_translate(0.0, style.shadow_distance as f32 * s);
                    match &stroke {
                        Some(stroke) => shadow.stroke_path(shape, &shadow_paint, stroke, offset, None),
                        None => shadow.fill_path(shape, &shadow_paint, FillRule::Winding, offset, None),
                    }
                    let sigma = style.shadow_blur as f32 * s / 2.0;
                    let (w, h) = (shadow.width() as usize, shadow.height() as usize);
                    gaussian_blur(shadow.data_mut(), w, h, sigma);
                    canvas.draw_pixmap(0, 0, shadow.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
                }
            }
            if let Some(stroke) = &stroke {
                canvas.stroke_path(shape, &paint(&style.stroke_color, 1.0), stroke, Transform::identity(), None);
            }
            canvas.fill_path(shape, &paint(&style.color, 1.0), FillRule::Winding, Transform::identity(), None);
        }
        let word_color = if style.highlight_mode == HighlightMode::Color { style.highlight_color.as_str() } else { style.highlight_text_color.as_str() };
        if let (Some(shape), false) = (&word_shape, word_color.is_empty()) {
            canvas.fill_path(shape, &paint(word_color, 1.0), FillRule::Winding, Transform::identity(), None);
        }
        for (x, y, key) in color_glyphs {
            if let Some(image) = self.swash.get_image(&mut self.fonts, key).clone() {
                draw_color_glyph(&mut canvas, &image, x, y);
            }
        }
        let (w, h) = (canvas.width(), canvas.height());
        (w, h, canvas.take())
    }
}

/// Splits keeping runs of whitespace as their own pieces ("a  b" → "a", "  ", "b").
fn split_keep_whitespace(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_space = None;
    for (index, c) in text.char_indices() {
        let space = c.is_whitespace();
        if in_space.is_some_and(|s| s != space) {
            parts.push(&text[start..index]);
            start = index;
        }
        in_space = Some(space);
    }
    if start < text.len() {
        parts.push(&text[start..]);
    }
    parts
}

/// Byte ranges of the whitespace-separated words of a line.
fn word_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = None;
    for (index, c) in text.char_indices() {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(index),
            (true, Some(s)) => {
                ranges.push((s, index));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        ranges.push((s, text.len()));
    }
    ranges
}

fn append_outline(path: &mut PathBuilder, commands: &[Command], x: f32, y: f32) {
    // Outlines are y-up around the glyph origin.
    for command in commands {
        match *command {
            Command::MoveTo(p) => path.move_to(x + p.x, y - p.y),
            Command::LineTo(p) => path.line_to(x + p.x, y - p.y),
            Command::CurveTo(a, b, p) => path.cubic_to(x + a.x, y - a.y, x + b.x, y - b.y, x + p.x, y - p.y),
            Command::QuadTo(a, p) => path.quad_to(x + a.x, y - a.y, x + p.x, y - p.y),
            Command::Close => path.close(),
        }
    }
}

fn draw_color_glyph(canvas: &mut Pixmap, image: &cosmic_text::SwashImage, x: f32, y: f32) {
    let (w, h) = (image.placement.width, image.placement.height);
    let Some(mut glyph) = Pixmap::new(w.max(1), h.max(1)) else { return };
    for (dst, src) in glyph.data_mut().chunks_exact_mut(4).zip(image.data.chunks_exact(4)) {
        let a = src[3] as u32;
        dst[0] = (src[0] as u32 * a / 255) as u8;
        dst[1] = (src[1] as u32 * a / 255) as u8;
        dst[2] = (src[2] as u32 * a / 255) as u8;
        dst[3] = src[3];
    }
    let left = (x + image.placement.left as f32).round() as i32;
    let top = (y - image.placement.top as f32).round() as i32;
    canvas.draw_pixmap(left, top, glyph.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
}

fn paint(hex: &str, opacity: f64) -> Paint<'static> {
    let [r, g, b] = hex_to_rgb(hex);
    let mut paint = Paint::default();
    paint.set_color_rgba8((r * 255.0).round() as u8, (g * 255.0).round() as u8, (b * 255.0).round() as u8, (opacity.clamp(0.0, 1.0) * 255.0).round() as u8);
    paint.anti_alias = true;
    paint
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Option<tiny_skia::Path> {
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    let mut pb = PathBuilder::new();
    if r <= 0.01 {
        pb.push_rect(Rect::from_xywh(x, y, w.max(0.01), h.max(0.01))?);
        return pb.finish();
    }
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

/// Three box-blur passes approximate a gaussian with the given sigma.
pub fn gaussian_blur(data: &mut [u8], width: usize, height: usize, sigma: f32) {
    if sigma < 0.5 || width == 0 || height == 0 {
        return;
    }
    let mut buffer = vec![0u8; data.len()];
    for radius in box_radii(sigma) {
        blur_pass(data, &mut buffer, width, height, radius, true);
        blur_pass(&buffer, data, width, height, radius, false);
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
    [0, 1, 2].map(|i| ((size(i) - 1) / 2).max(0) as usize)
}

/// One box blur along rows (`horizontal`) or columns.
fn blur_pass(src: &[u8], dst: &mut [u8], width: usize, height: usize, radius: usize, horizontal: bool) {
    if radius == 0 {
        dst.copy_from_slice(src);
        return;
    }
    let window = (2 * radius + 1) as u32;
    let (outer, inner) = if horizontal { (height, width) } else { (width, height) };
    let index = |o: usize, i: isize| -> usize {
        let i = i.clamp(0, inner as isize - 1) as usize;
        if horizontal {
            (o * width + i) * 4
        } else {
            (i * width + o) * 4
        }
    };
    for o in 0..outer {
        let mut sum = [0u32; 4];
        for i in -(radius as isize)..=(radius as isize) {
            let p = index(o, i);
            for (c, total) in sum.iter_mut().enumerate() {
                *total += src[p + c] as u32;
            }
        }
        for i in 0..inner {
            let out = index(o, i as isize);
            for (c, total) in sum.iter().enumerate() {
                dst[out + c] = (total / window) as u8;
            }
            let add = index(o, i as isize + radius as isize + 1);
            let remove = index(o, i as isize - radius as isize);
            for (c, total) in sum.iter_mut().enumerate() {
                *total = *total + src[add + c] as u32 - src[remove + c] as u32;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_words_and_spaces() {
        assert_eq!(split_keep_whitespace("one  two three"), vec!["one", "  ", "two", " ", "three"]);
        assert_eq!(word_ranges(" a bc "), vec![(1, 2), (3, 5)]);
    }

    #[test]
    fn wraps_long_text_and_grows_with_stroke() {
        let engine = TextEngine::new();
        let style = TextStyle { content: "This is a long caption that has to wrap onto several lines".into(), max_width: 50.0, ..Default::default() };
        let (w, h) = engine.measure(&style, 1080.0);
        assert!(w <= 1080.0 * 0.5 + 40.0, "wrapped width {w}");
        assert!(h > style.size * 2.0, "several lines tall, got {h}");
        let outlined = TextStyle { stroke_width: 10.0, ..style.clone() };
        let (w2, h2) = engine.measure(&outlined, 1080.0);
        assert!(w2 > w && h2 > h);
    }

    #[test]
    fn rasterises_visible_text() {
        let engine = TextEngine::new();
        let style = TextStyle { content: "Hi".into(), color: "#ff0000".into(), ..Default::default() };
        let (w, h, data) = engine.rasterize(&style, 1080.0, 1.0);
        assert_eq!(data.len(), (w * h * 4) as usize);
        let red = data.chunks_exact(4).filter(|p| p[0] > 200 && p[1] < 50 && p[3] > 200).count();
        assert!(red > 200, "red glyph pixels: {red}");
    }

    #[test]
    fn highlight_box_is_drawn() {
        let engine = TextEngine::new();
        let style = TextStyle {
            content: "one two".into(),
            highlight_mode: HighlightMode::Box,
            highlight_index: 1,
            highlight_color: "#00ff00".into(),
            ..Default::default()
        };
        let (_, _, data) = engine.rasterize(&style, 1080.0, 1.0);
        let green = data.chunks_exact(4).filter(|p| p[1] > 200 && p[0] < 50 && p[2] < 50).count();
        assert!(green > 500, "green box pixels: {green}");
    }

    #[test]
    fn blur_spreads_a_dot() {
        let (w, h) = (21, 21);
        let mut data = vec![0u8; w * h * 4];
        let center = (10 * w + 10) * 4;
        data[center..center + 4].copy_from_slice(&[255; 4]);
        gaussian_blur(&mut data, w, h, 2.0);
        assert!(data[center + 3] < 255 && data[((10 * w) + 12) * 4 + 3] > 0);
    }
}
