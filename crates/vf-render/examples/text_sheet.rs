//! Renders text presets to a PNG contact sheet (visual check of the text engine).
use vf_core::presets::TEXT_PRESETS;
use vf_render::TextEngine;

fn main() {
    let engine = TextEngine::new();
    let mut rows = Vec::new();
    for preset in TEXT_PRESETS.iter().take(14) {
        let mut style = preset.text_style();
        style.content = format!("{} 🔥", preset.sample);
        rows.push(engine.rasterize(&style, 1080.0, 0.5));
    }
    let width = rows.iter().map(|r| r.0).max().unwrap_or(1);
    let height: u32 = rows.iter().map(|r| r.1).sum();
    let mut sheet = tiny_skia::Pixmap::new(width, height).unwrap();
    sheet.fill(tiny_skia::Color::from_rgba8(60, 70, 90, 255));
    let mut y = 0;
    for (w, h, data) in rows {
        let pixmap = tiny_skia::Pixmap::from_vec(data, tiny_skia::IntSize::from_wh(w, h).unwrap()).unwrap();
        sheet.draw_pixmap(0, y, pixmap.as_ref(), &tiny_skia::PixmapPaint::default(), tiny_skia::Transform::identity(), None);
        y += h as i32;
    }
    let path = std::env::args().nth(1).unwrap_or_else(|| "text-sheet.png".into());
    sheet.save_png(&path).unwrap();
    println!("{path}");
}
