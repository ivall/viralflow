//! The editing tools: changes to the open project through `Editor`, with no
//! UI and no files, so they are tested directly. Each validates its arguments
//! and says what went wrong in words a model can act on; each reports clips
//! as they are afterwards (an overlap may have moved one).

use serde_json::{json, Value};
use vf_core::captions::highlight_cues;
use vf_core::edit::{can_have_effects, CaptionOptions, Editor, Placement, TrackPatch, TransformPatch, TrimOrigin, MAX_EFFECTS};
use vf_core::engine::animation::{default_in_duration, default_out_duration};
use vf_core::engine::color::is_hex_color;
use vf_core::engine::effects::effect_window;
use vf_core::engine::keyframes::{normalize_transform, sorted_keyframes, MAX_SCALE, MIN_SCALE};
use vf_core::engine::math::{sanitize_bezier, uid};
use vf_core::model::*;
use vf_core::presets::{caption_style, media_item, preset_text_style, shape_item, sticker_item, text_item, text_preset, TextPreset, ASPECT_PRESETS};
use vf_core::timeline::{Edge, MIN_CLIP_DURATION};

use super::args::{self, Args};
use super::{describe, round, specs};

type Outcome = Result<Value, String>;

/// Runs an editing tool; `None` when `name` is not one of them.
pub fn run(editor: &mut Editor, name: &str, value: &Value) -> Option<Outcome> {
    let a = match args::object(value) {
        Ok(a) => a,
        Err(error) => return Some(Err(error)),
    };
    let result = match name {
        "get_project" => get_project(editor, &a),
        "get_items" => get_items(editor, &a),
        "list_media" => Ok(describe::media(&editor.project)),
        "add_clips" => add_clips(editor, &a),
        "add_texts" => add_texts(editor, &a),
        "add_stickers" => add_stickers(editor, &a),
        "add_shapes" => add_shapes(editor, &a),
        "update_items" => update_items(editor, &a),
        "delete_items" => delete_items(editor, &a),
        "split_items" => split_items(editor, &a),
        "trim_items" => trim_items(editor, &a),
        "remove_time_ranges" => remove_time_ranges(editor, &a),
        "duplicate_items" => duplicate_items(editor, &a),
        "detach_audio" => detach_audio(editor, &a),
        "add_effect" => add_effect(editor, &a),
        "set_effects" => set_effects(editor, &a),
        "set_transitions" => set_transitions(editor, &a),
        "set_captions" => set_captions(editor, &a),
        "edit_tracks" => edit_tracks(editor, &a),
        "set_project" => set_project(editor, &a),
        "history" => history(editor, &a),
        _ => return None,
    };
    Some(result)
}

// ------------------------------------------------------------ helpers

fn clip<'a>(editor: &'a Editor, id: &str) -> Result<&'a TimelineItem, String> {
    editor.item(id).ok_or_else(|| format!("There is no clip with id \"{id}\". Call get_project for the current ids."))
}

/// A clip that may be changed, or why not.
fn editable(editor: &Editor, id: &str) -> Result<(), String> {
    let item = clip(editor, id)?;
    let (t, _) = editor.find(id).unwrap_or_default();
    let track = &editor.project.tracks[t];
    if track.locked {
        return Err(format!("Clip {id} is on the locked track \"{}\" ({}); unlock it with edit_tracks first.", track.name, track.id));
    }
    if item.locked {
        return Err(format!("Clip {id} is locked; update_items with locked: false unlocks it."));
    }
    Ok(())
}

fn report(editor: &Editor, id: &str, notes: Vec<String>) -> Value {
    let mut value = editor.item(id).map(describe::item).unwrap_or_else(|| json!({ "id": id, "removed": true }));
    if !notes.is_empty() {
        value["notes"] = json!(notes);
    }
    value
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// Fields of `fields` other than `taken`.
fn without(fields: &Args, taken: &[&str]) -> Args {
    fields.iter().filter(|(k, _)| !taken.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()
}

fn summary(value: Value, text: String) -> Value {
    let mut value = value;
    value["summary"] = json!(text);
    value
}

/// End of the last clip on tracks of `kind` (where a new clip is appended).
fn end_of(editor: &Editor, kind: TrackKind) -> f64 {
    editor.project.tracks.iter().filter(|t| t.kind == kind).flat_map(|t| t.items.iter()).map(TimelineItem::end).fold(0.0, f64::max)
}

fn track_for(editor: &Editor, id: &str, kind: ItemKind) -> Result<String, String> {
    let track = editor.project.tracks.iter().find(|t| t.id == id).ok_or_else(|| format!("There is no track with id \"{id}\"."))?;
    if track.kind != kind.track_kind() {
        return Err(format!("Track {id} is a {} track; a {} clip goes on a {} track.", track.kind, kind, kind.track_kind()));
    }
    if track.locked {
        return Err(format!("Track {id} is locked."));
    }
    Ok(track.id.clone())
}

/// Longest a media clip can be from `source_in` at `speed`.
fn source_room(editor: &Editor, item: &TimelineItem, source_in: f64, speed: f64) -> f64 {
    match (item.kind, editor.asset(item.asset_id.as_deref()).and_then(|a| a.duration)) {
        (ItemKind::Video | ItemKind::Audio, Some(length)) => ((length - source_in) / speed).max(MIN_CLIP_DURATION),
        _ => f64::INFINITY,
    }
}

/// The effect range to store: none when it covers the whole clip.
fn stored_range(duration: f64, window: EffectRange) -> Option<EffectRange> {
    (!(window.start <= 1e-6 && window.duration >= duration - 1e-6)).then_some(window)
}

fn effect_range(item: &TimelineItem, from: Option<f64>, to: Option<f64>) -> Result<Option<EffectRange>, String> {
    if from.is_none() && to.is_none() {
        return Ok(None);
    }
    let from = from.unwrap_or(item.start).max(item.start);
    let to = to.unwrap_or(item.end()).min(item.end());
    if to - from < 0.05 {
        return Err(format!("from..to must lie inside clip {} ({}..{} s).", item.id, round(item.start), round(item.end())));
    }
    Ok(stored_range(item.duration, effect_window(item.duration, Some(EffectRange { start: from - item.start, duration: to - from }))))
}

// ------------------------------------------------------------ looking

fn get_project(editor: &Editor, a: &Args) -> Outcome {
    let window = describe::Window {
        from: args::number(a, "from")?,
        to: args::number(a, "to")?,
        track: args::string(a, "track_id")?,
        captions: args::boolean(a, "include_captions")?.unwrap_or(false),
    };
    Ok(describe::project(editor, &window))
}

fn get_items(editor: &Editor, a: &Args) -> Outcome {
    let ids = args::strings(a, "item_ids")?;
    if ids.is_empty() {
        return Err("`item_ids` is empty.".into());
    }
    let items: Vec<Value> = ids
        .iter()
        .map(|id| match editor.item(id) {
            Some(item) => serde_json::to_value(item).unwrap_or_default(),
            None => json!({ "id": id, "error": "no such clip" }),
        })
        .collect();
    Ok(json!({ "items": items }))
}

// ------------------------------------------------------------ adding

fn add_clips(editor: &mut Editor, a: &Args) -> Outcome {
    let clips = args::objects(a, "clips")?;
    if clips.is_empty() {
        return Err("`clips` is empty.".into());
    }
    let mut results = Vec::new();
    let mut created = Vec::new();
    for fields in clips {
        match add_clip(editor, &fields) {
            Ok((id, notes)) => {
                results.push(report(editor, &id, notes));
                created.push(id);
            }
            Err(error) => results.push(json!({ "asset_id": fields.get("asset_id"), "error": error })),
        }
    }
    editor.select_many(&created);
    let failed = results.len() - created.len();
    let mut text = format!("Added {}", plural(created.len(), "clip", "clips"));
    if failed > 0 {
        text += &format!("; {failed} failed");
    }
    Ok(summary(json!({ "clips": results }), text))
}

fn add_clip(editor: &mut Editor, fields: &Args) -> Result<(String, Vec<String>), String> {
    let asset_id = args::required_string(fields, "asset_id")?;
    let asset = editor
        .project
        .assets
        .iter()
        .find(|a| a.id == asset_id)
        .cloned()
        .ok_or_else(|| format!("There is no media with id \"{asset_id}\". Call list_media, or import_media first."))?;
    let mut template = media_item(&asset, &editor.project);
    let rest = without(fields, &["asset_id", "new_track", "source_in", "duration", "start", "track_id"]);
    if let Some(message) = args::unknown_keys(&rest, UPDATE_FIELDS) {
        return Err(message);
    }
    let length = asset.duration.filter(|d| *d > 0.0);
    if let Some(source_in) = args::number(fields, "source_in")? {
        if template.kind == ItemKind::Image {
            return Err("source_in applies to video and audio.".into());
        }
        let source_in = source_in.clamp(0.0, length.map_or(f64::INFINITY, |l| (l - MIN_CLIP_DURATION).max(0.0)));
        template.source_in = Some(source_in);
        if let Some(length) = length {
            template.duration = length - source_in;
        }
    }
    if let Some(duration) = args::number(fields, "duration")? {
        let room = source_room(editor, &template, template.source_in(), 1.0);
        template.duration = duration.clamp(MIN_CLIP_DURATION, room);
    }
    let kind = template.kind;
    let start = args::number(fields, "start")?.unwrap_or_else(|| end_of(editor, kind.track_kind())).max(0.0);
    let placement = if args::boolean(fields, "new_track")?.unwrap_or(false) {
        // Above the other tracks of its kind (b-roll over the main video).
        let index = editor.project.tracks.iter().position(|t| t.kind == kind.track_kind()).unwrap_or(0);
        Placement { start: Some(start), new_track_index: Some(index), ..Default::default() }
    } else {
        let track_id = args::string(fields, "track_id")?.map(|id| track_for(editor, &id, kind)).transpose()?;
        Placement { start: Some(start), track_id, ..Default::default() }
    };
    let id = editor.add_item(template, placement).ok_or("The clip could not be added.")?;
    let mut notes = Vec::new();
    if let Some(item) = editor.item(&id) {
        if (item.start - start).abs() > 1e-3 {
            notes.push(format!("placed at {} s: {} s was taken on that track", round(item.start), round(start)));
        }
    }
    if let Err(error) = apply_update(editor, &id, &rest, &mut notes) {
        discard(editor, &id);
        return Err(error);
    }
    Ok((id, notes))
}

fn add_texts(editor: &mut Editor, a: &Args) -> Outcome {
    let texts = args::objects(a, "texts")?;
    if texts.is_empty() {
        return Err("`texts` is empty.".into());
    }
    let mut results = Vec::new();
    let mut created = Vec::new();
    for fields in texts {
        let added = (|| -> Result<(String, Vec<String>), String> {
            let content = args::required_string(&fields, "text")?;
            let preset = args::choice(&fields, "preset", text_preset, &specs::preset_names())?;
            let rest = without(&fields, &["text", "preset", "start", "duration", "track_id"]);
            if let Some(message) = args::unknown_keys(&rest, UPDATE_FIELDS) {
                return Err(message);
            }
            let mut template = text_item(&content, preset);
            template.duration = args::number(&fields, "duration")?.unwrap_or(3.0).max(MIN_CLIP_DURATION);
            let start = args::number(&fields, "start")?.unwrap_or(editor.time).max(0.0);
            let track_id = args::string(&fields, "track_id")?.map(|id| track_for(editor, &id, ItemKind::Text)).transpose()?;
            let id = editor.add_item(template, Placement { start: Some(start), track_id, ..Default::default() }).ok_or("The text could not be added.")?;
            let mut notes = Vec::new();
            if let Err(error) = apply_update(editor, &id, &rest, &mut notes) {
                discard(editor, &id);
                return Err(error);
            }
            Ok((id, notes))
        })();
        match added {
            Ok((id, notes)) => {
                results.push(report(editor, &id, notes));
                created.push(id);
            }
            Err(error) => results.push(json!({ "text": fields.get("text"), "error": error })),
        }
    }
    editor.select_many(&created);
    Ok(summary(json!({ "texts": results }), format!("Added {}", plural(created.len(), "text", "texts"))))
}

fn add_stickers(editor: &mut Editor, a: &Args) -> Outcome {
    let stickers = args::objects(a, "stickers")?;
    let mut results = Vec::new();
    let mut created = Vec::new();
    for fields in stickers {
        let added = (|| -> Result<String, String> {
            let emoji = args::required_string(&fields, "emoji")?;
            let rest = without(&fields, &["emoji", "start", "duration"]);
            if let Some(message) = args::unknown_keys(&rest, &["x", "y", "scale", "rotation", "opacity"]) {
                return Err(message);
            }
            let mut template = sticker_item(emoji.trim());
            template.duration = args::number(&fields, "duration")?.unwrap_or(3.0).max(MIN_CLIP_DURATION);
            let start = args::number(&fields, "start")?.unwrap_or(editor.time).max(0.0);
            let id = editor.add_item(template, Placement { start: Some(start), ..Default::default() }).ok_or("The sticker could not be added.")?;
            if let Err(error) = apply_update(editor, &id, &rest, &mut Vec::new()) {
                discard(editor, &id);
                return Err(error);
            }
            Ok(id)
        })();
        match added {
            Ok(id) => {
                results.push(report(editor, &id, vec![]));
                created.push(id);
            }
            Err(error) => results.push(json!({ "error": error })),
        }
    }
    editor.select_many(&created);
    Ok(summary(json!({ "stickers": results }), format!("Added {}", plural(created.len(), "sticker", "stickers"))))
}

fn add_shapes(editor: &mut Editor, a: &Args) -> Outcome {
    let shapes = args::objects(a, "shapes")?;
    let mut results = Vec::new();
    let mut created = Vec::new();
    for fields in shapes {
        let added = (|| -> Result<String, String> {
            let kind = args::choice(&fields, "kind", ShapeKind::parse, &["rect", "circle", "line", "blur"])?.ok_or("`kind` is required")?;
            let rest = without(&fields, &["kind", "start", "duration", "width", "height", "color", "blur"]);
            if let Some(message) = args::unknown_keys(&rest, &["x", "y", "scale", "rotation", "opacity", "radius", "name", "anim_in", "anim_out", "hidden"]) {
                return Err(message);
            }
            let mut template = shape_item(kind);
            template.duration = args::number(&fields, "duration")?.unwrap_or(3.0).max(MIN_CLIP_DURATION);
            let start = args::number(&fields, "start")?.unwrap_or(editor.time).max(0.0);
            let mut shape_fields = Args::new();
            for key in ["width", "height", "color", "blur"] {
                if let Some(value) = fields.get(key) {
                    shape_fields.insert(key.into(), value.clone());
                }
            }
            let id = editor.add_item(template, Placement { start: Some(start), ..Default::default() }).ok_or("The shape could not be added.")?;
            let mut update = rest;
            if !shape_fields.is_empty() {
                update.insert("shape".into(), Value::Object(shape_fields));
            }
            if let Err(error) = apply_update(editor, &id, &update, &mut Vec::new()) {
                discard(editor, &id);
                return Err(error);
            }
            Ok(id)
        })();
        match added {
            Ok(id) => {
                results.push(report(editor, &id, vec![]));
                created.push(id);
            }
            Err(error) => results.push(json!({ "error": error })),
        }
    }
    editor.select_many(&created);
    Ok(summary(json!({ "shapes": results }), format!("Added {}", plural(created.len(), "shape", "shapes"))))
}

// ------------------------------------------------------------ changing clips

const UPDATE_FIELDS: &[&str] = &[
    "name",
    "start",
    "duration",
    "track_id",
    "source_in",
    "speed",
    "x",
    "y",
    "scale",
    "rotation",
    "opacity",
    "keyframes",
    "text",
    "preset",
    "style",
    "shape",
    "volume",
    "muted",
    "fade_in",
    "fade_out",
    "voice_enhance",
    "fit",
    "focus",
    "crop",
    "flip_x",
    "flip_y",
    "radius",
    "blend",
    "adjust",
    "anim_in",
    "anim_out",
    "transition",
    "hidden",
    "locked",
    "asset_id",
    "group",
];

const STYLE_FIELDS: &[&str] = &[
    "font",
    "size",
    "weight",
    "italic",
    "color",
    "align",
    "uppercase",
    "letter_spacing",
    "line_height",
    "max_width",
    "stroke_color",
    "stroke_width",
    "background_color",
    "background_opacity",
    "background_padding",
    "background_radius",
    "shadow_color",
    "shadow_opacity",
    "shadow_blur",
    "shadow_distance",
    "highlight_mode",
    "highlight_color",
    "highlight_text_color",
    "highlight_index",
];

fn update_items(editor: &mut Editor, a: &Args) -> Outcome {
    let updates = args::objects(a, "updates")?;
    if updates.is_empty() {
        return Err("`updates` is empty.".into());
    }
    let mut results = Vec::new();
    let mut changed = 0;
    for fields in updates {
        let id = match args::required_string(&fields, "id") {
            Ok(id) => id,
            Err(error) => {
                results.push(json!({ "error": error }));
                continue;
            }
        };
        let mut notes = Vec::new();
        match apply_update(editor, &id, &without(&fields, &["id"]), &mut notes) {
            Ok(()) => {
                changed += 1;
                results.push(report(editor, &id, notes));
            }
            Err(error) => results.push(json!({ "id": id, "error": error })),
        }
    }
    let failed = results.len() - changed;
    let mut text = format!("Changed {}", plural(changed, "clip", "clips"));
    if failed > 0 {
        text += &format!("; {failed} failed");
    }
    Ok(summary(json!({ "items": results }), text))
}

/// Everything read from an update before anything changes, so a bad value changes nothing.
#[derive(Default)]
struct Props {
    name: Option<String>,
    transform: TransformPatch,
    text: Option<TextStyle>,
    anim_in_from_preset: Option<Option<Animation<AnimationInKind>>>,
    shape: Option<ShapeStyle>,
    volume: Option<f64>,
    muted: Option<bool>,
    fade_in: Option<f64>,
    fade_out: Option<f64>,
    voice_enhance: Option<Option<f64>>,
    fit: Option<Fit>,
    focus: Option<Point>,
    crop: Option<Option<Crop>>,
    flip_x: Option<bool>,
    flip_y: Option<bool>,
    radius: Option<f64>,
    blend: Option<Option<BlendMode>>,
    adjust: Option<Option<Adjust>>,
    anim_in: Option<Option<Animation<AnimationInKind>>>,
    anim_out: Option<Option<Animation<AnimationOutKind>>>,
    hidden: Option<bool>,
    group: Option<Option<String>>,
    media: Option<MediaAsset>,
}

fn require(ok: bool, fields: &[&str], what: &str, item: &TimelineItem, given: &Args) -> Result<(), String> {
    let used: Vec<&str> = fields.iter().copied().filter(|f| given.contains_key(*f)).collect();
    if ok || used.is_empty() {
        return Ok(());
    }
    Err(format!("{} apply to {what}; {} is a {} clip.", used.join(", "), item.id, item.kind))
}

fn apply_update(editor: &mut Editor, id: &str, fields: &Args, notes: &mut Vec<String>) -> Result<(), String> {
    if let Some(message) = args::unknown_keys(fields, UPDATE_FIELDS) {
        return Err(message);
    }
    if fields.is_empty() {
        return Ok(());
    }
    let item = clip(editor, id)?.clone();
    let (t, _) = editor.find(id).unwrap_or_default();
    let track = &editor.project.tracks[t];
    if track.locked {
        return Err(format!("Clip {id} is on the locked track \"{}\" ({}); unlock it with edit_tracks first.", track.name, track.id));
    }
    let lock = args::boolean(fields, "locked")?;
    let unlock = lock == Some(false);
    if item.locked && !unlock {
        return Err(format!("Clip {id} is locked; set locked: false to change it."));
    }
    let kind = item.kind;
    let sound = kind.has_sound();
    let picture = matches!(kind, ItemKind::Video | ItemKind::Image);
    require(kind == ItemKind::Text, &["text", "preset", "style"], "text clips", &item, fields)?;
    require(kind == ItemKind::Shape, &["shape"], "shape clips", &item, fields)?;
    require(sound, &["volume", "muted", "fade_in", "fade_out", "voice_enhance"], "clips with sound (video, audio)", &item, fields)?;
    require(sound, &["source_in", "speed"], "video and audio clips", &item, fields)?;
    require(picture, &["fit", "focus", "crop"], "videos and photos", &item, fields)?;
    require(kind.is_media(), &["asset_id"], "video, photo and audio clips", &item, fields)?;
    require(
        kind.is_visual(),
        &["x", "y", "scale", "rotation", "opacity", "keyframes", "flip_x", "flip_y", "radius", "blend", "adjust", "anim_in", "anim_out", "transition"],
        "visual clips (not audio)",
        &item,
        fields,
    )?;

    // Read every value first.
    let mut props = Props { name: args::string(fields, "name")?, hidden: args::boolean(fields, "hidden")?, ..Default::default() };
    props.transform = TransformPatch {
        x: args::number(fields, "x")?,
        y: args::number(fields, "y")?,
        scale: args::number(fields, "scale")?.map(|v| v.clamp(MIN_SCALE, MAX_SCALE)),
        rotation: args::number(fields, "rotation")?,
        opacity: args::number(fields, "opacity")?.map(|v| v.clamp(0.0, 1.0)),
    };
    let content = args::string(fields, "text")?;
    let preset = args::choice(fields, "preset", text_preset, &specs::preset_names())?;
    let style = match fields.get("style") {
        None | Some(Value::Null) => None,
        Some(value) => Some(args::object(value)?),
    };
    if content.is_some() || preset.is_some() || style.is_some() {
        props.text = Some(text_style(&item.text_style(), preset, style.as_ref(), content.as_deref())?);
        if let Some(preset) = preset {
            if !fields.contains_key("anim_in") {
                props.anim_in_from_preset = Some(preset.animation());
            }
        }
    }
    if let Some(value) = fields.get("shape") {
        props.shape = Some(shape_style(&item.shape.clone().unwrap_or_default(), &args::object(value)?)?);
    }
    props.volume = args::number(fields, "volume")?.map(|v| v.clamp(0.0, 4.0));
    props.muted = args::boolean(fields, "muted")?;
    props.fade_in = args::number(fields, "fade_in")?.map(|v| v.max(0.0));
    props.fade_out = args::number(fields, "fade_out")?.map(|v| v.max(0.0));
    props.voice_enhance = args::number(fields, "voice_enhance")?.map(|v| (v > 0.0).then_some(v.min(1.0)));
    props.fit = args::choice(fields, "fit", Fit::parse, &["contain", "cover", "fill"])?;
    if let Some(value) = fields.get("focus") {
        let focus = args::object(value)?;
        let current = item.focus.unwrap_or_default();
        props.focus = Some(Point {
            x: args::number(&focus, "x")?.unwrap_or(current.x).clamp(0.0, 100.0),
            y: args::number(&focus, "y")?.unwrap_or(current.y).clamp(0.0, 100.0),
        });
    }
    if let Some(value) = fields.get("crop") {
        props.crop = Some(crop(&args::object(value)?)?);
    }
    props.flip_x = args::boolean(fields, "flip_x")?;
    props.flip_y = args::boolean(fields, "flip_y")?;
    props.radius = args::number(fields, "radius")?.map(|v| v.max(0.0));
    let blend_names = specs::names_of(BlendMode::ALL);
    let blend_names: Vec<&str> = blend_names.iter().map(String::as_str).collect();
    props.blend = args::choice(fields, "blend", BlendMode::parse, &blend_names)?.map(|b| (b != BlendMode::Normal).then_some(b));
    if let Some(value) = fields.get("adjust") {
        props.adjust = Some(adjust(item.adjust.unwrap_or_default(), &args::object(value)?)?);
    }
    if let Some(value) = fields.get("anim_in") {
        props.anim_in = Some(animation(value, AnimationInKind::parse, &specs::anim_in_names(), default_in_duration, item.duration)?);
    }
    if let Some(value) = fields.get("anim_out") {
        props.anim_out = Some(animation(value, AnimationOutKind::parse, &specs::anim_out_names(), default_out_duration, item.duration)?);
    }
    let transition = match fields.get("transition") {
        None => None,
        Some(Value::Null) => Some((TransitionKind::None, None)),
        Some(Value::String(kind)) => Some((parse_transition(kind)?, None)),
        Some(value) => {
            let t = args::object(value)?;
            let kind = args::string(&t, "kind")?.map(|k| parse_transition(&k)).transpose()?.unwrap_or(TransitionKind::Fade);
            Some((kind, args::number(&t, "duration")?.map(|d| d.clamp(0.05, 5.0))))
        }
    };
    let keyframes = if fields.contains_key("keyframes") { Some(keyframes(fields, &item)?) } else { None };
    if let Some(group) = fields.get("group") {
        props.group = Some(match group {
            Value::Null => None,
            _ => args::string(fields, "group")?.map(|g| g.trim().to_string()).filter(|g| !g.is_empty()),
        });
    }
    if let Some(asset_id) = args::string(fields, "asset_id")? {
        let asset = editor
            .project
            .assets
            .iter()
            .find(|a| a.id == asset_id)
            .cloned()
            .ok_or_else(|| format!("There is no media with id \"{asset_id}\". Call list_media, or import_media first."))?;
        if (kind == ItemKind::Audio) != (asset.kind == MediaKind::Audio) {
            return Err(format!("{} is {} and can't replace the media of a {} clip.", asset.name, asset.kind, kind));
        }
        props.media = Some(asset);
    }
    let speed = args::number(fields, "speed")?.map(|v| v.clamp(0.1, 10.0));
    let source_in = args::number(fields, "source_in")?;
    let start = args::number(fields, "start")?.map(|v| v.max(0.0));
    let duration = args::number(fields, "duration")?.map(|v| v.max(MIN_CLIP_DURATION));
    let move_to = args::string(fields, "track_id")?.filter(|track| *track != item.track_id).map(|track| track_for(editor, &track, kind)).transpose()?;

    // Then change.
    if unlock {
        editor.begin_change();
        if let Some(target) = editor.project.item_mut(id) {
            target.locked = false;
        }
    }
    let animated = !item.keyframes.is_empty() && keyframes.is_none() && props.transform != TransformPatch::default();
    if animated {
        notes.push("the clip moves with keyframes: x/y/scale/rotation/opacity were set on every keyframe".into());
    }
    editor.update_item(id, true, |item| {
        if let Some(name) = props.name.take() {
            item.name = name;
        }
        if props.transform != TransformPatch::default() {
            item.transform = Some(normalize_transform(Some(&props.transform.apply(normalize_transform(item.transform.as_ref())))));
            for keyframe in &mut item.keyframes {
                keyframe.transform = props.transform.apply(keyframe.transform);
            }
        }
        if let Some(text) = props.text.take() {
            item.text = Some(text);
        }
        if let Some(anim) = props.anim_in_from_preset {
            item.anim_in = anim;
        }
        if let Some(shape) = props.shape.take() {
            item.shape = Some(shape);
        }
        if let Some(volume) = props.volume {
            item.volume = Some(volume);
        }
        if let Some(muted) = props.muted {
            item.muted = muted;
        }
        if let Some(fade) = props.fade_in {
            item.fade_in = Some(fade.min(item.duration));
        }
        if let Some(fade) = props.fade_out {
            item.fade_out = Some(fade.min(item.duration));
        }
        if let Some(enhance) = props.voice_enhance {
            item.voice_enhance = enhance;
        }
        if let Some(fit) = props.fit {
            item.fit = Some(fit);
        }
        if let Some(focus) = props.focus {
            item.focus = Some(focus);
        }
        if let Some(crop) = props.crop {
            item.crop = crop;
        }
        if let Some(flip) = props.flip_x {
            item.flip_x = flip;
        }
        if let Some(flip) = props.flip_y {
            item.flip_y = flip;
        }
        if let Some(radius) = props.radius {
            item.radius = Some(radius);
        }
        if let Some(blend) = props.blend {
            item.blend = blend;
        }
        if let Some(adjust) = props.adjust {
            item.adjust = adjust;
        }
        if let Some(anim) = props.anim_in {
            item.anim_in = anim;
        }
        if let Some(anim) = props.anim_out {
            item.anim_out = anim;
        }
        if let Some(hidden) = props.hidden {
            item.hidden = hidden;
        }
        if let Some(group) = props.group.take() {
            item.group_id = group;
        }
        if let Some(asset) = props.media.take() {
            item.asset_id = Some(asset.id.clone());
            item.name = asset.name.clone();
            item.kind = match asset.kind {
                MediaKind::Image => ItemKind::Image,
                MediaKind::Video => ItemKind::Video,
                MediaKind::Audio => ItemKind::Audio,
            };
            // The new file may end before where the old one started.
            let length = asset.duration.unwrap_or(f64::INFINITY);
            if item.kind != ItemKind::Image && item.source_in() >= length - MIN_CLIP_DURATION {
                item.source_in = Some(0.0);
            }
        }
    });
    if fields.contains_key("asset_id") && duration.is_none() {
        // A shorter file shortens the clip.
        let current = clip(editor, id)?.clone();
        let room = source_room(editor, &current, current.source_in(), current.speed());
        if current.duration > room + 1e-6 {
            notes.push(format!("shortened to {} s: the new file ends there", round(room)));
            editor.set_timing(id, None, Some(room), true);
        }
    }
    if let Some(speed) = speed {
        let selection = editor.selection.clone();
        editor.select_many(&[id.to_string()]);
        editor.set_speed(speed, true);
        editor.select_many(&selection);
    }
    if let Some(source_in) = source_in {
        let current = clip(editor, id)?.clone();
        let length = editor.asset(current.asset_id.as_deref()).and_then(|a| a.duration).unwrap_or(f64::INFINITY);
        let most = (length - current.duration * current.speed()).max(0.0);
        if source_in > most + 1e-6 {
            notes.push(format!("source_in limited to {} s: the file ends there", round(most)));
        }
        editor.update_item(id, true, |item| item.source_in = Some(source_in.clamp(0.0, most)));
    }
    if let Some(track) = &move_to {
        editor.begin_change();
        let at = start.unwrap_or(item.start);
        if !editor.move_item(id, at, Some(track)) {
            notes.push(format!("could not move to track {track}: no room at {} s", round(at)));
        }
    }
    if (start.is_some() && move_to.is_none()) || duration.is_some() {
        let current = clip(editor, id)?.clone();
        let room = source_room(editor, &current, current.source_in(), current.speed());
        if duration.is_some_and(|d| d > room + 1e-6) {
            notes.push(format!("duration limited to {} s: the file ends there", round(room)));
        }
        let start = if move_to.is_some() { None } else { start };
        editor.set_timing(id, start, duration.map(|d| d.min(room)), true);
    }
    if let Some(start) = start {
        let actual = clip(editor, id)?.start;
        if (actual - start).abs() > 1e-3 {
            notes.push(format!("starts at {} s: {} s would overlap another clip on its track", round(actual), round(start)));
        }
    }
    if let Some((kind, duration)) = transition {
        editor.set_transition(id, kind, duration, true);
        if kind != TransitionKind::None && clip(editor, id)?.transition.is_none_or(|t| t.kind == TransitionKind::None) {
            notes.push("no transition: no clip ends exactly where this one starts on its track".into());
        }
    }
    if let Some(keyframes) = keyframes {
        editor.update_item(id, true, |item| {
            item.transform = Some(normalize_transform(item.transform.as_ref()));
            item.keyframes = keyframes;
            if !item.keyframes.is_empty() {
                item.keyframes = sorted_keyframes(item);
            }
        });
    }
    // Locking comes last: a locked clip takes no change.
    if lock == Some(true) && !item.locked {
        editor.begin_change();
        if let Some(target) = editor.project.item_mut(id) {
            target.locked = true;
        }
    }
    Ok(())
}

/// Takes back a clip an add tool made when its properties were wrong.
fn discard(editor: &mut Editor, id: &str) {
    editor.select_many(&[id.to_string()]);
    editor.delete_selected(false);
}

fn parse_transition(kind: &str) -> Result<TransitionKind, String> {
    let wanted = kind.trim().to_lowercase().replace([' ', '_'], "-");
    TransitionKind::parse(&wanted).ok_or_else(|| format!("transition \"{kind}\" is not one of: {}", specs::transition_names().join(", ")))
}

fn animation<K: Copy + PartialEq + Default>(
    value: &Value,
    parse: fn(&str) -> Option<K>,
    names: &[&str],
    default_duration: fn(K) -> f64,
    clip_duration: f64,
) -> Result<Option<Animation<K>>, String> {
    let (kind, duration) = match value {
        Value::Null => return Ok(None),
        Value::String(kind) => (args::choice(&Args::from_iter([("kind".to_string(), json!(kind))]), "kind", parse, names)?, None),
        other => {
            let fields = args::object(other)?;
            (args::choice(&fields, "kind", parse, names)?, args::number(&fields, "duration")?)
        }
    };
    let kind = kind.unwrap_or_default();
    if kind == K::default() {
        return Ok(None);
    }
    let duration = duration.unwrap_or_else(|| default_duration(kind)).clamp(0.05, 5.0f64.min(clip_duration.max(0.1)));
    Ok(Some(Animation { kind, duration }))
}

fn text_style(current: &TextStyle, preset: Option<&TextPreset>, style: Option<&Args>, content: Option<&str>) -> Result<TextStyle, String> {
    let mut s = match preset {
        Some(preset) => preset_text_style(current, preset),
        None => current.clone(),
    };
    if let Some(style) = style {
        if let Some(message) = args::unknown_keys(style, STYLE_FIELDS) {
            return Err(format!("style: {message}"));
        }
        let color = |key: &str| -> Result<Option<String>, String> {
            match args::string(style, key)? {
                Some(value) if value.is_empty() && key == "highlight_text_color" => Ok(Some(value)),
                Some(value) => {
                    let value = if value.starts_with('#') { value } else { format!("#{value}") };
                    if is_hex_color(&value) {
                        Ok(Some(value.to_lowercase()))
                    } else {
                        Err(format!("style.{key} must be a hex color like \"#ffcc00\""))
                    }
                }
                None => Ok(None),
            }
        };
        let number = |key: &str| args::number(style, key);
        if let Some(font) = args::string(style, "font")? {
            s.font = font;
        }
        if let Some(v) = number("size")? {
            s.size = v.clamp(8.0, 600.0);
        }
        if let Some(v) = number("weight")? {
            s.weight = v.clamp(100.0, 900.0);
        }
        if let Some(v) = args::boolean(style, "italic")? {
            s.italic = v;
        }
        if let Some(v) = args::boolean(style, "uppercase")? {
            s.uppercase = v;
        }
        if let Some(v) = args::choice(style, "align", TextAlign::parse, &["left", "center", "right"])? {
            s.align = v;
        }
        if let Some(v) = number("letter_spacing")? {
            s.letter_spacing = v.clamp(-20.0, 60.0);
        }
        if let Some(v) = number("line_height")? {
            s.line_height = v.clamp(0.7, 2.5);
        }
        if let Some(v) = number("max_width")? {
            s.max_width = v.clamp(10.0, 100.0);
        }
        if let Some(v) = number("stroke_width")? {
            s.stroke_width = v.clamp(0.0, 30.0);
        }
        if let Some(v) = number("background_opacity")? {
            s.background_opacity = v.clamp(0.0, 1.0);
        }
        if let Some(v) = number("background_padding")? {
            s.background_padding = v.clamp(0.0, 80.0);
        }
        if let Some(v) = number("background_radius")? {
            s.background_radius = v.clamp(0.0, 80.0);
        }
        if let Some(v) = number("shadow_opacity")? {
            s.shadow_opacity = v.clamp(0.0, 1.0);
        }
        if let Some(v) = number("shadow_blur")? {
            s.shadow_blur = v.clamp(0.0, 80.0);
        }
        if let Some(v) = number("shadow_distance")? {
            s.shadow_distance = v.clamp(0.0, 60.0);
        }
        if let Some(v) = args::choice(style, "highlight_mode", HighlightMode::parse, &["none", "color", "box"])? {
            s.highlight_mode = v;
            if v != HighlightMode::None && s.highlight_index < 0 {
                s.highlight_index = 0;
            }
        }
        if let Some(v) = number("highlight_index")? {
            s.highlight_index = v.round().max(-1.0) as i64;
        }
        for (key, slot) in [
            ("color", &mut s.color),
            ("stroke_color", &mut s.stroke_color),
            ("background_color", &mut s.background_color),
            ("shadow_color", &mut s.shadow_color),
            ("highlight_color", &mut s.highlight_color),
            ("highlight_text_color", &mut s.highlight_text_color),
        ] {
            if let Some(value) = color(key)? {
                *slot = value;
            }
        }
    }
    if let Some(content) = content {
        s.content = content.replace("\\n", "\n");
    }
    Ok(s)
}

fn shape_style(current: &ShapeStyle, fields: &Args) -> Result<ShapeStyle, String> {
    if let Some(message) = args::unknown_keys(fields, &["kind", "width", "height", "color", "blur"]) {
        return Err(format!("shape: {message}"));
    }
    let mut shape = current.clone();
    if let Some(kind) = args::choice(fields, "kind", ShapeKind::parse, &["rect", "circle", "line", "blur"])? {
        shape.kind = kind;
    }
    if let Some(v) = args::number(fields, "width")? {
        shape.width = v.clamp(0.2, 400.0);
    }
    if let Some(v) = args::number(fields, "height")? {
        shape.height = v.clamp(0.2, 400.0);
    }
    if let Some(v) = args::number(fields, "blur")? {
        shape.blur = v.clamp(2.0, 120.0);
    }
    if let Some(color) = args::string(fields, "color")? {
        let color = if color.starts_with('#') { color } else { format!("#{color}") };
        if !is_hex_color(&color) {
            return Err("shape.color must be a hex color like \"#ffffff\"".into());
        }
        shape.color = color.to_lowercase();
    }
    Ok(shape)
}

/// Crop from % per edge (left/right/top/bottom) or a 0..1 rectangle (x/y/w/h).
fn crop(fields: &Args) -> Result<Option<Crop>, String> {
    if let Some(message) = args::unknown_keys(fields, &["left", "right", "top", "bottom", "x", "y", "w", "h"]) {
        return Err(format!("crop: {message}"));
    }
    let crop = if ["x", "y", "w", "h"].iter().any(|k| fields.contains_key(*k)) {
        let x = args::number(fields, "x")?.unwrap_or(0.0).clamp(0.0, 0.95);
        let y = args::number(fields, "y")?.unwrap_or(0.0).clamp(0.0, 0.95);
        Crop {
            x,
            y,
            w: args::number(fields, "w")?.unwrap_or(1.0 - x).clamp(0.05, 1.0 - x),
            h: args::number(fields, "h")?.unwrap_or(1.0 - y).clamp(0.05, 1.0 - y),
        }
    } else {
        let edge = |key: &str| -> Result<f64, String> { Ok(args::number(fields, key)?.unwrap_or(0.0).clamp(0.0, 90.0)) };
        let (left, right, top, bottom) = (edge("left")?, edge("right")?, edge("top")?, edge("bottom")?);
        let (left, right) = (left.min(95.0 - right) / 100.0, right.min(95.0 - left) / 100.0);
        let (top, bottom) = (top.min(95.0 - bottom) / 100.0, bottom.min(95.0 - top) / 100.0);
        Crop { x: left, y: top, w: 1.0 - left - right, h: 1.0 - top - bottom }
    };
    Ok((crop != Crop::default()).then_some(crop))
}

fn adjust(mut current: Adjust, fields: &Args) -> Result<Option<Adjust>, String> {
    if let Some(message) = args::unknown_keys(fields, &["exposure", "brightness", "contrast", "saturation", "temperature", "tint", "hue", "vignette"]) {
        return Err(format!("adjust: {message}"));
    }
    for (key, slot) in [
        ("exposure", &mut current.exposure),
        ("brightness", &mut current.brightness),
        ("contrast", &mut current.contrast),
        ("saturation", &mut current.saturation),
        ("temperature", &mut current.temperature),
        ("tint", &mut current.tint),
        ("hue", &mut current.hue),
    ] {
        if let Some(value) = args::number(fields, key)? {
            *slot = value.clamp(-1.0, 1.0);
        }
    }
    if let Some(value) = args::number(fields, "vignette")? {
        current.vignette = value.clamp(0.0, 1.0);
    }
    Ok((!current.is_neutral()).then_some(current))
}

fn keyframes(fields: &Args, item: &TimelineItem) -> Result<Vec<Keyframe>, String> {
    let base = normalize_transform(item.transform.as_ref());
    let easings = specs::names_of(Easing::ALL);
    let easings: Vec<&str> = easings.iter().map(String::as_str).collect();
    let mut list: Vec<Keyframe> = Vec::new();
    for fields in args::objects(fields, "keyframes")? {
        if let Some(message) = args::unknown_keys(&fields, &["time", "at", "x", "y", "scale", "rotation", "opacity", "easing", "bezier"]) {
            return Err(format!("keyframes: {message}"));
        }
        // Past the clip's end it lands on the end (its length may change in the same update).
        let time = match (args::number(&fields, "time")?, args::number(&fields, "at")?) {
            (Some(time), _) => time,
            (None, Some(at)) => at - item.start,
            (None, None) => return Err("each keyframe needs `time` (seconds from the clip's start) or `at` (a timeline second)".into()),
        }
        .max(0.0);
        let transform = Transform {
            x: args::number(&fields, "x")?.unwrap_or(base.x),
            y: args::number(&fields, "y")?.unwrap_or(base.y),
            scale: args::number(&fields, "scale")?.unwrap_or(base.scale).clamp(MIN_SCALE, MAX_SCALE),
            rotation: args::number(&fields, "rotation")?.unwrap_or(base.rotation),
            opacity: args::number(&fields, "opacity")?.unwrap_or(base.opacity).clamp(0.0, 1.0),
        };
        let curve = args::numbers(&fields, "bezier")?;
        let bezier = match curve.as_slice() {
            [] => None,
            [x1, y1, x2, y2] => sanitize_bezier([*x1, *y1, *x2, *y2]),
            _ => return Err("keyframes: `bezier` is four numbers, x1 y1 x2 y2 (like CSS cubic-bezier)".into()),
        };
        let easing = match (args::choice(&fields, "easing", Easing::parse, &easings)?, bezier) {
            (_, Some(_)) => Easing::Custom,
            (Some(Easing::Custom), None) => return Err("keyframes: easing custom needs `bezier` [x1, y1, x2, y2]".into()),
            (Some(easing), None) => easing,
            (None, None) => Easing::EaseInOut,
        };
        list.retain(|k| (k.time - time).abs() > 1e-3);
        list.push(Keyframe { id: uid("kf"), time, transform, easing, bezier });
    }
    list.sort_by(|a, b| a.time.total_cmp(&b.time));
    Ok(list)
}

// ------------------------------------------------------------ structure

fn delete_items(editor: &mut Editor, a: &Args) -> Outcome {
    let ids = args::strings(a, "item_ids")?;
    if ids.is_empty() {
        return Err("`item_ids` is empty.".into());
    }
    let mut skipped = Vec::new();
    let targets: Vec<String> = ids
        .iter()
        .filter(|id| match editable(editor, id) {
            Ok(()) => true,
            Err(error) => {
                skipped.push(error);
                false
            }
        })
        .cloned()
        .collect();
    let before = editor.project.items().count();
    if !targets.is_empty() {
        editor.select_many(&targets);
        editor.delete_selected(args::boolean(a, "ripple")?.unwrap_or(false));
    }
    let removed = before - editor.project.items().count();
    let mut result = json!({ "removed": removed, "duration": round(editor.playback_end()) });
    if !skipped.is_empty() {
        result["skipped"] = json!(skipped);
    }
    Ok(summary(result, format!("Deleted {}", plural(removed, "clip", "clips"))))
}

fn split_items(editor: &mut Editor, a: &Args) -> Outcome {
    let time = args::required_number(a, "time")?;
    let ids = args::strings(a, "item_ids")?;
    for id in &ids {
        editable(editor, id)?;
    }
    if ids.is_empty() {
        editor.select(None, false);
    } else {
        editor.select_many(&ids);
    }
    let before = editor.project.items().count();
    editor.split_at(time);
    let created = editor.project.items().count() - before;
    if created == 0 {
        return Err(format!(
            "Nothing was split at {} s: no unlocked clip runs across that second (cuts within 0.1 s of a clip's edge are not made).",
            round(time)
        ));
    }
    let parts: Vec<Value> = editor.selection.iter().map(|id| report(editor, id, vec![])).collect();
    Ok(summary(json!({ "new_parts": parts }), format!("Split {}", plural(created, "clip", "clips"))))
}

fn trim_items(editor: &mut Editor, a: &Args) -> Outcome {
    let trims = args::objects(a, "trims")?;
    let mut results = Vec::new();
    for fields in trims {
        let trimmed = (|| -> Result<String, String> {
            let id = args::required_string(&fields, "id")?;
            editable(editor, &id)?;
            let (start, end) = (args::number(&fields, "start")?, args::number(&fields, "end")?);
            if start.is_none() && end.is_none() {
                return Err("give `start` and/or `end`".into());
            }
            if let (Some(start), Some(end)) = (start, end) {
                if end - start < MIN_CLIP_DURATION {
                    return Err("`end` must be after `start`".into());
                }
            }
            editor.begin_change();
            if let Some(start) = start {
                let origin = TrimOrigin::of(clip(editor, &id)?);
                editor.trim_item(&id, Edge::Start, start, origin);
            }
            if let Some(end) = end {
                let origin = TrimOrigin::of(clip(editor, &id)?);
                editor.trim_item(&id, Edge::End, end, origin);
            }
            Ok(id)
        })();
        match trimmed {
            Ok(id) => {
                let item = clip(editor, &id)?.clone();
                let mut notes = Vec::new();
                if let Some(start) = args::number(&fields, "start")? {
                    if (item.start - start).abs() > 1e-3 {
                        notes.push(format!("the start edge stopped at {} s (start of the file or a neighbouring clip)", round(item.start)));
                    }
                }
                if let Some(end) = args::number(&fields, "end")? {
                    if (item.end() - end).abs() > 1e-3 {
                        notes.push(format!("the end edge stopped at {} s (end of the file or a neighbouring clip)", round(item.end())));
                    }
                }
                results.push(report(editor, &id, notes));
            }
            Err(error) => results.push(json!({ "id": fields.get("id"), "error": error })),
        }
    }
    let done = results.iter().filter(|r| r.get("error").is_none()).count();
    Ok(summary(json!({ "items": results }), format!("Trimmed {}", plural(done, "clip", "clips"))))
}

fn remove_time_ranges(editor: &mut Editor, a: &Args) -> Outcome {
    let mut ranges: Vec<(f64, f64)> = Vec::new();
    for fields in args::objects(a, "ranges")? {
        let (start, end) = (args::required_number(&fields, "start")?.max(0.0), args::required_number(&fields, "end")?);
        if end > start {
            ranges.push((start, end));
        }
    }
    if ranges.is_empty() {
        return Err("`ranges` needs at least one {start, end} with end after start.".into());
    }
    let tracks = args::strings(a, "track_ids")?;
    for id in &tracks {
        if !editor.project.tracks.iter().any(|t| t.id == *id) {
            return Err(format!("There is no track with id \"{id}\"."));
        }
    }
    // Merged, then cut from the last one back so earlier times stay valid.
    ranges.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (start, end) in ranges {
        match merged.last_mut() {
            Some(last) if start <= last.1 + 1e-6 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    let before = editor.playback_end();
    let mut cut = 0.0;
    let mut changed = 0;
    for &(start, end) in merged.iter().rev() {
        let count = editor.remove_range(start, end, (!tracks.is_empty()).then_some(tracks.as_slice()));
        if count > 0 {
            changed += count;
            cut += end - start;
        }
    }
    if changed == 0 {
        return Err("Nothing was removed: no unlocked clip lies in those ranges.".into());
    }
    let after = editor.playback_end();
    Ok(summary(
        json!({ "ranges": merged.len(), "removed_seconds": round(cut), "duration_before": round(before), "duration": round(after) }),
        format!("Cut out {} ({} s); the video is now {} s", plural(merged.len(), "range", "ranges"), round(cut), round(after)),
    ))
}

fn duplicate_items(editor: &mut Editor, a: &Args) -> Outcome {
    let ids = args::strings(a, "item_ids")?;
    if ids.is_empty() {
        return Err("`item_ids` is empty.".into());
    }
    for id in &ids {
        editable(editor, id)?;
    }
    editor.select_many(&ids);
    editor.duplicate_selected();
    let created = editor.selection.clone();
    if let Some(start) = args::number(a, "start")? {
        let first = created.iter().filter_map(|id| editor.item(id)).map(|i| i.start).fold(f64::INFINITY, f64::min);
        let moves: Vec<(String, f64)> = created.iter().filter_map(|id| editor.item(id).map(|i| (id.clone(), (i.start - first + start).max(0.0)))).collect();
        editor.move_items(&moves);
    }
    let copies: Vec<Value> = created.iter().map(|id| report(editor, id, vec![])).collect();
    Ok(summary(json!({ "copies": copies }), format!("Duplicated {}", plural(created.len(), "clip", "clips"))))
}

fn detach_audio(editor: &mut Editor, a: &Args) -> Outcome {
    let ids = args::strings(a, "item_ids")?;
    for id in &ids {
        editable(editor, id)?;
    }
    editor.select_many(&ids);
    let before = editor.project.items().count();
    editor.detach_audio_selected();
    if editor.project.items().count() == before {
        return Err("None of these clips is a video with sound that is not muted.".into());
    }
    let audio: Vec<Value> = editor.selection.iter().map(|id| report(editor, id, vec![])).collect();
    Ok(summary(json!({ "audio_clips": audio }), format!("Detached the sound of {}", plural(audio.len(), "video", "videos"))))
}

// ------------------------------------------------------------ effects and transitions

fn effect_kind(fields: &Args) -> Result<EffectKind, String> {
    args::choice(fields, "kind", EffectKind::parse, &specs::effect_names())?.filter(|k| *k != EffectKind::None).ok_or_else(|| "`kind` is required".to_string())
}

fn beat_every(editor: &Editor, fields: &Args) -> Result<Option<u32>, String> {
    let Some(every) = args::number(fields, "on_beat")? else { return Ok(None) };
    if every < 1.0 {
        return Ok(None);
    }
    if editor.project.beats.is_empty() {
        return Err("The project has no beats yet: call find_beats first (it needs music on the timeline).".into());
    }
    Ok(Some(every.round().clamp(1.0, 16.0) as u32))
}

fn add_effect(editor: &mut Editor, a: &Args) -> Outcome {
    let ids = args::strings(a, "item_ids")?;
    if ids.is_empty() {
        return Err("`item_ids` is empty.".into());
    }
    let kind = effect_kind(a)?;
    let amount = args::number(a, "amount")?.map(|v| v.clamp(0.0, 1.0));
    let (from, to) = (args::number(a, "from")?, args::number(a, "to")?);
    let every = beat_every(editor, a)?;
    let mut results = Vec::new();
    let mut added = 0;
    for id in &ids {
        let outcome = (|| -> Result<Vec<String>, String> {
            editable(editor, id)?;
            let item = clip(editor, id)?.clone();
            if !can_have_effects(&item) {
                return Err(format!("{id} is a {} clip; effects go on videos, photos, texts and stickers.", item.kind));
            }
            let range = effect_range(&item, from, to)?;
            let had = item.effects.len();
            let pairs = editor.add_effect(std::slice::from_ref(id), kind, range);
            let Some((_, effect_id)) = pairs.first().cloned() else {
                return Err(format!("{id} already has {MAX_EFFECTS} effects; set_effects can replace them."));
            };
            if range.is_none() && from.is_none() && to.is_none() {
                // add_effect keeps an existing effect's range; a new request for the whole clip resets it.
                editor.update_item(id, true, |item| {
                    if let Some(effect) = item.effects.iter_mut().find(|e| e.id == effect_id) {
                        effect.range = None;
                    }
                });
            }
            if let Some(amount) = amount {
                editor.set_effect_amount(id, &effect_id, amount, true);
            }
            if every.is_some() {
                editor.set_effect_on_beat(std::slice::from_ref(id), kind, every, None);
            }
            let mut notes = Vec::new();
            if clip(editor, id)?.effects.len() == had {
                notes.push("the clip had this effect already; it was updated".to_string());
            }
            Ok(notes)
        })();
        match outcome {
            Ok(notes) => {
                added += 1;
                results.push(report(editor, id, notes));
            }
            Err(error) => results.push(json!({ "id": id, "error": error })),
        }
    }
    Ok(summary(json!({ "items": results }), format!("Added {kind} to {}", plural(added, "clip", "clips"))))
}

fn set_effects(editor: &mut Editor, a: &Args) -> Outcome {
    let id = args::required_string(a, "item_id")?;
    editable(editor, &id)?;
    let item = clip(editor, &id)?.clone();
    if !can_have_effects(&item) {
        return Err(format!("{id} is a {} clip; effects go on videos, photos, texts and stickers.", item.kind));
    }
    let list = args::objects(a, "effects")?;
    if list.len() > MAX_EFFECTS {
        return Err(format!("A clip can have at most {MAX_EFFECTS} effects."));
    }
    let mut effects = Vec::new();
    for fields in &list {
        if let Some(message) = args::unknown_keys(fields, &["kind", "amount", "from", "to", "on_beat"]) {
            return Err(format!("effects: {message}"));
        }
        effects.push(ClipEffect {
            id: uid("fx"),
            kind: effect_kind(fields)?,
            amount: args::number(fields, "amount")?.map_or(DEFAULT_EFFECT_AMOUNT, |v| v.clamp(0.0, 1.0)),
            range: effect_range(&item, args::number(fields, "from")?, args::number(fields, "to")?)?,
            on_beat: beat_every(editor, fields)?,
            extra: Default::default(),
        });
    }
    let count = effects.len();
    editor.update_item(&id, true, |item| item.effects = effects);
    Ok(summary(report(editor, &id, vec![]), format!("Set {} on the clip", plural(count, "effect", "effects"))))
}

fn set_transitions(editor: &mut Editor, a: &Args) -> Outcome {
    let list = args::objects(a, "transitions")?;
    if list.is_empty() {
        let track = args::string(a, "track_id")?.ok_or("Give `transitions`, or `track_id` with `kind`.")?;
        let kind = parse_transition(&args::required_string(a, "kind")?)?;
        if !editor.project.tracks.iter().any(|t| t.id == track) {
            return Err(format!("There is no track with id \"{track}\"."));
        }
        let duration = args::number(a, "duration")?.unwrap_or(0.5).clamp(0.05, 5.0);
        editor.set_transition_for_track(&track, kind, duration);
        let cuts = editor
            .project
            .tracks
            .iter()
            .find(|t| t.id == track)
            .map_or(0, |t| t.items.iter().filter(|i| i.transition.is_some_and(|t| t.kind != TransitionKind::None)).count());
        return Ok(summary(json!({ "track_id": track, "transitions": cuts }), format!("{} on the track", plural(cuts, "transition", "transitions"))));
    }
    let mut results = Vec::new();
    let mut set = 0;
    for fields in list {
        let outcome = (|| -> Result<(String, Vec<String>), String> {
            let id = args::required_string(&fields, "item_id")?;
            editable(editor, &id)?;
            let kind = parse_transition(&args::required_string(&fields, "kind")?)?;
            let duration = args::number(&fields, "duration")?.map(|d| d.clamp(0.05, 5.0));
            editor.set_transition(&id, kind, duration, true);
            let mut notes = Vec::new();
            if kind != TransitionKind::None && clip(editor, &id)?.transition.is_none_or(|t| t.kind == TransitionKind::None) {
                notes.push("no transition: no clip ends exactly where this one starts on its track".to_string());
            }
            Ok((id, notes))
        })();
        match outcome {
            Ok((id, notes)) => {
                if notes.is_empty() {
                    set += 1;
                }
                results.push(report(editor, &id, notes));
            }
            Err(error) => results.push(json!({ "item_id": fields.get("item_id"), "error": error })),
        }
    }
    Ok(summary(json!({ "items": results }), format!("Set {}", plural(set, "transition", "transitions"))))
}

/// The look of captions: a preset id, or the default caption style.
pub fn caption_look(preset: Option<&str>) -> Result<TextStyle, String> {
    match preset.filter(|p| !p.is_empty() && *p != "caption-default") {
        Some(id) => text_preset(id)
            .map(|p| TextStyle { max_width: 80.0, ..p.text_style() })
            .ok_or_else(|| format!("preset \"{id}\" is not one of: caption-default, {}", specs::preset_names().join(", "))),
        None => Ok(caption_style()),
    }
}

fn set_captions(editor: &mut Editor, a: &Args) -> Outcome {
    let mut cues = Vec::new();
    for fields in args::objects(a, "captions")? {
        let text = args::required_string(&fields, "text")?;
        let start = args::required_number(&fields, "start")?.max(0.0);
        let end = match args::number(&fields, "end")? {
            Some(end) => end,
            None => start + args::number(&fields, "duration")?.ok_or("each caption needs `end`")?,
        };
        if end - start < 0.05 {
            return Err(format!("caption \"{text}\" ends before it starts"));
        }
        cues.push(CaptionCue { text: text.trim().to_string(), start, duration: end - start, highlight_index: None });
    }
    if cues.is_empty() {
        return Err("`captions` is empty.".into());
    }
    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    let style = caption_look(args::string(a, "preset")?.as_deref())?;
    let source = match args::string(a, "replace_group")? {
        Some(group) => group.strip_prefix("captions-").map(String::from).ok_or("replace_group must be a caption group id (\"captions-…\")")?,
        None => uid("ai"),
    };
    let cues = if style.highlight_mode != HighlightMode::None { highlight_cues(&cues, &[]) } else { cues };
    let y = args::number(a, "y")?.unwrap_or(72.0).clamp(0.0, 100.0);
    let count = editor.apply_captions(&cues, CaptionOptions { source_id: source.clone(), offset: 0.0, speed: 1.0, style, y });
    Ok(summary(json!({ "group": format!("captions-{source}"), "clips": count }), format!("Added {}", plural(count, "caption clip", "caption clips"))))
}

// ------------------------------------------------------------ tracks and project

fn edit_tracks(editor: &mut Editor, a: &Args) -> Outcome {
    let actions = args::objects(a, "actions")?;
    if actions.is_empty() {
        return Err("`actions` is empty.".into());
    }
    let mut notes = Vec::new();
    for fields in actions {
        let action = args::required_string(&fields, "action")?.to_lowercase();
        if action == "add" {
            let kind = args::choice(&fields, "kind", TrackKind::parse, &["video", "text", "audio"])?.unwrap_or(TrackKind::Video);
            let before: Vec<String> = editor.project.tracks.iter().map(|t| t.id.clone()).collect();
            editor.add_track(kind);
            let created = editor.project.tracks.iter().find(|t| !before.contains(&t.id)).map(|t| t.id.clone()).unwrap_or_default();
            if let Some(name) = args::string(&fields, "name")? {
                editor.update_track(&created, TrackPatch { name: Some(name), ..Default::default() });
            }
            notes.push(format!("added {kind} track {created}"));
            continue;
        }
        let id = args::required_string(&fields, "track_id")?;
        let Some(index) = editor.project.tracks.iter().position(|t| t.id == id) else {
            return Err(format!("There is no track with id \"{id}\"."));
        };
        match action.as_str() {
            "rename" => editor.update_track(&id, TrackPatch { name: Some(args::required_string(&fields, "name")?), ..Default::default() }),
            "move" => editor.move_track(&id, args::required_number(&fields, "index")?.max(0.0) as usize),
            "delete" => {
                if editor.project.tracks[index].locked {
                    return Err(format!("Track {id} is locked; unlock it first."));
                }
                editor.remove_track(&id);
            }
            "lock" | "unlock" => editor.update_track(&id, TrackPatch { locked: Some(action == "lock"), ..Default::default() }),
            "mute" | "unmute" => editor.update_track(&id, TrackPatch { muted: Some(action == "mute"), ..Default::default() }),
            "hide" | "show" => editor.update_track(&id, TrackPatch { hidden: Some(action == "hide"), ..Default::default() }),
            other => return Err(format!("Unknown action \"{other}\": add, rename, move, delete, lock, unlock, mute, unmute, hide or show.")),
        }
        notes.push(format!("{action} {id}"));
    }
    let tracks: Vec<Value> = editor
        .project
        .tracks
        .iter()
        .enumerate()
        .map(|(layer, t)| json!({ "id": t.id, "kind": t.kind.as_str(), "name": t.name, "layer": layer, "clips": t.items.len(), "locked": t.locked, "muted": t.muted, "hidden": t.hidden }))
        .collect();
    Ok(summary(json!({ "tracks": tracks }), notes.join(", ")))
}

fn set_project(editor: &mut Editor, a: &Args) -> Outcome {
    let known = ["name", "aspect", "width", "height", "background", "format", "codec", "quality", "resolution", "fps", "loudness", "hardware", "beats"];
    if let Some(message) = args::unknown_keys(a, &known) {
        return Err(message);
    }
    let mut changes = Vec::new();
    if let Some(name) = args::string(a, "name")? {
        editor.rename(&name);
        changes.push("name");
    }
    let aspect = args::string(a, "aspect")?;
    if let Some(aspect) = &aspect {
        let preset = ASPECT_PRESETS
            .iter()
            .find(|p| p.id == aspect.trim())
            .ok_or_else(|| format!("aspect \"{aspect}\" is not one of: {}", ASPECT_PRESETS.iter().map(|p| p.id).collect::<Vec<_>>().join(", ")))?;
        editor.set_canvas(preset.width, preset.height);
        changes.push("canvas");
    } else if let (Some(width), Some(height)) = (args::number(a, "width")?, args::number(a, "height")?) {
        let even = |v: f64| ((v.clamp(64.0, 8192.0) / 2.0).round() * 2.0) as u32;
        editor.set_canvas(even(width), even(height));
        changes.push("canvas");
    }
    if let Some(color) = args::string(a, "background")? {
        let color = if color.starts_with('#') { color } else { format!("#{color}") };
        if !is_hex_color(&color) {
            return Err("`background` must be a hex color like \"#000000\".".into());
        }
        editor.set_background(&color.to_lowercase(), true);
        changes.push("background");
    }
    if a.contains_key("beats") {
        let beats = args::numbers(a, "beats")?;
        let options = editor.project.beat_options;
        editor.set_beats(Some(beats), options, true);
        changes.push("beats");
    }
    let format = args::choice(a, "format", ExportFormat::parse, &["mp4", "mov", "webm"])?;
    let codec = args::choice(a, "codec", ExportCodec::parse, &["h264", "h265", "vp9"])?;
    let quality = args::choice(a, "quality", ExportQuality::parse, &["draft", "standard", "high", "ultra"])?;
    let resolution = args::number(a, "resolution")?.map(|r| r.round() as u32);
    if resolution.is_some_and(|r| ![480, 720, 1080, 1440, 2160].contains(&r)) {
        return Err("`resolution` must be 480, 720, 1080, 1440 or 2160.".into());
    }
    let fps = args::number(a, "fps")?.map(|f| f.round() as u32);
    if fps.is_some_and(|f| !FPS_CHOICES.contains(&f)) {
        return Err("`fps` must be 24, 25, 30, 50 or 60.".into());
    }
    let loudness = args::number(a, "loudness")?;
    let hardware = args::boolean(a, "hardware")?;
    if format.is_some() || codec.is_some() || quality.is_some() || resolution.is_some() || fps.is_some() || loudness.is_some() || hardware.is_some() {
        editor.begin_change();
        editor.update_export(|x| {
            if let Some(format) = format {
                x.format = format;
                if format == ExportFormat::Webm {
                    x.codec = ExportCodec::Vp9;
                } else if x.codec == ExportCodec::Vp9 {
                    x.codec = ExportCodec::H264;
                }
            }
            if let Some(codec) = codec {
                x.codec = match (x.format, codec) {
                    (ExportFormat::Webm, _) => ExportCodec::Vp9,
                    (_, ExportCodec::Vp9) => ExportCodec::H264,
                    (_, codec) => codec,
                };
            }
            if let Some(quality) = quality {
                x.quality = quality;
            }
            if let Some(resolution) = resolution {
                x.resolution = resolution;
            }
            if let Some(fps) = fps {
                x.fps = fps;
            }
            if let Some(loudness) = loudness {
                x.loudness = (loudness < 0.0).then_some(loudness.clamp(-40.0, -5.0));
            }
            if let Some(hardware) = hardware {
                x.hardware = hardware;
            }
        });
        changes.push("export settings");
    }
    if changes.is_empty() {
        return Err("Nothing to change: give name, aspect, width and height, background or export settings.".into());
    }
    let project = describe::project(editor, &describe::Window { track: Some(String::new()), ..Default::default() });
    Ok(summary(json!({ "project": project["project"], "export": project["export"] }), format!("Changed the {}", changes.join(", "))))
}

fn history(editor: &mut Editor, a: &Args) -> Outcome {
    let action = args::required_string(a, "action")?.to_lowercase();
    let steps = args::number(a, "steps")?.unwrap_or(1.0).clamp(1.0, 50.0) as usize;
    let mut done = 0;
    for _ in 0..steps {
        let moved = match action.as_str() {
            "undo" => editor.undo(),
            "redo" => editor.redo(),
            other => return Err(format!("`action` must be undo or redo, not {other}")),
        };
        if !moved {
            break;
        }
        done += 1;
    }
    if done == 0 {
        return Err(format!("There is nothing to {action}."));
    }
    Ok(summary(
        json!({ "steps": done, "duration": round(editor.playback_end()) }),
        format!("{} {}", if action == "undo" { "Undid" } else { "Redid" }, plural(done, "step", "steps")),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vf_core::presets::create_project;

    fn editor_with_video() -> (Editor, String) {
        let mut project = create_project("Test", 1080, 1920, 30);
        project.assets.push(MediaAsset {
            id: "talk".into(),
            kind: MediaKind::Video,
            name: "talk.mp4".into(),
            path: "talk.mp4".into(),
            duration: Some(20.0),
            width: Some(1080.0),
            height: Some(1920.0),
            has_audio: Some(true),
            ..Default::default()
        });
        let mut editor = Editor::new(project);
        let result = call(&mut editor, "add_clips", json!({ "clips": [{ "asset_id": "talk" }] }));
        let id = result["clips"][0]["id"].as_str().unwrap().to_string();
        (editor, id)
    }

    fn call(editor: &mut Editor, name: &str, args: Value) -> Value {
        editor.begin_batch();
        let result = run(editor, name, &args).expect("a tool").unwrap_or_else(|error| panic!("{name}: {error}"));
        editor.end_batch();
        result
    }

    fn fails(editor: &mut Editor, name: &str, args: Value) -> String {
        run(editor, name, &args).expect("a tool").expect_err("an error")
    }

    /// The error update_items reports for one clip.
    fn fails_in(editor: &mut Editor, id: &str, mut fields: Value) -> String {
        fields["id"] = json!(id);
        let result = call(editor, "update_items", json!({ "updates": [fields] }));
        result["items"][0]["error"].as_str().expect("an error").to_string()
    }

    #[test]
    fn texts_are_added_styled_and_placed_in_one_undo_step() {
        let (mut editor, _) = editor_with_video();
        let result = call(
            &mut editor,
            "add_texts",
            json!({ "texts": [
                { "text": "WAIT FOR IT", "start": 0, "duration": 2, "y": 25, "preset": "hl-yellow", "style": { "size": 110, "strokeColor": "000000" } },
                { "text": "Part 2", "start": 5, "anim_in": "pop", "x": "30" },
            ]}),
        );
        assert_eq!(result["summary"], "Added 2 texts");
        let first = editor.item(result["texts"][0]["id"].as_str().unwrap()).unwrap().clone();
        assert_eq!(first.transform.unwrap().y, 25.0);
        let style = first.text_style();
        assert_eq!(style.size, 110.0);
        assert_eq!(style.stroke_color, "#000000");
        assert_eq!(style.content, "WAIT FOR IT");
        let second = editor.item(result["texts"][1]["id"].as_str().unwrap()).unwrap().clone();
        assert_eq!(second.anim_in.unwrap().kind, AnimationInKind::Pop);
        assert_eq!(second.transform.unwrap().x, 30.0);
        editor.undo();
        assert_eq!(editor.project.items().filter(|i| i.kind == ItemKind::Text).count(), 0, "one undo removes both");
    }

    #[test]
    fn updates_validate_and_explain() {
        let (mut editor, video) = editor_with_video();
        let result = call(
            &mut editor,
            "update_items",
            json!({ "updates": [{ "id": video, "colour": "red" }, { "id": "nope", "x": 1 }, { "id": video, "text": "hi" }] }),
        );
        assert!(result["items"][0]["error"].as_str().unwrap().contains("Unknown field(s): colour"));
        assert!(result["items"][1]["error"].as_str().unwrap().contains("no clip"));
        assert!(result["items"][2]["error"].as_str().unwrap().contains("text clips"));
        let result = call(
            &mut editor,
            "update_items",
            json!({ "updates": [{ "id": video, "volume": 0.5, "fadeOut": 1, "scale": 1.2, "adjust": { "saturation": 0.3 }, "duration": 50, "voice_enhance": 0.7 }] }),
        );
        let item = editor.item(&video).unwrap();
        assert_eq!(item.volume, Some(0.5));
        assert_eq!(item.voice_enhance, Some(0.7));
        assert_eq!(item.transform.unwrap().scale, 1.2);
        assert_eq!(item.adjust.unwrap().saturation, 0.3);
        assert!((item.duration - 20.0).abs() < 1e-9, "a video can't run past its file");
        assert!(result["items"][0]["notes"][0].as_str().unwrap().contains("limited"));
    }

    #[test]
    fn keyframes_and_effects_go_on_by_timeline_time() {
        let (mut editor, video) = editor_with_video();
        call(
            &mut editor,
            "update_items",
            json!({ "updates": [{ "id": video, "keyframes": [{ "time": 0, "scale": 1 }, { "time": 4, "scale": 1.1, "easing": "ease out" }] }] }),
        );
        let item = editor.item(&video).unwrap();
        assert_eq!(item.keyframes.len(), 2);
        assert_eq!(item.keyframes[1].easing, Easing::EaseOut);
        // By timeline time, with a curve of its own; what a keyframe leaves out stays where the clip is.
        call(
            &mut editor,
            "update_items",
            json!({ "updates": [{ "id": video, "keyframes": [{ "at": 1, "x": 30 }, { "at": 2.5, "x": 70, "rotation": 8, "bezier": [0.3, 1.6, 0.6, 1] }] }] }),
        );
        let item = editor.item(&video).unwrap();
        assert_eq!(item.keyframes.iter().map(|k| k.time).collect::<Vec<_>>(), [1.0, 2.5]);
        assert_eq!(item.keyframes[1].easing, Easing::Custom);
        assert_eq!(item.keyframes[1].bezier, Some([0.3, 1.6, 0.6, 1.0]));
        assert_eq!(item.keyframes[0].transform.y, 50.0);
        let listed = describe::item(item);
        assert_eq!(listed["keyframes"][1]["rotation"], 8.0);
        assert!(fails_in(&mut editor, &video, json!({ "keyframes": [{ "x": 1 }] })).contains("`at`"));
        assert!(fails_in(&mut editor, &video, json!({ "keyframes": [{ "time": 0, "easing": "custom" }] })).contains("bezier"));
        call(&mut editor, "update_items", json!({ "updates": [{ "id": video, "keyframes": [] }] }));
        assert!(editor.item(&video).unwrap().keyframes.is_empty());
        call(&mut editor, "add_effect", json!({ "item_ids": [video], "kind": "punch", "from": 2, "to": 3, "amount": 0.8 }));
        let effect = editor.item(&video).unwrap().effects[0].clone();
        assert_eq!(effect.kind, EffectKind::Punch);
        assert_eq!(effect.amount, 0.8);
        let range = effect.range.unwrap();
        assert!((range.start - 2.0).abs() < 1e-9 && (range.duration - 1.0).abs() < 1e-9);
        assert!(fails(&mut editor, "add_effect", json!({ "item_ids": [video], "kind": "punch", "on_beat": 1 })).contains("find_beats"));
        assert!(fails(&mut editor, "add_effect", json!({ "item_ids": [video], "kind": "sparkles" })).contains("not one of"));
        call(&mut editor, "set_effects", json!({ "item_id": video, "effects": [] }));
        assert!(editor.item(&video).unwrap().effects.is_empty());
    }

    #[test]
    fn ranges_cut_out_and_captions_follow() {
        let (mut editor, _) = editor_with_video();
        call(
            &mut editor,
            "set_captions",
            json!({ "captions": [{ "text": "one", "start": 0, "end": 1 }, { "text": "two", "start": 1, "end": 2 }, { "text": "three", "start": 4, "end": 5 }] }),
        );
        let result = call(&mut editor, "remove_time_ranges", json!({ "ranges": [{ "start": 1, "end": 2 }, { "start": 1.5, "end": 3 }] }));
        assert_eq!(result["ranges"], 1, "overlapping ranges merge");
        assert!((editor.playback_end() - 18.0).abs() < 1e-6, "{}", editor.playback_end());
        let words: Vec<String> = editor.project.items().filter(|i| i.kind == ItemKind::Text).map(|i| i.text_style().content).collect();
        assert_eq!(words, ["one", "three"]);
        let three = editor.project.items().find(|i| i.text_style().content == "three").unwrap();
        assert!((three.start - 2.0).abs() < 1e-6);
    }

    #[test]
    fn tracks_project_and_history() {
        let (mut editor, video) = editor_with_video();
        let result = call(&mut editor, "edit_tracks", json!({ "actions": [{ "action": "add", "kind": "audio", "name": "Music" }] }));
        assert!(result["tracks"].as_array().unwrap().iter().any(|t| t["name"] == "Music"));
        call(&mut editor, "set_project", json!({ "name": "Renamed", "aspect": "1:1", "background": "#112233", "format": "webm", "fps": 60 }));
        assert_eq!(editor.project.name, "Renamed");
        assert_eq!((editor.project.width, editor.project.height), (1080, 1080));
        assert_eq!(editor.project.export.codec, ExportCodec::Vp9);
        assert_eq!(editor.project.fps, 60);
        call(&mut editor, "history", json!({ "action": "undo" }));
        assert_eq!(editor.project.name, "Test", "the whole set_project call was one step");
        let result = call(&mut editor, "split_items", json!({ "time": 5, "item_ids": [video] }));
        assert_eq!(result["new_parts"].as_array().unwrap().len(), 1);
        let result = call(&mut editor, "delete_items", json!({ "item_ids": [video], "ripple": true }));
        assert_eq!(result["removed"], 1);
        assert!(editor.project.items().any(|i| i.kind == ItemKind::Video && i.start == 0.0), "ripple closes the gap");
    }

    #[test]
    fn media_is_replaced_and_clips_grouped() {
        let (mut editor, video) = editor_with_video();
        editor.project.assets.push(MediaAsset {
            id: "short".into(),
            kind: MediaKind::Video,
            name: "short.mp4".into(),
            duration: Some(6.0),
            ..Default::default()
        });
        editor.project.assets.push(MediaAsset {
            id: "song".into(),
            kind: MediaKind::Audio,
            name: "song.mp3".into(),
            duration: Some(60.0),
            ..Default::default()
        });
        let result = call(&mut editor, "update_items", json!({ "updates": [{ "id": video, "asset_id": "short", "group": "intro" }] }));
        let item = editor.item(&video).unwrap();
        assert_eq!(item.asset_id.as_deref(), Some("short"));
        assert!((item.duration - 6.0).abs() < 1e-9, "the shorter file shortens the clip");
        assert_eq!(item.group_id.as_deref(), Some("intro"));
        assert!(result["items"][0]["notes"][0].as_str().unwrap().contains("shortened"));
        assert!(fails_in(&mut editor, &video, json!({ "asset_id": "song" })).contains("can't replace"));
        call(&mut editor, "update_items", json!({ "updates": [{ "id": video, "group": "" }] }));
        assert!(editor.item(&video).unwrap().group_id.is_none());
        call(&mut editor, "set_project", json!({ "beats": [2, 1, 3] }));
        assert_eq!(editor.project.beats, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn locked_clips_say_so() {
        let (mut editor, video) = editor_with_video();
        call(&mut editor, "update_items", json!({ "updates": [{ "id": video, "locked": true }] }));
        let result = call(&mut editor, "update_items", json!({ "updates": [{ "id": video, "x": 10 }] }));
        assert!(result["items"][0]["error"].as_str().unwrap().contains("locked"));
        call(&mut editor, "update_items", json!({ "updates": [{ "id": video, "locked": false, "x": 10 }] }));
        assert_eq!(editor.item(&video).unwrap().transform.unwrap().x, 10.0);
    }
}
