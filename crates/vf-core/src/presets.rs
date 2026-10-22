//! New projects, tracks and clips, and the built-in text and shape looks.

use crate::engine::math::uid;
use crate::model::*;

pub const TRACK_COLORS: [(ItemKind, &str); 5] =
    [(ItemKind::Video, "#4f7cff"), (ItemKind::Image, "#8b6cf6"), (ItemKind::Audio, "#2fb38a"), (ItemKind::Text, "#e2a03f"), (ItemKind::Shape, "#d16ba5")];

pub fn track_label(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Video => "Video",
        TrackKind::Text => "Text",
        TrackKind::Audio => "Audio",
    }
}

pub struct AspectPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub width: u32,
    pub height: u32,
    pub hint: &'static str,
}

pub const ASPECT_PRESETS: &[AspectPreset] = &[
    AspectPreset { id: "9:16", label: "9:16", width: 1080, height: 1920, hint: "TikTok, Reels, Shorts" },
    AspectPreset { id: "16:9", label: "16:9", width: 1920, height: 1080, hint: "YouTube" },
    AspectPreset { id: "1:1", label: "1:1", width: 1080, height: 1080, hint: "Square post" },
    AspectPreset { id: "4:5", label: "4:5", width: 1080, height: 1350, hint: "Instagram post" },
    AspectPreset { id: "3:4", label: "3:4", width: 1080, height: 1440, hint: "Classic portrait" },
    AspectPreset { id: "21:9", label: "21:9", width: 2520, height: 1080, hint: "Cinematic" },
];

pub fn aspect_label(width: u32, height: u32) -> String {
    let ratio = width as f64 / height.max(1) as f64;
    ASPECT_PRESETS
        .iter()
        .find(|p| (p.width as f64 / p.height as f64 - ratio).abs() < 0.01)
        .map(|p| p.label.to_string())
        .unwrap_or_else(|| format!("{width}×{height}"))
}

pub fn now_iso() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

pub fn create_project(name: &str, width: u32, height: u32, fps: u32) -> Project {
    let now = now_iso();
    let name = name.trim();
    Project {
        version: 2,
        id: uid("project"),
        name: if name.is_empty() { "Untitled project".into() } else { name.into() },
        created_at: now.clone(),
        updated_at: now,
        width,
        height,
        fps,
        background: "#000000".into(),
        tracks: vec![],
        assets: vec![],
        export: ExportSettings { fps, resolution: width.min(height), ..Default::default() },
        beats: vec![],
        beat_options: Default::default(),
        origin: None,
        extra: Default::default(),
    }
}

pub fn create_track(kind: TrackKind, existing: &[Track]) -> Track {
    let count = existing.iter().filter(|track| track.kind == kind).count() + 1;
    let label = track_label(kind);
    Track { id: uid("track"), kind, name: if count > 1 { format!("{label} {count}") } else { label.into() }, ..Default::default() }
}

/// A clip for `asset` (start, duration and id are set when it is placed).
pub fn media_item(asset: &MediaAsset, project: &Project) -> TimelineItem {
    let mut item = TimelineItem {
        name: asset.name.clone(),
        asset_id: Some(asset.id.clone()),
        source_in: Some(0.0),
        speed: Some(1.0),
        volume: Some(1.0),
        ..Default::default()
    };
    match asset.kind {
        MediaKind::Audio => {
            item.kind = ItemKind::Audio;
            item.duration = asset.duration.filter(|d| *d > 0.0).unwrap_or(5.0);
        }
        MediaKind::Video | MediaKind::Image => {
            let landscape_on_portrait = asset.width.unwrap_or(1.0) / asset.height.unwrap_or(1.0) > project.width as f64 / project.height as f64 + 0.01;
            item.kind = if asset.kind == MediaKind::Image { ItemKind::Image } else { ItemKind::Video };
            item.duration = if asset.kind == MediaKind::Image { 4.0 } else { asset.duration.filter(|d| *d > 0.0).unwrap_or(5.0) };
            item.fit = Some(if asset.kind == MediaKind::Video && !landscape_on_portrait { Fit::Cover } else { Fit::Contain });
            item.transform = Some(Transform::default());
        }
    }
    item
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetCategory {
    Highlight,
    Captions,
    Titles,
    Boxes,
    Glow,
}

impl PresetCategory {
    pub const ALL: [PresetCategory; 5] =
        [PresetCategory::Highlight, PresetCategory::Captions, PresetCategory::Titles, PresetCategory::Boxes, PresetCategory::Glow];

    pub fn label(self) -> &'static str {
        match self {
            PresetCategory::Highlight => "Highlight",
            PresetCategory::Captions => "Captions",
            PresetCategory::Titles => "Titles",
            PresetCategory::Boxes => "Boxes",
            PresetCategory::Glow => "Glow",
        }
    }
}

pub struct TextPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub category: PresetCategory,
    /// Text a new clip starts with, and what the preview card shows.
    pub sample: &'static str,
    pub style: fn(&mut TextStyle),
    pub anim_in: Option<(AnimationInKind, f64)>,
}

impl TextPreset {
    pub fn text_style(&self) -> TextStyle {
        let mut style = TextStyle::default();
        (self.style)(&mut style);
        style
    }

    pub fn animation(&self) -> Option<Animation<AnimationInKind>> {
        self.anim_in.map(|(kind, duration)| Animation { kind, duration })
    }
}

fn heavy_outline(s: &mut TextStyle) {
    s.font = "Montserrat".into();
    s.weight = 900.0;
    s.size = 92.0;
    s.uppercase = true;
    s.stroke_color = "#000000".into();
    s.stroke_width = 9.0;
    s.shadow_color = "#000000".into();
    s.shadow_opacity = 0.9;
    s.shadow_blur = 0.0;
    s.shadow_distance = 6.0;
}

fn font(s: &mut TextStyle, family: &str, weight: f64, size: f64) {
    s.font = family.into();
    s.weight = weight;
    s.size = size;
}

fn shadow(s: &mut TextStyle, color: &str, opacity: f64, blur: f64, distance: f64) {
    s.shadow_color = color.into();
    s.shadow_opacity = opacity;
    s.shadow_blur = blur;
    s.shadow_distance = distance;
}

fn stroke(s: &mut TextStyle, color: &str, width: f64) {
    s.stroke_color = color.into();
    s.stroke_width = width;
}

fn plate(s: &mut TextStyle, color: &str, opacity: f64, padding: f64, radius: f64) {
    s.background_color = color.into();
    s.background_opacity = opacity;
    s.background_padding = padding;
    s.background_radius = radius;
}

fn highlight(s: &mut TextStyle, mode: HighlightMode, color: &str, index: i64) {
    s.highlight_mode = mode;
    s.highlight_color = color.into();
    s.highlight_index = index;
}

use AnimationInKind as In;
use PresetCategory as C;

/// Each preset looks clearly different; font-only variants are left to the inspector.
pub const TEXT_PRESETS: &[TextPreset] = &[
    // Highlight: captions light up the word being spoken
    TextPreset {
        id: "hl-yellow",
        label: "Yellow word",
        category: C::Highlight,
        sample: "THIS CHANGED EVERYTHING",
        anim_in: Some((In::Pop, 0.18)),
        style: |s| {
            heavy_outline(s);
            highlight(s, HighlightMode::Color, "#ffe14d", 1);
        },
    },
    TextPreset {
        id: "hl-green",
        label: "Green word",
        category: C::Highlight,
        sample: "you need to hear this",
        anim_in: Some((In::Pop, 0.18)),
        style: |s| {
            font(s, "Poppins", 700.0, 76.0);
            shadow(s, "#000000", 0.85, 16.0, 2.0);
            highlight(s, HighlightMode::Color, "#4ef08a", 3);
        },
    },
    TextPreset {
        id: "hl-box-purple",
        label: "Purple box",
        category: C::Highlight,
        sample: "WATCH THIS PART",
        anim_in: Some((In::Pop, 0.18)),
        style: |s| {
            font(s, "Montserrat", 900.0, 88.0);
            s.uppercase = true;
            shadow(s, "#000000", 0.6, 12.0, 3.0);
            highlight(s, HighlightMode::Box, "#7c3aed", 1);
        },
    },
    TextPreset {
        id: "hl-box-yellow",
        label: "Yellow box",
        category: C::Highlight,
        sample: "the secret is simple",
        anim_in: Some((In::Pop, 0.18)),
        style: |s| {
            font(s, "DM Sans", 800.0, 78.0);
            shadow(s, "#000000", 0.6, 12.0, 3.0);
            highlight(s, HighlightMode::Box, "#ffe14d", 1);
            s.highlight_text_color = "#111111".into();
        },
    },
    TextPreset {
        id: "hl-pink",
        label: "Pink on dark",
        category: C::Highlight,
        sample: "and then she said",
        anim_in: Some((In::Fade, 0.18)),
        style: |s| {
            font(s, "DM Sans", 700.0, 66.0);
            plate(s, "#000000", 0.72, 18.0, 14.0);
            highlight(s, HighlightMode::Color, "#ff4fa3", 3);
        },
    },
    // Captions: readable over any footage
    TextPreset {
        id: "hormozi",
        label: "Punchy",
        category: C::Captions,
        sample: "NOBODY TALKS ABOUT THIS",
        anim_in: Some((In::Pop, 0.25)),
        style: heavy_outline,
    },
    TextPreset {
        id: "bold",
        label: "Bold outline",
        category: C::Captions,
        sample: "here is the thing",
        anim_in: Some((In::Pop, 0.25)),
        style: |s| {
            font(s, "Manrope", 800.0, 80.0);
            stroke(s, "#000000", 6.0);
        },
    },
    TextPreset {
        id: "caption-soft",
        label: "Soft shadow",
        category: C::Captions,
        sample: "wait until the end",
        anim_in: Some((In::Typewriter, 0.6)),
        style: |s| {
            font(s, "Inter", 800.0, 72.0);
            shadow(s, "#000000", 0.85, 18.0, 0.0);
        },
    },
    TextPreset {
        id: "caption",
        label: "Boxed",
        category: C::Captions,
        sample: "let me explain",
        anim_in: Some((In::Fade, 0.3)),
        style: |s| {
            font(s, "DM Sans", 700.0, 64.0);
            plate(s, "#000000", 0.75, 18.0, 14.0);
        },
    },
    TextPreset { id: "plain", label: "Plain", category: C::Captions, sample: "Your text", anim_in: None, style: |s| font(s, "Inter", 700.0, 72.0) },
    // Titles: hooks and headings
    TextPreset {
        id: "beast",
        label: "Challenge",
        category: C::Titles,
        sample: "24 HOUR CHALLENGE",
        anim_in: Some((In::Bounce, 0.45)),
        style: |s| {
            font(s, "Archivo Black", 400.0, 96.0);
            s.uppercase = true;
            s.color = "#ffd400".into();
            stroke(s, "#000000", 11.0);
            shadow(s, "#000000", 1.0, 0.0, 9.0);
        },
    },
    TextPreset {
        id: "pop-3d",
        label: "Pop 3D",
        category: C::Titles,
        sample: "NO WAY",
        anim_in: Some((In::Drop, 0.4)),
        style: |s| {
            font(s, "Rubik", 900.0, 110.0);
            s.uppercase = true;
            stroke(s, "#111111", 6.0);
            shadow(s, "#ff2d55", 1.0, 0.0, 12.0);
        },
    },
    TextPreset {
        id: "sticker",
        label: "Sticker",
        category: C::Titles,
        sample: "so good",
        anim_in: Some((In::Pop, 0.3)),
        style: |s| {
            font(s, "Poppins", 700.0, 96.0);
            s.color = "#111111".into();
            stroke(s, "#ffffff", 12.0);
            shadow(s, "#000000", 0.35, 14.0, 6.0);
        },
    },
    TextPreset {
        id: "headline",
        label: "Headline",
        category: C::Titles,
        sample: "3 THINGS I WISH I KNEW",
        anim_in: Some((In::ZoomIn, 0.35)),
        style: |s| {
            font(s, "Archivo Black", 400.0, 88.0);
            s.uppercase = true;
            shadow(s, "#000000", 0.7, 20.0, 6.0);
        },
    },
    TextPreset {
        id: "outline",
        label: "Color outline",
        category: C::Titles,
        sample: "Level up",
        anim_in: Some((In::Rise, 0.35)),
        style: |s| {
            font(s, "Poppins", 700.0, 88.0);
            s.color = "#ffffff".into();
            stroke(s, "#7c3aed", 8.0);
        },
    },
    TextPreset {
        id: "retro",
        label: "Retro",
        category: C::Titles,
        sample: "THROWBACK",
        anim_in: Some((In::Drop, 0.4)),
        style: |s| {
            font(s, "Barlow Condensed", 700.0, 110.0);
            s.uppercase = true;
            s.color = "#ffcc4d".into();
            stroke(s, "#e4572e", 6.0);
            shadow(s, "#2b2d42", 1.0, 0.0, 10.0);
        },
    },
    TextPreset {
        id: "minimal",
        label: "Minimal",
        category: C::Titles,
        sample: "CHAPTER ONE",
        anim_in: Some((In::Fade, 0.5)),
        style: |s| {
            font(s, "Inter", 500.0, 56.0);
            s.uppercase = true;
            s.letter_spacing = 10.0;
        },
    },
    TextPreset {
        id: "serif",
        label: "Elegant",
        category: C::Titles,
        sample: "a little story",
        anim_in: Some((In::Blur, 0.5)),
        style: |s| {
            font(s, "Georgia", 400.0, 80.0);
            s.italic = true;
        },
    },
    // Boxes: labels and tags
    TextPreset {
        id: "label",
        label: "Red label",
        category: C::Boxes,
        sample: "BREAKING",
        anim_in: Some((In::Rise, 0.4)),
        style: |s| {
            font(s, "Bebas Neue", 400.0, 96.0);
            plate(s, "#ef3340", 1.0, 14.0, 6.0);
            s.uppercase = true;
        },
    },
    TextPreset {
        id: "storytime",
        label: "White box",
        category: C::Boxes,
        sample: "storytime",
        anim_in: Some((In::Pop, 0.3)),
        style: |s| {
            font(s, "DM Sans", 700.0, 72.0);
            s.color = "#111111".into();
            plate(s, "#ffffff", 1.0, 18.0, 16.0);
        },
    },
    TextPreset {
        id: "tag-yellow",
        label: "Yellow tag",
        category: C::Boxes,
        sample: "MUST WATCH",
        anim_in: Some((In::Pop, 0.3)),
        style: |s| {
            font(s, "Archivo Black", 400.0, 68.0);
            s.color = "#111111".into();
            plate(s, "#ffe14d", 1.0, 16.0, 10.0);
            s.uppercase = true;
        },
    },
    TextPreset {
        id: "tag-blue",
        label: "Blue pill",
        category: C::Boxes,
        sample: "Tip #1",
        anim_in: Some((In::SlideLeft, 0.35)),
        style: |s| {
            font(s, "DM Sans", 700.0, 66.0);
            s.color = "#ffffff".into();
            plate(s, "#2f6bff", 1.0, 18.0, 60.0);
        },
    },
    TextPreset {
        id: "tag-pink",
        label: "Pastel",
        category: C::Boxes,
        sample: "get ready with me",
        anim_in: Some((In::Pop, 0.3)),
        style: |s| {
            font(s, "Poppins", 700.0, 60.0);
            s.color = "#3a1027".into();
            plate(s, "#ffd1e3", 1.0, 18.0, 18.0);
        },
    },
    // Glow: neon and night looks
    TextPreset {
        id: "glow",
        label: "Ice glow",
        category: C::Glow,
        sample: "late night",
        anim_in: Some((In::Blur, 0.4)),
        style: |s| {
            font(s, "Rubik", 800.0, 88.0);
            s.color = "#e9fbff".into();
            shadow(s, "#4cc9f0", 1.0, 28.0, 0.0);
        },
    },
    TextPreset {
        id: "neon-pink",
        label: "Neon pink",
        category: C::Glow,
        sample: "vibes only",
        anim_in: Some((In::ZoomIn, 0.35)),
        style: |s| {
            font(s, "Poppins", 700.0, 88.0);
            s.color = "#fff0fa".into();
            shadow(s, "#ff2bd6", 1.0, 30.0, 0.0);
        },
    },
    TextPreset {
        id: "gold",
        label: "Gold",
        category: C::Glow,
        sample: "WINNER",
        anim_in: Some((In::ZoomIn, 0.4)),
        style: |s| {
            font(s, "Anton", 400.0, 110.0);
            s.uppercase = true;
            s.color = "#ffd977".into();
            stroke(s, "#5a3a00", 5.0);
            shadow(s, "#ffb800", 0.8, 26.0, 0.0);
        },
    },
];

pub fn text_preset(id: &str) -> Option<&'static TextPreset> {
    TEXT_PRESETS.iter().find(|preset| preset.id == id)
}

/// A preset replaces the whole look; the words and the layout box carry over.
pub fn preset_text_style(current: &TextStyle, preset: &TextPreset) -> TextStyle {
    let mut style = TextStyle { content: current.content.clone(), align: current.align, max_width: current.max_width, ..TextStyle::default() };
    (preset.style)(&mut style);
    // Word-highlight captions keep the word they light up.
    if current.highlight_index >= 0 {
        style.highlight_index = current.highlight_index;
    }
    style
}

/// Default look of generated captions.
pub fn caption_style() -> TextStyle {
    let mut s = TextStyle::default();
    font(&mut s, "Manrope", 800.0, 78.0);
    stroke(&mut s, "#000000", 6.0);
    s.max_width = 80.0;
    s
}

pub fn text_item(content: &str, preset: Option<&TextPreset>) -> TimelineItem {
    let mut text = preset.map(TextPreset::text_style).unwrap_or_default();
    text.content = content.into();
    TimelineItem {
        kind: ItemKind::Text,
        name: item_name(content, "Text"),
        duration: 3.0,
        transform: Some(Transform::default()),
        text: Some(text),
        anim_in: preset.and_then(TextPreset::animation),
        ..Default::default()
    }
}

pub fn sticker_item(emoji: &str) -> TimelineItem {
    let text = TextStyle { content: emoji.into(), size: 220.0, weight: 400.0, font: "Segoe UI Emoji".into(), ..Default::default() };
    TimelineItem {
        kind: ItemKind::Text,
        name: format!("Sticker {emoji}"),
        duration: 3.0,
        transform: Some(Transform { y: 40.0, ..Default::default() }),
        text: Some(text),
        ..Default::default()
    }
}

pub struct ShapePreset {
    pub kind: ShapeKind,
    pub label: &'static str,
    pub width: f64,
    pub height: f64,
    pub color: &'static str,
    pub radius: f64,
}

pub const SHAPE_PRESETS: &[ShapePreset] = &[
    ShapePreset { kind: ShapeKind::Rect, label: "Rectangle", width: 60.0, height: 12.0, color: "#ffffff", radius: 24.0 },
    ShapePreset { kind: ShapeKind::Circle, label: "Circle", width: 30.0, height: 30.0, color: "#4cc9f0", radius: 0.0 },
    ShapePreset { kind: ShapeKind::Line, label: "Line", width: 60.0, height: 0.4, color: "#ffffff", radius: 999.0 },
    ShapePreset { kind: ShapeKind::Blur, label: "Blur region", width: 60.0, height: 14.0, color: "#ffffff", radius: 28.0 },
];

pub fn shape_item(kind: ShapeKind) -> TimelineItem {
    let preset = SHAPE_PRESETS.iter().find(|p| p.kind == kind).unwrap_or(&SHAPE_PRESETS[0]);
    TimelineItem {
        kind: ItemKind::Shape,
        name: preset.label.into(),
        duration: 3.0,
        radius: Some(preset.radius),
        transform: Some(Transform::default()),
        shape: Some(ShapeStyle { kind, width: preset.width, height: preset.height, color: preset.color.into(), blur: 30.0 }),
        ..Default::default()
    }
}

/// Clip name from text: the first line, at most 40 characters.
pub fn item_name(content: &str, fallback: &str) -> String {
    let line: String = content.lines().next().unwrap_or("").chars().take(40).collect();
    if line.trim().is_empty() {
        fallback.into()
    } else {
        line
    }
}

pub const STICKER_GROUPS: &[(&str, &[&str])] = &[
    ("Reactions", &["😂", "🤣", "😍", "🥹", "😎", "🤯", "😱", "🥳", "😭", "😡", "🤔", "🙄", "😴", "🤩", "😇", "🫠"]),
    ("Gestures", &["👍", "👎", "👏", "🙌", "🙏", "👉", "👈", "👆", "👇", "✌️", "🤞", "💪", "👀", "🫶", "🤝", "✋"]),
    ("Symbols", &["🔥", "💯", "✨", "⭐", "💥", "💫", "❤️", "💔", "✅", "❌", "⚠️", "❗", "❓", "💡", "🎯", "🚀"]),
    ("Things", &["🎉", "🎁", "💰", "💎", "🏆", "📈", "📉", "⏰", "📱", "💻", "🎮", "🎧", "📸", "🎬", "🍕", "☕"]),
    ("Arrows", &["⬆️", "⬇️", "⬅️", "➡️", "↗️", "↘️", "🔄", "🔁", "⤴️", "⤵️", "🔝", "🆕", "🆗", "🔴", "🟢", "🔵"]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_have_unique_ids_and_apply() {
        let ids: std::collections::HashSet<_> = TEXT_PRESETS.iter().map(|p| p.id).collect();
        assert_eq!(ids.len(), TEXT_PRESETS.len());
        let current = TextStyle { content: "hello world".into(), highlight_index: 1, max_width: 70.0, ..Default::default() };
        let styled = preset_text_style(&current, text_preset("gold").unwrap());
        assert_eq!(styled.font, "Anton");
        assert_eq!(styled.content, "hello world");
        assert_eq!(styled.highlight_index, 1);
        assert_eq!(styled.max_width, 70.0);
    }

    #[test]
    fn names_use_the_first_line() {
        assert_eq!(item_name("Hello\nworld", "Text"), "Hello");
        assert_eq!(item_name("  ", "Text"), "Text");
    }
}
