//! Every tool: its name, what it does and its arguments (JSON Schema), in
//! the shapes the chat (OpenAI function calling) and MCP expect. Schemas keep
//! to the plain subset every model provider accepts (no type unions).

use serde_json::{json, Map, Value};
use vf_core::engine::animation::{ANIMATIONS_IN, ANIMATIONS_OUT};
use vf_core::engine::effects::EFFECTS;
use vf_core::engine::transitions::TRANSITIONS;
use vf_core::model::{BlendMode, Easing, ExportCodec, ExportFormat, ExportQuality, Fit, HighlightMode, ShapeKind, TextAlign};
use vf_core::presets::{ASPECT_PRESETS, TEXT_PRESETS};

pub struct ToolSpec {
    pub name: &'static str,
    /// Short label for people ("Add texts").
    pub title: &'static str,
    pub description: String,
    pub schema: Value,
    /// Only looks, changes nothing.
    pub read_only: bool,
    /// Opens or creates projects: AI apps only (the chat belongs to its project).
    pub app: bool,
}

pub fn effect_names() -> Vec<&'static str> {
    EFFECTS.iter().map(|e| e.kind.as_str()).collect()
}

pub fn transition_names() -> Vec<&'static str> {
    std::iter::once("none").chain(TRANSITIONS.iter().map(|t| t.kind.as_str())).collect()
}

pub fn anim_in_names() -> Vec<&'static str> {
    ANIMATIONS_IN.iter().map(|a| a.kind.as_str()).collect()
}

pub fn anim_out_names() -> Vec<&'static str> {
    ANIMATIONS_OUT.iter().map(|a| a.kind.as_str()).collect()
}

pub fn preset_names() -> Vec<&'static str> {
    TEXT_PRESETS.iter().map(|p| p.id).collect()
}

pub fn names_of<T: Copy + std::fmt::Display>(all: &[T]) -> Vec<String> {
    all.iter().map(|v| v.to_string()).collect()
}

fn number(description: &str) -> Value {
    json!({ "type": "number", "description": description })
}

fn string(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

fn boolean(description: &str) -> Value {
    json!({ "type": "boolean", "description": description })
}

fn one_of(values: Vec<impl Into<Value>>, description: &str) -> Value {
    json!({ "type": "string", "enum": values.into_iter().map(Into::into).collect::<Vec<Value>>(), "description": description })
}

fn ids(description: &str) -> Value {
    json!({ "type": "array", "items": { "type": "string" }, "description": description })
}

fn list(items: Value, description: &str) -> Value {
    json!({ "type": "array", "items": items, "description": description })
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": properties, "required": required })
}

fn described(mut schema: Value, description: &str) -> Value {
    schema["description"] = json!(description);
    schema
}

fn transform_fields() -> Map<String, Value> {
    let fields = json!({
        "x": number("Centre x in % of the canvas width (50 = middle)."),
        "y": number("Centre y in % of the canvas height (0 = top, 50 = middle)."),
        "scale": number("1 = the layer's fitted size; 0.05-10."),
        "rotation": number("Degrees, clockwise."),
        "opacity": number("0-1."),
    });
    fields.as_object().cloned().unwrap_or_default()
}

pub fn text_style_schema() -> Value {
    object(
        json!({
            "font": string("Font family, e.g. Inter, Montserrat, Anton, Bebas Neue, Poppins (get_catalog lists all)."),
            "size": number("Font size in project pixels (8-600; 60-100 suits captions on a 1080 px wide canvas)."),
            "weight": number("100-900."),
            "italic": boolean(""),
            "color": string("Hex color."),
            "align": one_of(names_of(TextAlign::ALL), ""),
            "uppercase": boolean(""),
            "letter_spacing": number("-20..60."),
            "line_height": number("0.7-2.5."),
            "max_width": number("Line width before wrapping, % of the canvas (10-100)."),
            "stroke_color": string("Outline color."),
            "stroke_width": number("Outline width 0-30."),
            "background_color": string("Box behind the text."),
            "background_opacity": number("0-1 (0 = no box)."),
            "background_padding": number("0-80."),
            "background_radius": number("0-80."),
            "shadow_color": string(""),
            "shadow_opacity": number("0-1 (0 = no shadow)."),
            "shadow_blur": number("0-80."),
            "shadow_distance": number("0-60."),
            "highlight_mode": one_of(names_of(HighlightMode::ALL), "How the highlighted word stands out: its own color, or a box behind it."),
            "highlight_color": string("Color of the highlighted word (or its box)."),
            "highlight_text_color": string("Text color of the highlighted word in box mode."),
            "highlight_index": number("Which word is highlighted (0-based); -1 = none."),
        }),
        &[],
    )
}

fn animation_schema(kinds: Vec<&'static str>, description: &str) -> Value {
    described(object(json!({ "kind": one_of(kinds, "\"none\" removes it."), "duration": number("Seconds (default suits the kind).") }), &["kind"]), description)
}

/// Properties a clip can have; update_items changes them and the add tools accept them.
pub fn item_fields() -> Map<String, Value> {
    let mut fields = json!({
        "name": string("Label on the timeline."),
        "start": number("Timeline second where the clip starts (its length stays)."),
        "duration": number("Length in seconds (video and audio can't run past the end of their file)."),
        "track_id": string("Move the clip to this track (a track of the same kind)."),
        "source_in": number("Video/audio: second of the file the clip begins with (a slip edit: the clip stays where it is)."),
        "speed": number("Video/audio speed 0.1-10, pitch kept; the clip's length follows and later clips make room."),
        "keyframes": list(
            object(
                json!({
                    "time": number("Seconds from the clip's start (or give `at`)."),
                    "at": number("A timeline second, instead of `time`."),
                    "x": number("Centre x in %."),
                    "y": number("Centre y in %."),
                    "scale": number("1 = fitted size."),
                    "rotation": number("Degrees."),
                    "opacity": number("0-1."),
                    "easing": one_of(names_of(Easing::ALL), "How the motion that ends at this keyframe speeds up and slows down (default ease-in-out); back-* overshoot; custom uses `bezier`."),
                    "bezier": json!({ "type": "array", "items": { "type": "number" }, "description": "A curve of its own: cubic-bezier x1, y1, x2, y2 like CSS (y may leave 0..1 to overshoot, e.g. [0.3, 1.6, 0.6, 1])." }),
                }),
                &[],
            ),
            "Animates the clip: its position, scale, rotation and opacity at each keyframe, moving smoothly between them (zooms, pans, slides, pops, fades, hand-made shakes). Replaces the clip's keyframes; [] removes the animation. Fields a keyframe leaves out take the clip's base transform. get_project lists the current ones.",
        ),
        "text": string("Text clips: the words shown (\\n for a new line)."),
        "preset": one_of(preset_names(), "Text clips: a ready-made look (get_catalog describes them); `style` fields given with it win."),
        "style": described(text_style_schema(), "Text clips: the look; only the fields given change."),
        "shape": described(
            object(json!({
                "kind": one_of(names_of(ShapeKind::ALL), ""),
                "width": number("% of the canvas width."),
                "height": number("% of the canvas height."),
                "color": string("Hex color."),
                "blur": number("Blur regions: strength 2-120."),
            }), &[]),
            "Shape clips."),
        "volume": number("Video/audio: 0-4, 1 = as recorded."),
        "muted": boolean("Video/audio."),
        "fade_in": number("Video/audio: sound fade-in in seconds."),
        "fade_out": number("Video/audio: sound fade-out in seconds."),
        "voice_enhance": number("Video/audio: voice cleanup 0-1 (noise, harshness, levels); 0 = off."),
        "fit": one_of(names_of(Fit::ALL), "Videos/photos: contain shows the whole picture, cover fills the canvas (cropping), fill stretches."),
        "focus": described(object(json!({ "x": number("0-100"), "y": number("0-100") }), &[]), "With fit cover: which part of the picture stays in view (50,50 = centre)."),
        "crop": described(object(json!({ "left": number(""), "right": number(""), "top": number(""), "bottom": number("") }), &[]), "Videos/photos: % cut off each edge of the source (all 0 = no crop)."),
        "flip_x": boolean("Mirror horizontally."),
        "flip_y": boolean("Mirror vertically."),
        "radius": number("Rounded corners in project pixels."),
        "blend": one_of(names_of(BlendMode::ALL), "How the layer mixes with what is under it."),
        "adjust": described(
            object(json!({
                "exposure": number("-1..1"), "brightness": number("-1..1"), "contrast": number("-1..1"), "saturation": number("-1..1 (-1 = grey)"),
                "temperature": number("-1..1 (warm > 0)"), "tint": number("-1..1"), "hue": number("-1..1"), "vignette": number("0..1"),
            }), &[]),
            "Color correction; only the fields given change, all 0 = neutral."),
        "anim_in": animation_schema(anim_in_names(), "How the clip appears."),
        "anim_out": animation_schema(anim_out_names(), "How the clip leaves."),
        "transition": described(
            object(json!({ "kind": one_of(transition_names(), "\"none\" removes it."), "duration": number("Seconds, 0.1-2 (default 0.5).") }), &["kind"]),
            "Transition into this clip from the clip that ends exactly where it starts on the same track."),
        "asset_id": string("Video/photo/audio clips: show another file from the library instead (a b-roll swap); timing, position, effects and the rest stay."),
        "group": string("Clips with the same group select and move together; \"\" takes the clip out of its group."),
        "hidden": boolean("Hidden clips are neither shown nor heard."),
        "locked": boolean("Locked clips can't be changed until unlocked."),
    })
    .as_object()
    .cloned()
    .unwrap_or_default();
    fields.extend(transform_fields());
    fields
}

fn with_fields(base: Value, extra: Map<String, Value>, required: &[&str]) -> Value {
    let mut properties = base.as_object().cloned().unwrap_or_default();
    for (key, value) in extra {
        properties.entry(key).or_insert(value);
    }
    object(Value::Object(properties), required)
}

fn spec(name: &'static str, title: &'static str, description: &str, schema: Value) -> ToolSpec {
    ToolSpec { name, title, description: description.to_string(), schema, read_only: false, app: false }
}

fn look(name: &'static str, title: &'static str, description: &str, schema: Value) -> ToolSpec {
    ToolSpec { read_only: true, ..spec(name, title, description, schema) }
}

fn no_args() -> Value {
    object(json!({}), &[])
}

pub fn all() -> Vec<ToolSpec> {
    let item_fields = item_fields();
    let aspects: Vec<&str> = ASPECT_PRESETS.iter().map(|p| p.id).collect();
    let export_fields = json!({
        "format": one_of(names_of(ExportFormat::ALL), ""),
        "codec": one_of(names_of(ExportCodec::ALL), "h264 plays everywhere, h265 makes smaller files, vp9 is for webm."),
        "quality": one_of(names_of(ExportQuality::ALL), ""),
        "resolution": number("Short side in pixels: 480, 720, 1080, 1440 or 2160."),
        "fps": number("24, 25, 30, 50 or 60."),
        "loudness": number("Target loudness in LUFS (-14 suits social media); 0 = leave the mix as it is."),
        "hardware": boolean("Use the GPU encoder when there is one."),
    });
    vec![
        // ------------------------------------------------------------ projects (AI apps)
        ToolSpec {
            app: true,
            ..look("list_projects", "List projects", "Lists the user's ViralFlow projects (id, name, format, length, last edit), newest first, and which one is open.", no_args())
        },
        ToolSpec {
            app: true,
            ..spec("open_project", "Open project", "Opens a project in the ViralFlow editor (the one open before is saved). Every other tool works on the open project.", object(json!({ "project_id": string("") }), &["project_id"]))
        },
        ToolSpec {
            app: true,
            ..spec(
                "create_project",
                "Create project",
                "Creates an empty project and opens it.",
                object(json!({ "name": string(""), "aspect": one_of(aspects.clone(), "Canvas shape; 9:16 for TikTok, Reels and Shorts (default)."), "fps": number("24, 25, 30 (default), 50 or 60.") }), &[]),
            )
        },
        // ------------------------------------------------------------ looking
        look(
            "get_project",
            "Read the timeline",
            "The open project: canvas, length, playhead, selection, export settings and every track with its clips (compact). Caption groups are summarised unless include_captions is true or a time window is given. Use get_items for every stored detail of particular clips.",
            object(
                json!({
                    "from": number("Only clips that end after this second."),
                    "to": number("Only clips that start before this second."),
                    "track_id": string("Only this track."),
                    "include_captions": boolean("List each caption clip instead of a summary per caption group."),
                }),
                &[],
            ),
        ),
        look("get_items", "Read clips", "Everything stored for the given clips (full JSON, as in the project file).", object(json!({ "item_ids": ids("") }), &["item_ids"])),
        look("list_media", "List media", "The project's media library: files with their ids, kind, length, size, whether they are on the timeline and whether the file is missing.", no_args()),
        look(
            "get_catalog",
            "Read the catalog",
            "What the editor offers: effects and transitions with descriptions, in/out animations, text style presets, fonts, stickers, shapes, canvas formats.",
            object(json!({ "topic": one_of(vec!["all", "effects", "transitions", "animations", "text", "fonts", "stickers"], "Default all.") }), &[]),
        ),
        look(
            "view_frames",
            "Look at the video",
            "Renders the video exactly as it will export at the given times and shows you the pictures. Use it to check text size and position, framing, colors and effects after visual edits.",
            object(json!({ "times": list(json!({ "type": "number" }), "Timeline seconds, up to 6 (default: the playhead)."), "width": number("Picture width in pixels, 160-1080 (default 432).") }), &[]),
        ),
        look(
            "view_media",
            "Look at a file",
            "Shows frames of a media file from the library (a photo, or a video at the given seconds of the file), e.g. to pick b-roll.",
            object(json!({ "asset_id": string(""), "times": list(json!({ "type": "number" }), "Seconds into the video file, up to 6 (default: 3 spread over it).") }), &["asset_id"]),
        ),
        look(
            "transcribe",
            "Transcribe speech",
            "Every spoken word of a video or audio clip with its timeline time ([start, end, word]) and the sentences. Uses the speech recognition chosen in Settings (local Whisper or cloud); results are kept, so asking again is instant. Use it to find what is said where: cuts, captions, b-roll moments.",
            object(json!({ "item_id": string("A video or audio clip on the timeline (default: the clip with the most speech)."), "language": string("Language code like en, pl, de; default: the user's caption language or detect.") }), &[]),
        ),
        look(
            "find_media_files",
            "Find media files",
            "Lists video, audio and photo files in a folder on the user's computer, to import them. Without a folder, looks in the user's Videos, Pictures, Music, Downloads and Desktop folders.",
            object(json!({ "folder": string("Absolute folder path."), "recursive": boolean("Also look in subfolders (default false).") }), &[]),
        ),
        look("job_status", "Check a job", "State of a long job (transcription, captions, beats, export); waits up to wait_seconds for it to finish.", object(json!({ "job_id": string(""), "wait_seconds": number("0-120 (default 30).") }), &["job_id"])),
        // ------------------------------------------------------------ media
        spec(
            "import_media",
            "Import media",
            "Adds files from the user's computer (absolute paths) to the project's media library, and optionally to the timeline one after another.",
            object(
                json!({ "paths": ids("Absolute file paths."), "add_to_timeline": boolean("Also place them on the timeline (default false)."), "start": number("Where the first one goes on the timeline (default: after the last clip).") }),
                &["paths"],
            ),
        ),
        spec(
            "add_clips",
            "Add clips",
            "Places media from the library on the timeline. Without start, a clip goes right after the last clip on tracks of its kind. Any property update_items knows (duration, source_in, x, y, scale, volume, fit…) can be set at once.",
            object(
                json!({
                    "clips": list(
                        with_fields(
                            json!({
                                "asset_id": string("Media id from list_media."),
                                "new_track": boolean("Put it on a new track above the others (e.g. b-roll over the main video)."),
                            }),
                            item_fields.clone(),
                            &["asset_id"],
                        ),
                        "",
                    )
                }),
                &["clips"],
            ),
        ),
        spec(
            "add_texts",
            "Add texts",
            "Adds text clips (titles, labels, call-outs). For captions of speech use generate_captions or set_captions instead.",
            object(
                json!({
                    "texts": list(
                        with_fields(json!({ "text": string("The words."), "start": number("Default: the playhead."), "duration": number("Default 3 s.") }), item_fields.clone(), &["text"]),
                        "",
                    )
                }),
                &["texts"],
            ),
        ),
        spec(
            "add_stickers",
            "Add stickers",
            "Adds emoji stickers (any emoji; get_catalog lists suggestions).",
            object(
                json!({
                    "stickers": list(
                        with_fields(json!({ "emoji": string(""), "start": number("Default: the playhead."), "duration": number("Default 3 s.") }), transform_fields(), &["emoji"]),
                        "",
                    )
                }),
                &["stickers"],
            ),
        ),
        spec(
            "add_shapes",
            "Add shapes",
            "Adds shapes: rectangles and circles (backgrounds, bars, frames), lines, or blur regions that blur what is under them (hide a face, a plate, a logo).",
            object(
                json!({
                    "shapes": list(
                        with_fields(
                            json!({
                                "kind": one_of(names_of(ShapeKind::ALL), ""),
                                "start": number("Default: the playhead."),
                                "duration": number("Default 3 s."),
                                "width": number("% of the canvas width."),
                                "height": number("% of the canvas height."),
                                "color": string("Hex color."),
                                "blur": number("Blur regions: strength 2-120."),
                                "radius": number("Rounded corners."),
                            }),
                            transform_fields(),
                            &["kind"],
                        ),
                        "",
                    )
                }),
                &["shapes"],
            ),
        ),
        // ------------------------------------------------------------ editing
        spec(
            "update_items",
            "Change clips",
            "Changes properties of clips: timing, position, text and its look, sound, color, animations, transition, keyframes… Only the fields given change. Each update reports the clip as it is afterwards.",
            object(json!({ "updates": list(with_fields(json!({ "id": string("Clip id.") }), item_fields.clone(), &["id"]), "") }), &["updates"]),
        ),
        spec(
            "delete_items",
            "Delete clips",
            "Removes clips. With ripple, later clips on the same tracks move left to close the gaps.",
            object(json!({ "item_ids": ids(""), "ripple": boolean("Close the gaps (default false).") }), &["item_ids"]),
        ),
        spec(
            "split_items",
            "Split clips",
            "Cuts clips in two at a timeline second: the given clips, or every unlocked clip under that second.",
            object(json!({ "time": number(""), "item_ids": ids("Default: every clip under `time`.") }), &["time"]),
        ),
        spec(
            "trim_items",
            "Trim clips",
            "Moves the start and/or end edge of clips to timeline seconds. Unlike update_items start, the content stays in place: trimming a video's start skips its first moments.",
            object(json!({ "trims": list(object(json!({ "id": string(""), "start": number("New start edge."), "end": number("New end edge.") }), &["id"]), "") }), &["trims"]),
        ),
        spec(
            "remove_time_ranges",
            "Cut out time ranges",
            "Removes stretches of time from the video and closes the gaps (a ripple delete): clips are cut at both ends of each range, what is inside goes and everything later moves left. By default on every unlocked track, so video, captions and sound stay in sync; with track_ids only those tracks change (e.g. keep the music running). The tool for removing silences, filler words and retakes.",
            object(
                json!({
                    "ranges": list(object(json!({ "start": number(""), "end": number("") }), &["start", "end"]), "Timeline seconds as they are now (they may overlap or come in any order)."),
                    "track_ids": ids("Only these tracks."),
                }),
                &["ranges"],
            ),
        ),
        spec(
            "duplicate_items",
            "Duplicate clips",
            "Copies clips; the copies go right after the originals, or at `start`.",
            object(json!({ "item_ids": ids(""), "start": number("Where the first copy starts.") }), &["item_ids"]),
        ),
        spec("detach_audio", "Detach audio", "Moves the sound of videos to audio clips of their own (in sync); the videos are muted.", object(json!({ "item_ids": ids("") }), &["item_ids"])),
        spec(
            "add_effect",
            "Add an effect",
            "Puts an effect on clips (videos, photos, texts, stickers), over the whole clip or from..to. A clip that has the effect already keeps one.",
            object(
                json!({
                    "item_ids": ids(""),
                    "kind": one_of(effect_names(), "See get_catalog for what each does."),
                    "amount": number("Strength 0-1 (default 0.5, the designed look)."),
                    "from": number("Timeline second it starts (default: the clip's start)."),
                    "to": number("Timeline second it ends (default: the clip's end)."),
                    "on_beat": number("Fire it on every n-th beat of the music instead (1 = every beat); needs find_beats first."),
                }),
                &["item_ids", "kind"],
            ),
        ),
        spec(
            "set_effects",
            "Set a clip's effects",
            "Replaces all effects of one clip (in order; [] removes them all).",
            object(
                json!({
                    "item_id": string(""),
                    "effects": list(
                        object(json!({ "kind": one_of(effect_names(), ""), "amount": number("0-1"), "from": number("Timeline second."), "to": number("Timeline second."), "on_beat": number("Every n-th beat.") }), &["kind"]),
                        "",
                    ),
                }),
                &["item_id", "effects"],
            ),
        ),
        spec(
            "set_transitions",
            "Set transitions",
            "Sets transitions on cuts: each goes on the clip after the cut. With track_id (and no list), the same transition on every cut of that track.",
            object(
                json!({
                    "transitions": list(object(json!({ "item_id": string("The clip after the cut."), "kind": one_of(transition_names(), ""), "duration": number("Seconds (default 0.5).") }), &["item_id", "kind"]), ""),
                    "track_id": string("Every cut of this track."),
                    "kind": one_of(transition_names(), "With track_id."),
                    "duration": number("With track_id."),
                }),
                &[],
            ),
        ),
        spec(
            "generate_captions",
            "Generate captions",
            "Transcribes the speech of a clip and adds timed captions on a captions track (replacing captions made from that clip before). Highlight presets light up each spoken word.",
            object(
                json!({
                    "item_id": string("A video or audio clip (default: the clip with the most speech)."),
                    "language": string("Language code (default: the user's setting or detect)."),
                    "words": number("Most words per caption, 1-8 (default 3)."),
                    "preset": one_of(std::iter::once("caption-default").chain(preset_names()).collect(), "Look of the captions (default caption-default: bold white with an outline)."),
                    "y": number("Vertical position in % (default 72)."),
                }),
                &[],
            ),
        ),
        spec(
            "set_captions",
            "Write captions",
            "Adds captions with the given text and times (e.g. translated, corrected or written by you), styled as one group. Highlight presets time each word by its length.",
            object(
                json!({
                    "captions": list(object(json!({ "text": string(""), "start": number(""), "end": number("") }), &["text", "start", "end"]), "Timeline seconds, in order, not overlapping."),
                    "replace_group": string("A caption group id from get_project to replace."),
                    "preset": one_of(std::iter::once("caption-default").chain(preset_names()).collect(), ""),
                    "y": number("Vertical position in % (default 72)."),
                }),
                &["captions"],
            ),
        ),
        spec(
            "find_beats",
            "Find the beats",
            "Analyses the music on the timeline and stores its beats, so effects can fire on them (add_effect on_beat). Returns the beat times and the tempo, also useful to cut on the beat.",
            object(json!({ "focus": one_of(vec!["bass", "all"], "Follow the kick and bass (default) or every hit."), "amount": number("0-1: only the strongest hits .. every hit (default 0.5).") }), &[]),
        ),
        spec(
            "edit_tracks",
            "Change tracks",
            "Adds, renames, reorders, locks, mutes, hides or deletes tracks (deleting removes its clips).",
            object(
                json!({
                    "actions": list(
                        object(
                            json!({
                                "action": one_of(vec!["add", "rename", "move", "delete", "lock", "unlock", "mute", "unmute", "hide", "show"], ""),
                                "track_id": string("Every action but add."),
                                "kind": one_of(vec!["video", "text", "audio"], "add"),
                                "name": string("add, rename"),
                                "index": number("move: new position, 0 = top."),
                            }),
                            &["action"],
                        ),
                        "",
                    )
                }),
                &["actions"],
            ),
        ),
        spec(
            "set_project",
            "Change the project",
            "Renames the project, changes the canvas format, background color, frame rate, beats or export settings.",
            with_fields(
                json!({
                    "name": string(""),
                    "aspect": one_of(aspects, "Canvas shape."),
                    "width": number("Canvas width in pixels (with height, instead of aspect)."),
                    "height": number(""),
                    "background": string("Hex color behind everything."),
                    "beats": list(json!({ "type": "number" }), "Beat times in timeline seconds, replacing the found ones (effects with on_beat fire on them); [] clears them."),
                }),
                export_fields.as_object().cloned().unwrap_or_default(),
                &[],
            ),
        ),
        spec("history", "Undo or redo", "Undoes or redoes the last changes (the user's and yours).", object(json!({ "action": one_of(vec!["undo", "redo"], ""), "steps": number("Default 1.") }), &["action"])),
        spec(
            "control_preview",
            "Control the preview",
            "Moves the playhead, plays or pauses, and selects clips, to show the user something.",
            object(json!({ "time": number("Playhead second."), "play": boolean("true plays, false pauses."), "select": ids("Clips to select ([] clears).") }), &[]),
        ),
        spec(
            "export_video",
            "Export the video",
            "Renders the video to a file. Settings given here are saved with the project. Without a path it goes to the user's export folder with a free name.",
            with_fields(json!({ "path": string("Absolute file path (.mp4, .mov or .webm).") }), export_fields.as_object().cloned().unwrap_or_default(), &[]),
        ),
    ]
}

/// Tools in OpenAI function format (the chat leaves out the project tools).
pub fn functions(chat: bool) -> Value {
    Value::Array(
        all()
            .into_iter()
            .filter(|s| !(chat && s.app))
            .map(|s| json!({ "type": "function", "function": { "name": s.name, "description": s.description, "parameters": s.schema } }))
            .collect(),
    )
}

/// Tools as MCP lists them.
pub fn mcp_tools() -> Value {
    Value::Array(
        all()
            .into_iter()
            .map(|s| {
                json!({
                    "name": s.name,
                    "title": s.title,
                    "description": s.description,
                    "inputSchema": s.schema,
                    "annotations": { "title": s.title, "readOnlyHint": s.read_only, "destructiveHint": false, "openWorldHint": false },
                })
            })
            .collect(),
    )
}

pub fn title(name: &str) -> String {
    all().into_iter().find(|s| s.name == name).map(|s| s.title.to_string()).unwrap_or_else(|| name.replace('_', " "))
}

pub fn exists(name: &str) -> bool {
    all().iter().any(|s| s.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_have_unique_names_and_object_schemas() {
        let tools = all();
        let names: std::collections::HashSet<_> = tools.iter().map(|t| t.name).collect();
        assert_eq!(names.len(), tools.len());
        for tool in &tools {
            assert_eq!(tool.schema["type"], "object", "{}", tool.name);
            assert!(tool.description.len() > 20, "{}", tool.name);
            // Every required field is a property.
            for required in tool.schema["required"].as_array().unwrap() {
                assert!(tool.schema["properties"].get(required.as_str().unwrap()).is_some(), "{} requires {required}", tool.name);
            }
        }
        assert!(functions(true).as_array().unwrap().len() < functions(false).as_array().unwrap().len(), "the chat has no project tools");
        // No type unions anywhere (some providers reject them).
        assert!(!serde_json::to_string(&functions(false)).unwrap().contains("\"type\":["));
    }
}
