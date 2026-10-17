//! The project document (version 2), stored as JSON. Field names and values are
//! the ones the Tauri app wrote, so every existing project opens unchanged.
//! Unknown enum values fall back to a safe default instead of failing the
//! whole document.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

macro_rules! string_enum {
    ($name:ident { $($variant:ident = $text:literal),+ $(,)? } default $default:ident = $default_text:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
        pub enum $name {
            #[default]
            $default,
            $($variant),+
        }

        impl $name {
            pub const ALL: &'static [$name] = &[$name::$default, $($name::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self {
                    $name::$default => $default_text,
                    $($name::$variant => $text,)+
                }
            }

            pub fn parse(value: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|candidate| candidate.as_str() == value)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = Option::<String>::deserialize(deserializer)?;
                Ok(value.as_deref().and_then(Self::parse).unwrap_or_default())
            }
        }
    };
}

string_enum!(TrackKind { Text = "text", Audio = "audio" } default Video = "video");
string_enum!(ItemKind { Image = "image", Audio = "audio", Text = "text", Shape = "shape" } default Video = "video");
string_enum!(MediaKind { Image = "image", Audio = "audio" } default Video = "video");
string_enum!(Fit { Cover = "cover", Fill = "fill" } default Contain = "contain");
string_enum!(Easing {
    EaseIn = "ease-in", EaseOut = "ease-out", EaseInOut = "ease-in-out",
    BackIn = "back-in", BackOut = "back-out", BackInOut = "back-in-out", Custom = "custom"
} default Linear = "linear");
string_enum!(BlendMode {
    Multiply = "multiply", Screen = "screen", Overlay = "overlay", Darken = "darken", Lighten = "lighten",
    ColorDodge = "color-dodge", ColorBurn = "color-burn", HardLight = "hard-light", SoftLight = "soft-light",
    Difference = "difference", Exclusion = "exclusion"
} default Normal = "normal");
string_enum!(AnimationInKind {
    Fade = "fade", Pop = "pop", Rise = "rise", Drop = "drop", SlideLeft = "slide-left", SlideRight = "slide-right",
    ZoomIn = "zoom-in", ZoomOut = "zoom-out", Bounce = "bounce", Spin = "spin", Blur = "blur", Typewriter = "typewriter"
} default None = "none");
string_enum!(AnimationOutKind {
    Fade = "fade", Shrink = "shrink", Grow = "grow", Sink = "sink", Lift = "lift",
    SlideLeft = "slide-left", SlideRight = "slide-right", Spin = "spin", Blur = "blur"
} default None = "none");
string_enum!(EffectKind {
    Punch = "punch", Shake = "shake", Blur = "blur", Vhs = "vhs", Glitch = "glitch",
    ZoomIn = "zoom-in", ZoomOut = "zoom-out", Flash = "flash",
    Pulse = "pulse", Sway = "sway", Strobe = "strobe", RgbSplit = "rgb-split", Pixelate = "pixelate",
    ZoomBlur = "zoom-blur", Fisheye = "fisheye", Mirror = "mirror", Mono = "mono", Vintage = "vintage",
    Film = "film", Negative = "negative", Rainbow = "rainbow", Posterize = "posterize", Sharpen = "sharpen",
    Vignette = "vignette", LightLeak = "light-leak", Letterbox = "letterbox"
} default None = "none");
string_enum!(TransitionKind {
    Fade = "fade", Black = "black", White = "white", SlideLeft = "slide-left", SlideRight = "slide-right",
    SlideUp = "slide-up", SlideDown = "slide-down", Zoom = "zoom", Blur = "blur",
    WipeLeft = "wipe-left", WipeRight = "wipe-right", Flash = "flash", Spin = "spin",
    ZoomOut = "zoom-out", ZoomBlur = "zoom-blur", WhipLeft = "whip-left", WhipRight = "whip-right",
    CoverLeft = "cover-left", CoverRight = "cover-right", WipeUp = "wipe-up", WipeDown = "wipe-down",
    Iris = "iris", Clock = "clock", Pixelate = "pixelate", Glitch = "glitch", RgbSplit = "rgb-split",
    LightLeak = "light-leak"
} default None = "none");
string_enum!(ShapeKind { Circle = "circle", Line = "line", Blur = "blur" } default Rect = "rect");
string_enum!(HighlightMode { Color = "color", Box = "box" } default None = "none");
string_enum!(TextAlign { Left = "left", Right = "right" } default Center = "center");
string_enum!(ExportFormat { Mov = "mov", Webm = "webm" } default Mp4 = "mp4");
string_enum!(ExportCodec { H265 = "h265", Vp9 = "vp9" } default H264 = "h264");
string_enum!(ExportQuality { Draft = "draft", Standard = "standard", Ultra = "ultra" } default High = "high");

impl ItemKind {
    pub fn track_kind(self) -> TrackKind {
        match self {
            ItemKind::Audio => TrackKind::Audio,
            ItemKind::Text => TrackKind::Text,
            _ => TrackKind::Video,
        }
    }

    pub fn is_visual(self) -> bool {
        self != ItemKind::Audio
    }

    pub fn is_media(self) -> bool {
        matches!(self, ItemKind::Video | ItemKind::Image | ItemKind::Audio)
    }

    pub fn has_sound(self) -> bool {
        matches!(self, ItemKind::Video | ItemKind::Audio)
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Transform {
    /// Centre of the layer in % of the canvas width.
    pub x: f64,
    /// Centre of the layer in % of the canvas height.
    pub y: f64,
    pub scale: f64,
    /// Degrees, clockwise.
    pub rotation: f64,
    pub opacity: f64,
}

impl Default for Transform {
    fn default() -> Self {
        Transform { x: 50.0, y: 50.0, scale: 1.0, rotation: 0.0, opacity: 1.0 }
    }
}

/// CSS-style cubic-bezier(x1, y1, x2, y2); y may leave 0..1 to overshoot.
pub type Bezier = [f64; 4];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Keyframe {
    pub id: String,
    /// Seconds from the clip start.
    pub time: f64,
    pub transform: Transform,
    /// Easing of the segment that ends at this keyframe.
    pub easing: Easing,
    /// Curve used when easing is `Custom`.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient_bezier")]
    pub bezier: Option<Bezier>,
}

/// A malformed curve only loses the curve, never the document.
fn lenient_bezier<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Bezier>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| {
        let list = value.as_array()?;
        let numbers: Vec<f64> = list.iter().filter_map(serde_json::Value::as_f64).collect();
        let curve: Bezier = numbers.try_into().ok()?;
        curve.iter().all(|v| v.is_finite()).then_some(curve)
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Animation<K> {
    pub kind: K,
    pub duration: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
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

impl Adjust {
    pub fn is_neutral(&self) -> bool {
        *self == Adjust::default()
    }
}

/// Normalised source rectangle (0..1).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TextStyle {
    pub content: String,
    pub font: String,
    pub size: f64,
    pub weight: f64,
    pub italic: bool,
    pub color: String,
    pub align: TextAlign,
    pub letter_spacing: f64,
    pub line_height: f64,
    /// Maximum line width in % of the canvas width.
    pub max_width: f64,
    pub uppercase: bool,
    pub stroke_color: String,
    pub stroke_width: f64,
    pub background_color: String,
    pub background_opacity: f64,
    pub background_padding: f64,
    pub background_radius: f64,
    pub shadow_color: String,
    pub shadow_opacity: f64,
    pub shadow_blur: f64,
    pub shadow_distance: f64,
    /// How the highlighted word stands out: its own color, or a box behind it.
    pub highlight_mode: HighlightMode,
    pub highlight_color: String,
    /// Text color of the highlighted word in box mode ("" keeps the normal color).
    pub highlight_text_color: String,
    /// Index of the highlighted word (whitespace separated); -1 for none.
    pub highlight_index: i64,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            content: "Your text".into(),
            font: "Inter".into(),
            size: 84.0,
            weight: 800.0,
            italic: false,
            color: "#ffffff".into(),
            align: TextAlign::Center,
            letter_spacing: 0.0,
            line_height: 1.15,
            max_width: 86.0,
            uppercase: false,
            stroke_color: "#000000".into(),
            stroke_width: 0.0,
            background_color: "#000000".into(),
            background_opacity: 0.0,
            background_padding: 18.0,
            background_radius: 14.0,
            shadow_color: "#000000".into(),
            shadow_opacity: 0.0,
            shadow_blur: 12.0,
            shadow_distance: 4.0,
            highlight_mode: HighlightMode::None,
            highlight_color: "#ffe14d".into(),
            highlight_text_color: String::new(),
            highlight_index: -1,
        }
    }
}

impl TextStyle {
    /// A word is highlighted when a mode is chosen and an index is set.
    pub fn has_highlight(&self) -> bool {
        self.highlight_mode != HighlightMode::None && self.highlight_index >= 0
    }

    /// Text as drawn (uppercase applied).
    pub fn display_text(&self) -> String {
        if self.uppercase {
            self.content.to_uppercase()
        } else {
            self.content.clone()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShapeStyle {
    pub kind: ShapeKind,
    /// % of the canvas width.
    pub width: f64,
    /// % of the canvas height.
    pub height: f64,
    pub color: String,
    /// Blur strength for blur regions, in project pixels.
    pub blur: f64,
}

impl Default for ShapeStyle {
    fn default() -> Self {
        ShapeStyle { kind: ShapeKind::Rect, width: 30.0, height: 20.0, color: "#ffffff".into(), blur: 30.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct EffectRange {
    pub start: f64,
    pub duration: f64,
}

/// Strength of an effect nobody has adjusted: the look each effect was designed with.
pub const DEFAULT_EFFECT_AMOUNT: f64 = 0.5;

/// One effect of a clip's stack. Effects apply in order, each over its own
/// part of the clip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClipEffect {
    pub id: String,
    pub kind: EffectKind,
    /// Strength 0..1 (see `DEFAULT_EFFECT_AMOUNT`).
    pub amount: f64,
    /// Part of the clip the effect covers, relative to the clip start. Whole clip when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<EffectRange>,
    #[serde(flatten, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for ClipEffect {
    fn default() -> Self {
        ClipEffect { id: String::new(), kind: EffectKind::None, amount: DEFAULT_EFFECT_AMOUNT, range: None, extra: Default::default() }
    }
}

impl ClipEffect {
    pub fn amount(&self) -> f64 {
        if self.amount.is_finite() {
            self.amount.clamp(0.0, 1.0)
        } else {
            DEFAULT_EFFECT_AMOUNT
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TimelineItem {
    pub id: String,
    pub track_id: String,
    pub kind: ItemKind,
    pub name: String,
    pub start: f64,
    pub duration: f64,
    #[serde(skip_serializing_if = "is_false")]
    pub locked: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub hidden: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,

    /// Media items reference `Project::assets` by id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    /// Seconds into the source where the clip begins.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_in: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
    #[serde(skip_serializing_if = "is_false")]
    pub muted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fade_in: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fade_out: Option<f64>,
    /// Voice cleanup strength, 0..1 (absent = off).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_enhance: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub transform: Option<Transform>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub keyframes: Vec<Keyframe>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit: Option<Fit>,
    /// Object position (0..100) used when the fit crops the source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<Point>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crop: Option<Crop>,
    #[serde(skip_serializing_if = "is_false")]
    pub flip_x: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub flip_y: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blend: Option<BlendMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjust: Option<Adjust>,
    /// The clip's effects, applied in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<ClipEffect>,
    /// The single effect of documents written before effect stacks; read only,
    /// moved into `effects` by `adopt_legacy_effect` when a project loads.
    #[serde(rename = "effect", skip_serializing)]
    pub legacy_effect: Option<EffectKind>,
    #[serde(rename = "effectRange", skip_serializing)]
    pub legacy_effect_range: Option<EffectRange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anim_in: Option<Animation<AnimationInKind>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anim_out: Option<Animation<AnimationOutKind>>,
    /// Transition into this clip from the clip that ends exactly where it starts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition: Option<Animation<TransitionKind>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<TextStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<ShapeStyle>,
    /// Fields this build does not know (written by a newer one): kept as they are.
    #[serde(flatten, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl TimelineItem {
    pub fn end(&self) -> f64 {
        self.start + self.duration
    }

    pub fn speed(&self) -> f64 {
        match self.speed {
            Some(value) if value.is_finite() && value > 0.0 => value.clamp(0.1, 100.0),
            _ => 1.0,
        }
    }

    pub fn volume(&self) -> f64 {
        self.volume.filter(|v| v.is_finite()).unwrap_or(1.0).clamp(0.0, 4.0)
    }

    pub fn source_in(&self) -> f64 {
        self.source_in.filter(|v| v.is_finite()).unwrap_or(0.0).max(0.0)
    }

    pub fn fade_in(&self) -> f64 {
        self.fade_in.unwrap_or(0.0).max(0.0)
    }

    pub fn fade_out(&self) -> f64 {
        self.fade_out.unwrap_or(0.0).max(0.0)
    }

    pub fn radius(&self) -> f64 {
        self.radius.unwrap_or(0.0).max(0.0)
    }

    /// Effects that do something (an unknown kind reads as None).
    pub fn active_effects(&self) -> impl Iterator<Item = &ClipEffect> {
        self.effects.iter().filter(|effect| effect.kind != EffectKind::None)
    }

    pub fn has_effects(&self) -> bool {
        self.active_effects().next().is_some()
    }

    /// Moves the single effect of an older document into `effects`, at the
    /// strength it always had.
    pub fn adopt_legacy_effect(&mut self) {
        let kind = self.legacy_effect.take().unwrap_or_default();
        let range = self.legacy_effect_range.take();
        if kind != EffectKind::None && self.effects.is_empty() {
            self.effects.push(ClipEffect { id: format!("{}-fx", self.id), kind, range, ..Default::default() });
        }
    }

    pub fn text_style(&self) -> TextStyle {
        self.text.clone().unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Track {
    pub id: String,
    pub name: String,
    pub kind: TrackKind,
    #[serde(skip_serializing_if = "is_false")]
    pub locked: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub muted: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub hidden: bool,
    pub items: Vec<TimelineItem>,
    /// Fields this build does not know (written by a newer one): kept as they are.
    #[serde(flatten, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct MediaAsset {
    pub id: String,
    pub kind: MediaKind,
    pub name: String,
    /// Absolute path of the original file.
    pub path: String,
    /// Older projects may carry a playable proxy; the native player reads originals.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_path: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub needs_proxy: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_audio: Option<bool>,
    pub size: u64,
    pub added_at: String,
    /// Fields this build does not know (written by a newer one): kept as they are.
    #[serde(flatten, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportSettings {
    pub format: ExportFormat,
    pub codec: ExportCodec,
    pub quality: ExportQuality,
    /// Short edge of the rendered file in pixels (720, 1080, 1440, 2160).
    pub resolution: u32,
    pub fps: u32,
    pub hardware: bool,
    /// Target loudness of the soundtrack in LUFS; absent leaves the mix as it is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loudness: Option<f64>,
}

impl Default for ExportSettings {
    fn default() -> Self {
        ExportSettings {
            format: ExportFormat::Mp4,
            codec: ExportCodec::H264,
            quality: ExportQuality::High,
            resolution: 1080,
            fps: 30,
            hardware: false,
            loudness: None,
        }
    }
}

pub const FPS_CHOICES: [u32; 5] = [24, 25, 30, 50, 60];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Project {
    pub version: u32,
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub background: String,
    pub tracks: Vec<Track>,
    pub assets: Vec<MediaAsset>,
    pub export: ExportSettings,
    /// Free-form origin info, e.g. which generator produced the draft.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// Fields this build does not know (written by a newer one): kept as they are.
    #[serde(flatten, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for Project {
    fn default() -> Self {
        Project {
            version: 2,
            id: String::new(),
            name: "Untitled project".into(),
            created_at: String::new(),
            updated_at: String::new(),
            width: 1080,
            height: 1920,
            fps: 30,
            background: "#000000".into(),
            tracks: vec![],
            assets: vec![],
            export: ExportSettings::default(),
            origin: None,
            extra: serde_json::Map::new(),
        }
    }
}

impl Project {
    pub fn asset(&self, id: Option<&str>) -> Option<&MediaAsset> {
        let id = id?;
        self.assets.iter().find(|asset| asset.id == id)
    }

    pub fn items(&self) -> impl Iterator<Item = &TimelineItem> {
        self.tracks.iter().flat_map(|track| track.items.iter())
    }

    /// (track index, item index) of an item.
    pub fn locate(&self, id: &str) -> Option<(usize, usize)> {
        self.tracks.iter().enumerate().find_map(|(t, track)| track.items.iter().position(|item| item.id == id).map(|i| (t, i)))
    }

    pub fn item(&self, id: &str) -> Option<&TimelineItem> {
        self.locate(id).map(|(t, i)| &self.tracks[t].items[i])
    }

    pub fn item_mut(&mut self, id: &str) -> Option<&mut TimelineItem> {
        self.locate(id).map(|(t, i)| &mut self.tracks[t].items[i])
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct CaptionCue {
    pub text: String,
    pub start: f64,
    pub duration: f64,
    /// Word lit up while this cue shows (word-by-word highlight captions).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight_index: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_enum_values_fall_back() {
        let item: TimelineItem = serde_json::from_str(r#"{"id":"a","kind":"hologram","effects":[{"id":"e","kind":"sparkles"}],"fit":"cover"}"#).unwrap();
        assert_eq!(item.kind, ItemKind::Video);
        assert_eq!(item.effects[0].kind, EffectKind::None);
        assert!(!item.has_effects());
        assert_eq!(item.fit, Some(Fit::Cover));
    }

    #[test]
    fn enums_round_trip_their_text() {
        for easing in Easing::ALL {
            assert_eq!(Easing::parse(easing.as_str()), Some(*easing));
        }
        assert_eq!(Easing::EaseInOut.as_str(), "ease-in-out");
        assert_eq!(TransitionKind::None.as_str(), "none");
    }

    #[test]
    fn text_style_fills_missing_fields_with_defaults() {
        let style: TextStyle = serde_json::from_str(r#"{"content":"Hi","highlightIndex":2}"#).unwrap();
        assert_eq!(style.font, "Inter");
        assert_eq!(style.highlight_index, 2);
        assert_eq!(style.line_height, 1.15);
    }

    #[test]
    fn malformed_bezier_is_dropped() {
        let keyframe: Keyframe = serde_json::from_str(r#"{"id":"k","easing":"custom","bezier":[1,2]}"#).unwrap();
        assert_eq!(keyframe.bezier, None);
        let keyframe: Keyframe = serde_json::from_str(r#"{"id":"k","easing":"custom","bezier":[0.2,1.4,0.6,-0.3]}"#).unwrap();
        assert_eq!(keyframe.bezier, Some([0.2, 1.4, 0.6, -0.3]));
    }

    #[test]
    fn a_legacy_effect_becomes_the_first_of_the_stack() {
        let mut item: TimelineItem = serde_json::from_str(r#"{"id":"a","kind":"video","effect":"vhs","effectRange":{"start":0.5,"duration":1}}"#).unwrap();
        item.adopt_legacy_effect();
        assert_eq!(item.effects.len(), 1);
        assert_eq!(item.effects[0].kind, EffectKind::Vhs);
        assert_eq!(item.effects[0].amount, DEFAULT_EFFECT_AMOUNT);
        assert_eq!(item.effects[0].range, Some(EffectRange { start: 0.5, duration: 1.0 }));
        let json = serde_json::to_value(&item).unwrap();
        assert!(json.get("effect").is_none() && json.get("effectRange").is_none());
        assert_eq!(json["effects"][0]["kind"], "vhs");
        // Read twice, it is still one effect.
        item.adopt_legacy_effect();
        assert_eq!(item.effects.len(), 1);
    }

    #[test]
    fn optional_fields_are_not_written() {
        let item = TimelineItem { id: "a".into(), kind: ItemKind::Text, ..Default::default() };
        let json = serde_json::to_value(&item).unwrap();
        let object = json.as_object().unwrap();
        assert!(!object.contains_key("locked"));
        assert!(!object.contains_key("keyframes"));
        assert!(!object.contains_key("transform"));
        assert_eq!(object["kind"], "text");
    }
}
