//! Typed view of the project document (mirror of src/lib/types.ts), used by the
//! exporter and AI features. Unknown fields are ignored and missing fields fall
//! back to the same defaults as the frontend.

use serde::Deserialize;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
#[serde(default)]
pub struct Transform {
    pub x: f64,
    pub y: f64,
    pub scale: f64,
    pub rotation: f64,
    pub opacity: f64,
}

impl Default for Transform {
    fn default() -> Self {
        Transform { x: 50.0, y: 50.0, scale: 1.0, rotation: 0.0, opacity: 1.0 }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Keyframe {
    pub id: String,
    pub time: f64,
    pub transform: Transform,
    pub easing: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(default)]
pub struct EffectRange {
    pub start: f64,
    pub duration: f64,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Animation {
    pub kind: String,
    pub duration: f64,
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(default)]
pub struct Adjust {
    pub exposure: f64,
    pub brightness: f64,
    pub contrast: f64,
    pub saturation: f64,
    pub temperature: f64,
    pub tint: f64,
    pub hue: f64,
    pub vignette: f64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(default)]
pub struct Crop {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Default for Crop {
    fn default() -> Self {
        Crop { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Default for Point {
    fn default() -> Self {
        Point { x: 50.0, y: 50.0 }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ShapeStyle {
    pub kind: String,
    pub width: f64,
    pub height: f64,
    pub color: String,
    pub blur: f64,
}

impl Default for ShapeStyle {
    fn default() -> Self {
        ShapeStyle { kind: "rect".into(), width: 30.0, height: 20.0, color: "#ffffff".into(), blur: 30.0 }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TextStyle {
    pub content: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TimelineItem {
    pub id: String,
    pub track_id: String,
    pub kind: String,
    pub name: String,
    pub start: f64,
    pub duration: f64,
    pub hidden: bool,
    pub asset_id: Option<String>,
    pub source_in: f64,
    pub speed: Option<f64>,
    pub volume: Option<f64>,
    pub muted: bool,
    pub fade_in: f64,
    pub fade_out: f64,
    pub transform: Option<Transform>,
    pub keyframes: Vec<Keyframe>,
    pub fit: Option<String>,
    pub focus: Option<Point>,
    pub crop: Option<Crop>,
    pub flip_x: bool,
    pub flip_y: bool,
    pub radius: f64,
    pub blend: Option<String>,
    pub adjust: Option<Adjust>,
    pub effect: Option<String>,
    pub effect_range: Option<EffectRange>,
    pub anim_in: Option<Animation>,
    pub anim_out: Option<Animation>,
    pub transition: Option<Animation>,
    pub text: Option<TextStyle>,
    pub shape: Option<ShapeStyle>,
}

impl TimelineItem {
    pub fn speed(&self) -> f64 {
        match self.speed {
            Some(value) if value.is_finite() && value > 0.0 => value.clamp(0.1, 100.0),
            _ => 1.0,
        }
    }

    pub fn volume(&self) -> f64 {
        self.volume.unwrap_or(1.0).clamp(0.0, 4.0)
    }

    pub fn end(&self) -> f64 {
        self.start + self.duration
    }

    pub fn is_visual(&self) -> bool {
        self.kind != "audio"
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Track {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub muted: bool,
    pub hidden: bool,
    pub items: Vec<TimelineItem>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct MediaAsset {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub path: String,
    pub duration: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub has_audio: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExportSettings {
    pub format: String,
    pub codec: String,
    pub quality: String,
    pub resolution: u32,
    pub fps: u32,
    pub hardware: bool,
}

impl Default for ExportSettings {
    fn default() -> Self {
        ExportSettings { format: "mp4".into(), codec: "h264".into(), quality: "high".into(), resolution: 1080, fps: 30, hardware: false }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub background: String,
    pub tracks: Vec<Track>,
    pub assets: Vec<MediaAsset>,
    pub export: ExportSettings,
}

impl Default for Project {
    fn default() -> Self {
        Project {
            id: String::new(),
            name: String::new(),
            width: 1080,
            height: 1920,
            fps: 30,
            background: "#000000".into(),
            tracks: vec![],
            assets: vec![],
            export: ExportSettings::default(),
        }
    }
}

impl Project {
    pub fn asset(&self, id: Option<&str>) -> Option<&MediaAsset> {
        let id = id?;
        self.assets.iter().find(|asset| asset.id == id)
    }

    /// Mirror of `contentEnd` in scene.ts.
    pub fn content_end(&self) -> f64 {
        let mut end: f64 = 0.0;
        for track in &self.tracks {
            if track.hidden && track.kind != "audio" {
                continue;
            }
            for item in &track.items {
                if item.hidden {
                    continue;
                }
                if item.kind == "audio" && (track.muted || item.muted || item.volume() <= 0.0) {
                    continue;
                }
                end = end.max(item.end());
            }
        }
        (end * 1000.0).round() / 1000.0
    }
}
