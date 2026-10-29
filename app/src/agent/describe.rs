//! The project as the model reads it: compact JSON with timeline times,
//! leaving out what has its default value.

use std::collections::HashMap;

use serde_json::{json, Map, Value};
use vf_core::edit::Editor;
use vf_core::engine::animation::{ANIMATIONS_IN, ANIMATIONS_OUT};
use vf_core::engine::effects::{effect_window, EFFECTS};
use vf_core::engine::transitions::TRANSITIONS;
use vf_core::model::{AnimationInKind, AnimationOutKind, ItemKind, MediaAsset, Project, TimelineItem, Transform, TransitionKind};
use vf_core::presets::{aspect_label, ASPECT_PRESETS, SHAPE_PRESETS, STICKER_GROUPS, TEXT_PRESETS};

use super::round;

/// Which part of the timeline to describe.
#[derive(Debug, Clone, Default)]
pub struct Window {
    pub from: Option<f64>,
    pub to: Option<f64>,
    pub track: Option<String>,
    /// Every caption clip instead of one summary per caption group.
    pub captions: bool,
}

fn put(map: &mut Map<String, Value>, key: &str, value: impl Into<Value>) {
    map.insert(key.into(), value.into());
}

fn is_caption(item: &TimelineItem) -> bool {
    item.group_id.as_deref().is_some_and(|g| g.starts_with("captions-"))
}

/// One clip in a few fields: what it is, when, and what differs from the defaults.
pub fn item(item: &TimelineItem) -> Value {
    let mut map = Map::new();
    put(&mut map, "id", item.id.clone());
    put(&mut map, "kind", item.kind.as_str());
    put(&mut map, "start", round(item.start));
    put(&mut map, "end", round(item.end()));
    match item.kind {
        ItemKind::Text => {
            let style = item.text_style();
            put(&mut map, "text", style.content.chars().take(160).collect::<String>());
            put(&mut map, "font", style.font.clone());
            put(&mut map, "size", round(style.size));
            put(&mut map, "color", style.color.clone());
            if style.has_highlight() {
                put(&mut map, "highlight", format!("{} {} word {}", style.highlight_mode, style.highlight_color, style.highlight_index));
            }
        }
        ItemKind::Shape => {
            let shape = item.shape.clone().unwrap_or_default();
            put(&mut map, "shape", format!("{} {}×{}% {}", shape.kind, round(shape.width), round(shape.height), shape.color));
        }
        _ => {
            put(&mut map, "name", item.name.clone());
            if let Some(asset) = item.asset_id.clone() {
                put(&mut map, "asset", asset);
            }
            if item.source_in() > 0.0 {
                put(&mut map, "source_in", round(item.source_in()));
            }
        }
    }
    if item.kind.is_media() && item.kind != ItemKind::Image {
        if (item.speed() - 1.0).abs() > 1e-9 {
            put(&mut map, "speed", round(item.speed()));
        }
        if (item.volume() - 1.0).abs() > 1e-9 {
            put(&mut map, "volume", round(item.volume()));
        }
        if item.muted {
            put(&mut map, "muted", true);
        }
        if item.fade_in() > 0.0 {
            put(&mut map, "fade_in", round(item.fade_in()));
        }
        if item.fade_out() > 0.0 {
            put(&mut map, "fade_out", round(item.fade_out()));
        }
        if let Some(enhance) = item.voice_enhance {
            put(&mut map, "voice_enhance", round(enhance));
        }
    }
    if item.kind.is_visual() {
        let t = item.transform.unwrap_or_default();
        let base = Transform::default();
        for (key, value, default) in [
            ("x", t.x, base.x),
            ("y", t.y, base.y),
            ("scale", t.scale, base.scale),
            ("rotation", t.rotation, base.rotation),
            ("opacity", t.opacity, base.opacity),
        ] {
            if (value - default).abs() > 1e-6 {
                put(&mut map, key, (value * 100.0).round() / 100.0);
            }
        }
        if !item.keyframes.is_empty() {
            let two = |v: f64| (v * 100.0).round() / 100.0;
            let keyframes: Vec<Value> = item
                .keyframes
                .iter()
                .map(|k| {
                    let t = k.transform;
                    let mut entry = json!({ "time": round(k.time), "x": two(t.x), "y": two(t.y), "scale": two(t.scale), "rotation": two(t.rotation), "opacity": two(t.opacity), "easing": k.easing.as_str() });
                    if let Some(curve) = k.bezier.filter(|_| k.easing == vf_core::model::Easing::Custom) {
                        entry["bezier"] = json!(curve);
                    }
                    entry
                })
                .collect();
            put(&mut map, "keyframes", keyframes);
        }
        if let Some(fit) = item.fit {
            if item.kind != ItemKind::Text {
                put(&mut map, "fit", fit.as_str());
            }
        }
        if let Some(blend) = item.blend {
            put(&mut map, "blend", blend.as_str());
        }
        if item.adjust.is_some_and(|a| !a.is_neutral()) {
            put(&mut map, "adjust", serde_json::to_value(item.adjust).unwrap_or_default());
        }
    }
    let effects: Vec<Value> = item
        .active_effects()
        .map(|effect| {
            let window = effect_window(item.duration, effect.range);
            let mut e = Map::new();
            put(&mut e, "id", effect.id.clone());
            put(&mut e, "kind", effect.kind.as_str());
            if (effect.amount() - vf_core::model::DEFAULT_EFFECT_AMOUNT).abs() > 1e-6 {
                put(&mut e, "amount", round(effect.amount()));
            }
            if effect.range.is_some() {
                put(&mut e, "from", round(item.start + window.start));
                put(&mut e, "to", round(item.start + window.start + window.duration));
            }
            if let Some(n) = effect.on_beat {
                put(&mut e, "on_beat", n);
            }
            Value::Object(e)
        })
        .collect();
    if !effects.is_empty() {
        put(&mut map, "effects", effects);
    }
    if let Some(transition) = item.transition.filter(|t| t.kind != TransitionKind::None) {
        put(&mut map, "transition", format!("{} {}s", transition.kind, round(transition.duration)));
    }
    if let Some(anim) = item.anim_in.filter(|a| a.kind != AnimationInKind::None) {
        put(&mut map, "anim_in", format!("{} {}s", anim.kind, round(anim.duration)));
    }
    if let Some(anim) = item.anim_out.filter(|a| a.kind != AnimationOutKind::None) {
        put(&mut map, "anim_out", format!("{} {}s", anim.kind, round(anim.duration)));
    }
    if let Some(group) = &item.group_id {
        put(&mut map, "group", group.clone());
    }
    if item.hidden {
        put(&mut map, "hidden", true);
    }
    if item.locked {
        put(&mut map, "locked", true);
    }
    Value::Object(map)
}

/// A caption group in one entry: how many, when, what they say and how they look.
fn caption_summary(group: &str, items: &[&TimelineItem]) -> Value {
    let words: Vec<String> = items.iter().map(|i| i.text_style().content).collect();
    let mut text = words.join(" ");
    if text.chars().count() > 300 {
        text = text.chars().take(300).collect::<String>() + "…";
    }
    let style = items[0].text_style();
    json!({
        "captions": group,
        "count": items.len(),
        "start": round(items.iter().map(|i| i.start).fold(f64::INFINITY, f64::min)),
        "end": round(items.iter().map(|i| i.end()).fold(0.0, f64::max)),
        "text": text,
        "font": style.font,
        "size": round(style.size),
        "color": style.color,
        "y": round(items[0].transform.unwrap_or_default().y),
        "note": "ask get_project with include_captions (or a from/to window) for each caption's id",
    })
}

/// The open project, its timeline and the editor's state.
pub fn project(editor: &Editor, window: &Window) -> Value {
    let project = &editor.project;
    let tracks: Vec<Value> = project
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, track)| window.track.as_ref().is_none_or(|id| *id == track.id))
        .map(|(index, track)| {
            let visible: Vec<&TimelineItem> =
                track.items.iter().filter(|item| window.from.is_none_or(|from| item.end() > from) && window.to.is_none_or(|to| item.start < to)).collect();
            let summarise = !window.captions && window.from.is_none() && window.to.is_none();
            let mut items = Vec::new();
            let mut groups: Vec<(String, Vec<&TimelineItem>)> = Vec::new();
            for item in visible {
                match item.group_id.as_deref().filter(|_| summarise && is_caption(item)) {
                    Some(group) => match groups.iter_mut().find(|(g, _)| g == group) {
                        Some((_, members)) => members.push(item),
                        None => {
                            groups.push((group.to_string(), vec![item]));
                            items.push(Value::Null);
                        }
                    },
                    None => items.push(self::item(item)),
                }
            }
            // Each group's summary sits where its first caption was.
            let mut summaries = groups.iter().map(|(group, members)| caption_summary(group, members));
            let items: Vec<Value> = items.into_iter().map(|v| if v.is_null() { summaries.next().unwrap_or_default() } else { v }).collect();
            let mut map = Map::new();
            put(&mut map, "id", track.id.clone());
            put(&mut map, "kind", track.kind.as_str());
            put(&mut map, "name", track.name.clone());
            put(&mut map, "layer", index);
            for (key, on) in [("locked", track.locked), ("muted", track.muted), ("hidden", track.hidden)] {
                if on {
                    put(&mut map, key, true);
                }
            }
            put(&mut map, "items", items);
            Value::Object(map)
        })
        .collect();
    let x = &project.export;
    let mut root = json!({
        "project": {
            "id": project.id,
            "name": project.name,
            "width": project.width,
            "height": project.height,
            "aspect": aspect_label(project.width, project.height),
            "fps": project.fps,
            "background": project.background,
            "duration": round(editor.playback_end()),
        },
        "playhead": round(editor.time),
        "selection": editor.selection,
        "tracks": tracks,
        "media_files": project.assets.len(),
        "export": { "format": x.format.as_str(), "codec": x.codec.as_str(), "quality": x.quality.as_str(), "resolution": x.resolution, "fps": x.fps, "loudness": x.loudness },
    });
    if !project.beats.is_empty() {
        root["beats"] = json!({ "count": project.beats.len(), "first": project.beats.iter().take(8).map(|b| round(*b)).collect::<Vec<_>>() });
    }
    if project.tracks.is_empty() {
        root["note"] = json!("The timeline is empty. Media in the library: see list_media; import files with import_media.");
    }
    root
}

/// The project for the chat's context: the timeline as `project` gives it,
/// shortened for very large projects.
pub fn context(editor: &Editor) -> String {
    let full = project(editor, &Window::default());
    let text = serde_json::to_string(&full).unwrap_or_default();
    if text.len() <= 24_000 {
        return text;
    }
    // Only the tracks' outlines; the model asks for the details.
    let mut outline = full.clone();
    if let Some(tracks) = outline.get_mut("tracks").and_then(Value::as_array_mut) {
        for track in tracks {
            let count = track["items"].as_array().map_or(0, Vec::len);
            track["items"] = json!(format!("{count} clips: call get_project with a from/to window or track_id to list them"));
        }
    }
    serde_json::to_string(&outline).unwrap_or_default()
}

/// The media library.
pub fn media(project: &Project) -> Value {
    let mut uses: HashMap<&str, usize> = HashMap::new();
    for item in project.items() {
        if let Some(id) = item.asset_id.as_deref() {
            *uses.entry(id).or_default() += 1;
        }
    }
    let files: Vec<Value> = project.assets.iter().map(|asset| asset_entry(asset, uses.get(asset.id.as_str()).copied().unwrap_or(0))).collect();
    json!({ "summary": format!("{} media files", files.len()), "media": files })
}

fn asset_entry(asset: &MediaAsset, clips: usize) -> Value {
    let mut map = Map::new();
    put(&mut map, "id", asset.id.clone());
    put(&mut map, "kind", asset.kind.as_str());
    put(&mut map, "name", asset.name.clone());
    put(&mut map, "path", asset.path.clone());
    if let Some(duration) = asset.duration {
        put(&mut map, "duration", round(duration));
    }
    if let (Some(w), Some(h)) = (asset.width, asset.height) {
        put(&mut map, "size", format!("{w}×{h}"));
    }
    if let Some(fps) = asset.fps {
        put(&mut map, "fps", round(fps));
    }
    if asset.has_audio == Some(true) && asset.kind != vf_core::model::MediaKind::Audio {
        put(&mut map, "has_audio", true);
    }
    put(&mut map, "on_timeline", clips);
    if !std::path::Path::new(&asset.path).is_file() {
        put(&mut map, "missing", true);
    }
    Value::Object(map)
}

/// What the editor offers, by topic ("all" for everything).
pub fn catalog(topic: &str, fonts: Vec<String>) -> Value {
    let all = topic == "all" || topic.is_empty();
    let mut out = Map::new();
    if all || topic == "effects" {
        let effects: Vec<Value> = EFFECTS.iter().map(|e| json!({ "kind": e.kind.as_str(), "name": e.label, "does": e.description })).collect();
        put(&mut out, "effects", effects);
    }
    if all || topic == "transitions" {
        let transitions: Vec<Value> = TRANSITIONS.iter().map(|t| json!({ "kind": t.kind.as_str(), "name": t.label })).collect();
        put(&mut out, "transitions", transitions);
    }
    if all || topic == "animations" {
        let anim_in: Vec<Value> = ANIMATIONS_IN.iter().skip(1).map(|a| json!({ "kind": a.kind.as_str(), "name": a.label, "duration": a.duration })).collect();
        let anim_out: Vec<Value> = ANIMATIONS_OUT.iter().skip(1).map(|a| json!({ "kind": a.kind.as_str(), "name": a.label, "duration": a.duration })).collect();
        put(&mut out, "anim_in", anim_in);
        put(&mut out, "anim_out", anim_out);
    }
    if all || topic == "text" {
        let presets: Vec<Value> = TEXT_PRESETS
            .iter()
            .map(|p| {
                let s = p.text_style();
                let mut look = format!("{} {} {}", s.font, s.weight, s.color);
                if s.stroke_width > 0.0 {
                    look += &format!(", outline {}", s.stroke_color);
                }
                if s.background_opacity > 0.0 {
                    look += &format!(", box {}", s.background_color);
                }
                if s.has_highlight() {
                    look += &format!(", highlights the spoken word ({} {})", s.highlight_mode, s.highlight_color);
                }
                json!({ "id": p.id, "name": p.label, "category": p.category.label(), "look": look })
            })
            .collect();
        put(&mut out, "text_presets", presets);
    }
    if all || topic == "fonts" {
        put(&mut out, "fonts", fonts);
    }
    if all || topic == "stickers" {
        let stickers: Map<String, Value> = STICKER_GROUPS.iter().map(|(group, emoji)| (group.to_string(), json!(emoji.join(" ")))).collect();
        put(&mut out, "stickers", Value::Object(stickers));
        let shapes: Vec<Value> = SHAPE_PRESETS.iter().map(|s| json!({ "kind": s.kind.as_str(), "name": s.label })).collect();
        put(&mut out, "shapes", shapes);
    }
    if all {
        let aspects: Vec<Value> =
            ASPECT_PRESETS.iter().map(|p| json!({ "aspect": p.id, "size": format!("{}×{}", p.width, p.height), "for": p.hint })).collect();
        put(&mut out, "canvas_formats", aspects);
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vf_core::edit::{CaptionOptions, Placement};
    use vf_core::model::CaptionCue;
    use vf_core::presets::{create_project, text_item};

    #[test]
    fn captions_are_summarised_unless_asked_for() {
        let mut editor = Editor::new(create_project("Test", 1080, 1920, 30));
        editor.add_item(text_item("Title", None), Placement { start: Some(0.0), ..Default::default() });
        let cues: Vec<CaptionCue> =
            (0..30).map(|i| CaptionCue { text: format!("word{i}"), start: i as f64 * 0.5, duration: 0.5, highlight_index: None }).collect();
        editor.apply_captions(&cues, CaptionOptions { source_id: "v".into(), offset: 0.0, speed: 1.0, style: Default::default(), y: 72.0 });
        let summary = project(&editor, &Window::default());
        let text = summary.to_string();
        assert!(text.contains("\"captions\":\"captions-v\"") && text.contains("\"count\":30"), "{text}");
        assert!(!text.contains("word29\",\"font"), "captions are not listed one by one");
        let full = project(&editor, &Window { captions: true, ..Default::default() });
        let count = full["tracks"].as_array().unwrap().iter().map(|t| t["items"].as_array().unwrap().len()).sum::<usize>();
        assert_eq!(count, 31);
        let window = project(&editor, &Window { from: Some(2.0), to: Some(3.0), ..Default::default() });
        let count = window["tracks"].as_array().unwrap().iter().map(|t| t["items"].as_array().unwrap().len()).sum::<usize>();
        assert_eq!(count, 2 + 1, "two captions and the title in the window");
        assert!(context(&editor).len() < 24_000);
    }

    #[test]
    fn catalog_lists_by_topic() {
        let all = catalog("all", vec!["Inter".into()]);
        assert!(all["effects"].as_array().unwrap().len() > 10);
        assert_eq!(all["fonts"][0], "Inter");
        let effects = catalog("effects", vec![]);
        assert!(effects.get("transitions").is_none());
    }
}
