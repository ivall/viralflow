//! Brings any stored document to the current schema and repairs invalid values.

use serde_json::{Map, Value};

use crate::engine::color::is_hex_color;
use crate::engine::keyframes::normalize_transform;
use crate::engine::math::uid;
use crate::model::*;
use crate::presets::now_iso;
use crate::{Error, Result};

/// Newest document schema this build understands.
pub const SCHEMA_VERSION: u64 = 2;

/// Reads a stored document: refuses one made by a newer ViralFlow (opening
/// it here would drop what this build does not know), migrates the rest.
pub fn read_project(input: &Value) -> Result<Project> {
    if input.get("version").and_then(Value::as_u64).is_some_and(|v| v > SCHEMA_VERSION) {
        return Err(Error::msg("This project was made by a newer version of ViralFlow. Update the app to open it."));
    }
    Ok(migrate_project(input))
}

/// The largest part of `value` that `parses` accepts: a field with a value
/// of the wrong type is dropped (its default applies) instead of the whole
/// document; inside objects and arrays only the broken parts go. All model
/// types use `#[serde(default)]`, so every field can be checked on its own.
fn salvage(value: &Value, parses: &dyn Fn(&Value) -> bool) -> Option<Value> {
    if parses(value) {
        return Some(value.clone());
    }
    let kept = match value {
        Value::Object(map) => {
            let mut kept = Map::new();
            for (key, field) in map {
                let alone = |candidate: &Value| parses(&Value::Object(Map::from_iter([(key.clone(), candidate.clone())])));
                match salvage(field, &alone) {
                    Some(fixed) => {
                        kept.insert(key.clone(), fixed);
                    }
                    None => log::warn!("project: dropped unreadable field \"{key}\""),
                }
            }
            Value::Object(kept)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .filter_map(|item| {
                    let alone = |candidate: &Value| parses(&Value::Array(vec![candidate.clone()]));
                    salvage(item, &alone)
                })
                .collect(),
        ),
        _ => return None,
    };
    parses(&kept).then_some(kept)
}

fn finite(value: Option<&Value>, fallback: f64) -> f64 {
    value.and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(fallback)
}

fn text(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

fn sanitize_export(mut export: ExportSettings, raw: Option<&Value>, fps: u32, width: u32, height: u32) -> ExportSettings {
    // Missing values follow the project, like a freshly created one.
    let has = |key: &str| raw.and_then(|value| value.get(key)).is_some();
    if !has("fps") {
        export.fps = fps;
    }
    if !has("resolution") {
        export.resolution = width.min(height);
    }
    if export.format == ExportFormat::Webm {
        export.codec = ExportCodec::Vp9;
    }
    if export.codec == ExportCodec::Vp9 && export.format != ExportFormat::Webm {
        export.codec = ExportCodec::H264;
    }
    if !FPS_CHOICES.contains(&export.fps) {
        export.fps = fps;
    }
    export.resolution = export.resolution.clamp(240, 4320);
    if export.loudness.is_some_and(|l| !l.is_finite() || l >= 0.0) {
        export.loudness = None;
    }
    export
}

/// Parses a stored document of any version into a valid project.
pub fn migrate_project(input: &Value) -> Project {
    let source = input.as_object().cloned().unwrap_or_default();
    if source.get("version").and_then(Value::as_u64) != Some(SCHEMA_VERSION) {
        return convert_legacy_project(&source);
    }
    let mut cleaned = source.clone();
    // Drop items and assets that lack the fields everything else relies on.
    if let Some(Value::Array(tracks)) = cleaned.get_mut("tracks") {
        tracks.retain(Value::is_object);
        for track in tracks.iter_mut() {
            if let Some(Value::Array(items)) = track.get_mut("items") {
                items.retain(|item| text(item.get("id")).is_some() && item.get("kind").and_then(Value::as_str).and_then(ItemKind::parse).is_some());
            }
        }
    }
    if let Some(Value::Array(assets)) = cleaned.get_mut("assets") {
        assets.retain(|asset| text(asset.get("id")).is_some() && text(asset.get("path")).is_some());
    }
    let parses = |candidate: &Value| serde_json::from_value::<Project>(candidate.clone()).is_ok();
    let readable = salvage(&Value::Object(cleaned), &parses).unwrap_or_else(|| Value::Object(Map::new()));
    let mut project: Project = serde_json::from_value(readable).unwrap_or_default();
    project.version = 2;
    let width = finite(source.get("width"), 1080.0).round().max(1.0) as u32;
    let height = finite(source.get("height"), 1920.0).round().max(1.0) as u32;
    project.width = width;
    project.height = height;
    if !FPS_CHOICES.contains(&project.fps) {
        project.fps = 30;
    }
    if project.id.is_empty() {
        project.id = uid("project");
    }
    if project.name.trim().is_empty() {
        project.name = "Untitled project".into();
    }
    if project.created_at.is_empty() {
        project.created_at = now_iso();
    }
    if project.updated_at.is_empty() {
        project.updated_at = now_iso();
    }
    if !is_hex_color(&project.background) {
        project.background = "#000000".into();
    }
    for track in &mut project.tracks {
        if track.id.is_empty() {
            track.id = uid("track");
        }
        if track.name.is_empty() {
            track.name = "Track".into();
        }
        for item in &mut track.items {
            item.adopt_legacy_effect();
            item.track_id = track.id.clone();
            item.start = if item.start.is_finite() { item.start.max(0.0) } else { 0.0 };
            item.duration = if item.duration.is_finite() { item.duration.max(0.05) } else { 3.0 };
        }
    }
    project.export = sanitize_export(project.export.clone(), source.get("export"), project.fps, width, height);
    project
}

// ---------------------------------------------------------------- legacy v1

fn legacy_animation(name: &str) -> Option<AnimationInKind> {
    Some(match name {
        "pop" | "karaoke" => AnimationInKind::Pop,
        "rise" => AnimationInKind::Rise,
        "slide" => AnimationInKind::SlideLeft,
        "bounce" => AnimationInKind::Bounce,
        "fade" => AnimationInKind::Fade,
        _ => return None,
    })
}

fn legacy_transition(name: &str) -> Option<TransitionKind> {
    Some(match name {
        "fade" => TransitionKind::Fade,
        "slide-left" | "slide" => TransitionKind::SlideLeft,
        "slide-right" => TransitionKind::SlideRight,
        "zoom" => TransitionKind::Zoom,
        "blur" => TransitionKind::Blur,
        "wipe-left" => TransitionKind::WipeLeft,
        "wipe-right" => TransitionKind::WipeRight,
        "flash" => TransitionKind::Flash,
        "spin" => TransitionKind::Spin,
        _ => return None,
    })
}

fn get<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |current, key| current.get(*key))
}

/// Converts one ViralFlow 1.x item. Media items need their files, which old
/// documents referenced by server URL, so they cannot be carried over.
fn convert_legacy_item(item: &Value, track_id: &str) -> Option<TimelineItem> {
    let raw_kind = item.get("kind").and_then(Value::as_str).unwrap_or("");
    let kind = match raw_kind {
        "overlay" => ItemKind::Image,
        "caption" | "text" => ItemKind::Text,
        "shape" => ItemKind::Shape,
        _ => return None,
    };
    if kind == ItemKind::Image {
        return None;
    }
    let number = |path: &[&str], fallback: f64| finite(get(item, path), fallback);
    let transform = normalize_transform(Some(&Transform {
        x: number(&["position", "x"], 50.0),
        y: number(&["position", "y"], 50.0),
        scale: number(&["transform", "scale"], 1.0),
        rotation: number(&["transform", "rotation"], 0.0),
        opacity: number(&["transform", "opacity"], 1.0),
    }));
    let mut result = TimelineItem {
        id: text(item.get("id")).unwrap_or_else(|| uid("item")),
        track_id: track_id.into(),
        kind,
        name: text(item.get("name")).unwrap_or_else(|| kind.as_str().into()),
        start: number(&["start"], 0.0).max(0.0),
        duration: number(&["duration"], 3.0).max(0.05),
        locked: item.get("locked").and_then(Value::as_bool).unwrap_or(false),
        hidden: item.get("hidden").and_then(Value::as_bool).unwrap_or(false),
        group_id: text(item.get("groupId")),
        transform: Some(transform),
        ..Default::default()
    };
    if let Some(Value::Array(keyframes)) = item.get("keyframes") {
        result.keyframes = keyframes
            .iter()
            .map(|k| {
                let n = |path: &[&str], fallback: f64| finite(get(k, path), fallback);
                Keyframe {
                    id: text(k.get("id")).unwrap_or_else(|| uid("kf")),
                    time: n(&["time"], 0.0),
                    easing: k
                        .get("easing")
                        .and_then(Value::as_str)
                        .and_then(Easing::parse)
                        .filter(|e| matches!(e, Easing::Linear | Easing::EaseIn | Easing::EaseOut | Easing::EaseInOut))
                        .unwrap_or_default(),
                    bezier: None,
                    transform: normalize_transform(Some(&Transform {
                        x: n(&["position", "x"], 50.0),
                        y: n(&["position", "y"], 50.0),
                        scale: n(&["transform", "scale"], 1.0),
                        rotation: n(&["transform", "rotation"], 0.0),
                        opacity: n(&["transform", "opacity"], 1.0),
                    })),
                }
            })
            .collect();
    }
    if let Some(animation) = item.get("animation").and_then(Value::as_str).and_then(legacy_animation) {
        result.anim_in = Some(Animation { kind: animation, duration: number(&["animationDuration"], 0.3) });
    }
    if let Some(transition) = item.get("transition").and_then(Value::as_str).and_then(legacy_transition) {
        result.transition = Some(Animation { kind: transition, duration: number(&["transitionDuration"], 0.4) });
    }
    if kind == ItemKind::Text {
        let size = number(&["fontSize"], 58.0);
        let content = item.get("text").and_then(Value::as_str).unwrap_or("").to_string();
        result.name = crate::presets::item_name(&content, "Text");
        result.text = Some(TextStyle {
            content,
            font: text(item.get("fontFamily")).unwrap_or_else(|| "Manrope".into()),
            size,
            weight: number(&["fontWeight"], 800.0),
            color: text(item.get("color")).unwrap_or_else(|| "#ffffff".into()),
            align: item.get("align").and_then(Value::as_str).and_then(TextAlign::parse).unwrap_or_default(),
            stroke_color: text(item.get("textStrokeColor")).unwrap_or_else(|| "#101010".into()),
            stroke_width: number(&["textStrokeWidth"], 4.2),
            background_color: text(item.get("textBackgroundColor")).unwrap_or_else(|| "#000000".into()),
            background_opacity: number(&["textBackgroundOpacity"], 0.0),
            background_padding: number(&["textBackgroundPadding"], 12.0),
            line_height: 1.02,
            letter_spacing: -0.045 * size,
            ..Default::default()
        });
    }
    if kind == ItemKind::Shape {
        let graphic = match item.get("graphic").and_then(Value::as_str).unwrap_or("rect") {
            "square" | "rectangle" => "rect",
            other => other,
        };
        result.shape = Some(ShapeStyle {
            kind: ShapeKind::parse(graphic).unwrap_or_default(),
            width: number(&["size", "width"], 32.0),
            height: number(&["size", "height"], 24.0),
            color: text(item.get("color")).unwrap_or_else(|| "#ffffff".into()),
            blur: number(&["blurAmount"], 22.0),
        });
        result.radius = Some(number(&["borderRadius"], 0.0));
    }
    Some(result)
}

pub fn convert_legacy_project(source: &Map<String, Value>) -> Project {
    let export = source.get("export").cloned().unwrap_or(Value::Null);
    let width = finite(export.get("width"), 1080.0).round().max(1.0) as u32;
    let height = finite(export.get("height"), 1920.0).round().max(1.0) as u32;
    let fps = export.get("fps").and_then(Value::as_u64).map(|v| v as u32).filter(|v| FPS_CHOICES.contains(v)).unwrap_or(30);
    let tracks: Vec<Track> = source
        .get("tracks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|track| {
            let kind = match track.get("kind").and_then(Value::as_str) {
                Some("audio") => TrackKind::Audio,
                Some("text") | Some("caption") => TrackKind::Text,
                _ => TrackKind::Video,
            };
            let id = text(track.get("id")).unwrap_or_else(|| uid("track"));
            let items = track
                .get("items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|item| convert_legacy_item(item, &id))
                .filter(|item| item.kind.track_kind() == kind)
                .collect();
            Track {
                id,
                name: text(track.get("name")).unwrap_or_else(|| "Track".into()),
                kind,
                locked: track.get("locked").and_then(Value::as_bool).unwrap_or(false),
                muted: track.get("muted").and_then(Value::as_bool).unwrap_or(false),
                hidden: track.get("hidden").and_then(Value::as_bool).unwrap_or(false),
                items,
                extra: Map::new(),
            }
        })
        .collect();
    let format = match export.get("format").and_then(Value::as_str) {
        Some("webm") => ExportFormat::Webm,
        Some("mov") => ExportFormat::Mov,
        _ => ExportFormat::Mp4,
    };
    let document = Project {
        version: 2,
        id: text(source.get("id")).unwrap_or_else(|| uid("project")),
        name: text(source.get("name")).unwrap_or_else(|| "Imported project".into()),
        created_at: text(source.get("createdAt")).unwrap_or_else(now_iso),
        updated_at: text(source.get("updatedAt")).unwrap_or_else(now_iso),
        width,
        height,
        fps,
        background: "#11120f".into(),
        tracks,
        assets: vec![],
        export: ExportSettings { fps, resolution: width.min(height), format, ..Default::default() },
        origin: Some("ViralFlow 1.x".into()),
        extra: Map::new(),
    };
    migrate_project(&serde_json::to_value(&document).unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repairs_invalid_values() {
        let input = serde_json::json!({
            "version": 2, "id": "p", "width": 720, "height": 1280, "fps": 33, "background": "red",
            "tracks": [{ "id": "t", "kind": "video", "items": [{ "id": "a", "kind": "video", "start": -3, "duration": 0 }, { "kind": "video" }] }],
            "assets": [{ "id": "x" }],
            "export": { "format": "webm", "codec": "h264", "resolution": 99999, "fps": 31 }
        });
        let project = migrate_project(&input);
        assert_eq!(project.fps, 30);
        assert_eq!(project.background, "#000000");
        assert_eq!(project.tracks[0].items.len(), 1);
        assert_eq!(project.tracks[0].items[0].start, 0.0);
        assert_eq!(project.tracks[0].items[0].duration, 0.05);
        assert_eq!(project.tracks[0].items[0].track_id, "t");
        assert!(project.assets.is_empty());
        assert_eq!(project.export.codec, ExportCodec::Vp9);
        assert_eq!(project.export.resolution, 4320);
        assert_eq!(project.export.fps, 30);
    }

    #[test]
    fn round_trips_a_document_unchanged() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/parity-project.json");
        let input: Value = serde_json::from_slice(&std::fs::read(root).unwrap()).unwrap();
        let once = migrate_project(&input);
        let twice = migrate_project(&serde_json::to_value(&once).unwrap());
        assert_eq!(once, twice);
    }

    #[test]
    fn converts_legacy_text() {
        let input = serde_json::json!({ "name": "Old", "export": { "width": 1080, "height": 1920, "fps": 30 },
            "tracks": [{ "kind": "caption", "items": [{ "id": "c", "kind": "caption", "text": "Hello", "start": 1, "duration": 2, "animation": "karaoke" }] }] });
        let project = migrate_project(&input);
        assert_eq!(project.version, 2);
        let item = &project.tracks[0].items[0];
        assert_eq!(item.text.as_ref().unwrap().content, "Hello");
        assert_eq!(item.anim_in.unwrap().kind, AnimationInKind::Pop);
        assert_eq!(project.origin.as_deref(), Some("ViralFlow 1.x"));
    }

    #[test]
    fn a_wrong_field_costs_only_that_field() {
        let document = serde_json::json!({
            "version": 2, "id": "p", "name": "Mine", "width": 1080, "height": 1920, "fps": 30,
            "tracks": [{ "id": "t", "kind": "text", "items": [
                { "id": "a", "kind": "text", "start": 1.0, "duration": 2.0, "volume": "loud", "text": { "content": "Hi", "size": "big" } },
                { "id": "b", "kind": "text", "start": "soon", "duration": 1.0 },
                { "id": "c", "kind": "hologram", "start": 0.0, "duration": 1.0 }
            ] }],
            "export": { "fps": "fast" }
        });
        let project = read_project(&document).unwrap();
        assert_eq!(project.name, "Mine");
        let items = &project.tracks[0].items;
        assert_eq!(items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["a", "b"], "unknown kinds are dropped, the rest kept");
        assert_eq!(items[0].start, 1.0);
        assert_eq!(items[0].volume, None);
        assert_eq!(items[0].text.as_ref().unwrap().content, "Hi", "the good part of a nested object stays");
        assert_eq!(items[1].start, 0.0);
        assert_eq!(project.export.fps, 30);
    }

    #[test]
    fn unknown_fields_survive_a_round_trip() {
        let document = serde_json::json!({
            "version": 2, "id": "p", "futureSetting": { "a": 1 },
            "tracks": [{ "id": "t", "kind": "video", "laneColor": "red", "items": [{ "id": "a", "kind": "video", "start": 0, "duration": 1, "aiTag": "x" }] }],
            "assets": [{ "id": "m", "path": "C:/a.mp4", "kind": "video", "hash": "abc" }]
        });
        let project = read_project(&document).unwrap();
        let saved = serde_json::to_value(&project).unwrap();
        assert_eq!(saved["futureSetting"]["a"], 1);
        assert_eq!(saved["tracks"][0]["laneColor"], "red");
        assert_eq!(saved["tracks"][0]["items"][0]["aiTag"], "x");
        assert_eq!(saved["assets"][0]["hash"], "abc");
    }

    #[test]
    fn newer_documents_are_refused() {
        let error = read_project(&serde_json::json!({ "version": 3, "id": "p" })).unwrap_err();
        assert!(error.to_string().contains("newer version"), "{error}");
        assert!(read_project(&serde_json::json!({ "version": 2, "id": "p" })).is_ok());
    }
}
