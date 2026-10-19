//! The inspector: pushes the selected clip's properties to `InspectorState` and
//! turns field edits back into `Editor` calls. Fields are addressed by name
//! ("transform.x", "text.color", ...) so the Slint side stays declarative.

use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::{Color, ComponentHandle, Model, ModelRc, SharedString, VecModel};
use vf_core::edit::{AnimationChoice, Editor, Flag, Slot, TransformPatch};
use vf_core::engine::animation::{animation_windows, ANIMATIONS_IN, ANIMATIONS_OUT};
use vf_core::engine::effects::{effect_label, effect_preset};
use vf_core::engine::keyframes::{keyframe_at, normalize_transform, resolve_transform};
use vf_core::engine::transitions::{track_seams, MAX_TRANSITION, TRANSITIONS};
use vf_core::model::*;
use vf_core::presets::{text_preset, ASPECT_PRESETS, TEXT_PRESETS};

use super::EditorController;
use crate::{
    format, AdjustData, AnimationData, AudioData, Choice, EffectRow, FrameData, InspectorState, ItemInfo, ProjectData, ShapeData, SpeedData, Stat, TextData,
    TransformData,
};

/// Typing within this time of the last keystroke stays in one undo step.
const TYPING_GROUP: Duration = Duration::from_millis(1200);
const DEFAULT_CURVE: Bezier = [0.42, 0.0, 0.58, 1.0];

/// What the inspector last showed; it is refreshed only when this changes.
#[derive(Clone, PartialEq, Default)]
pub(super) struct SyncKey {
    revision: u64,
    selection: Vec<String>,
    primary: Option<String>,
    /// Playhead frame, only while it matters (keyframes).
    frame: i64,
}

impl EditorController {
    pub(super) fn bind_inspector(self: &Rc<Self>, state: &InspectorState) {
        macro_rules! on {
            ($method:ident, |$this:ident $(, $arg:ident)*| $body:expr) => {{
                let weak = Rc::downgrade(self);
                state.$method(move |$($arg),*| {
                    if let Some($this) = weak.upgrade() {
                        $body
                    }
                });
            }};
        }
        on!(on_begin, |this| {
            this.with(|s| s.editor.begin_change());
        });
        on!(on_number, |this, field, value| this.edit(|e| set_number(e, &field, value as f64)));
        on!(on_commit, |this, field, value| this.edit(|e| {
            e.begin_change();
            set_number(e, &field, value as f64)
        }));
        on!(on_text_changed, |this, field, value| this.inspector_text(&field, &value));
        on!(on_choose, |this, field, value| this.edit(|e| choose(e, &field, &value)));
        on!(on_toggle, |this, field| this.edit(|e| toggle(e, &field)));
        on!(on_color, |this, field, color| this.edit(|e| set_color(e, &field, &hex_of(color))));
        on!(on_hex, |this, field, value| {
            match parse_hex(&value) {
                Some(color) => this.edit(|e| {
                    e.begin_change();
                    set_color(e, &field, &hex_of(color))
                }),
                // Invalid input: show the stored value again.
                None => this.refresh_inspector(),
            }
        });
        on!(on_action, |this, name| this.inspector_action(&name));
        on!(on_bezier, |this, index, x, y| this.edit(|e| move_curve_point(e, index as usize, x as f64, y as f64)));
        state.on_matches(|family, query| family.to_lowercase().contains(&query.to_lowercase()));

        state.set_fonts(strings(self.text.families()));
        state.set_animations_in(choices(ANIMATIONS_IN.iter().map(|p| (p.kind.as_str(), p.label))));
        state.set_animations_out(choices(ANIMATIONS_OUT.iter().map(|p| (p.kind.as_str(), p.label))));
        state.set_transitions(choices(std::iter::once(("none", "None")).chain(TRANSITIONS.iter().map(|p| (p.kind.as_str(), p.label)))));
        // The style presets show what they look like, as in the text panel.
        let previews = super::assets::text_preset_previews(&self.text);
        let presets: Vec<Choice> = TEXT_PRESETS
            .iter()
            .map(|p| Choice {
                icon: previews.iter().find(|card| card.id == p.id).map(|card| card.preview.clone()).unwrap_or_default(),
                ..choice(p.id, p.label)
            })
            .collect();
        state.set_text_presets(ModelRc::new(VecModel::from(presets)));
    }

    fn inspector_text(&self, field: &str, value: &str) {
        match field {
            "content" => {
                let record = self
                    .with(|s| {
                        let fresh = s.typing.is_none_or(|at| at.elapsed() > TYPING_GROUP);
                        s.typing = Some(Instant::now());
                        fresh
                    })
                    .unwrap_or(false);
                self.edit(|e| e.update_text(record, Some(value), |_| {}));
            }
            "project.name" => self.edit(|e| e.rename(value)),
            _ => {}
        }
    }

    fn inspector_action(self: &Rc<Self>, name: &str) {
        match name {
            "reset-transform" => self.edit(|e| {
                let defaults = normalize_transform(None);
                let patch = TransformPatch {
                    x: Some(defaults.x),
                    y: Some(defaults.y),
                    scale: Some(defaults.scale),
                    rotation: Some(defaults.rotation),
                    opacity: Some(defaults.opacity),
                };
                let updates: Vec<_> = visual_selection(e).into_iter().map(|id| (id, patch)).collect();
                e.set_transform(&updates, true);
            }),
            "rotate-90" => self.edit(|e| {
                let time = e.time;
                let updates: Vec<_> = visual_selection(e)
                    .into_iter()
                    .filter_map(|id| {
                        let item = e.item(&id)?;
                        let current = current_transform(item, time);
                        let rotation = ((current.rotation / 90.0).round() * 90.0 + 90.0) % 360.0;
                        Some((id, TransformPatch { rotation: Some(rotation), ..Default::default() }))
                    })
                    .collect();
                e.set_transform(&updates, true);
            }),
            "toggle-keyframe" => self.edit(|e| e.toggle_keyframe_at_playhead()),
            "clear-keyframes" => self.edit(|e| e.clear_keyframes()),
            "previous-keyframe" | "next-keyframe" => {
                let time = self.with(|s| {
                    s.editor.jump_keyframe(if name == "next-keyframe" { 1 } else { -1 });
                    s.editor.time
                });
                if let Some(time) = time {
                    self.seek(time);
                }
            }
            "reset-crop" => self.edit(|e| {
                e.update_selected(true, |item| {
                    item.crop = None;
                    item.focus = None;
                })
            }),
            "reset-adjust" => self.edit(|e| e.update_selected(true, |item| item.adjust = None)),
            "style-to-group" => {
                let count = self.with(|s| s.editor.apply_style_to_group()).unwrap_or(0);
                self.changed();
                if count > 0 {
                    self.context.toasts.success("Style applied", &format!("Updated {count} caption{}.", if count == 1 { "" } else { "s" }));
                }
            }
            "preview-in" | "preview-out" => self.preview_animation(name == "preview-in"),
            "captions" => self.open_panel("captions"),
            "detach-audio" => self.edit(|e| e.detach_audio_selected()),
            "open-effects" => self.open_panel("effects"),
            "rewrite" => self.rewrite_text(),
            _ if name.starts_with("remove-effect:") => {
                let effect = &name["remove-effect:".len()..];
                self.edit(|e| {
                    if let Some(id) = e.primary.clone() {
                        e.remove_effect(&id, effect);
                    }
                });
            }
            _ => {}
        }
    }

    /// Plays the animated part of the clip once so the user sees the result.
    fn preview_animation(&self, entrance: bool) {
        let range = self.with(|s| {
            let id = s.editor.primary.clone()?;
            let span = s.editor.motion_span_of(&id)?;
            let windows = animation_windows(&span);
            let length = if entrance { windows.in_duration } else { windows.out_duration };
            if length <= 0.0 {
                return None;
            }
            let from = if entrance { span.start } else { span.start + span.duration - length - 0.2 };
            Some((from.max(0.0), length))
        });
        let Some(Some((from, length))) = range else { return };
        self.with(|s| {
            if s.playing() {
                s.audio.stop();
            }
            s.editor.seek(from);
        });
        self.toggle_play();
        self.with(|s| s.stop_at = Some(from + length + 0.35));
    }

    /// Rewrites the selected text with the OpenRouter model (punchier, same length).
    fn rewrite_text(self: &Rc<Self>) {
        let Some(window) = self.context.window() else { return };
        let state = window.global::<InspectorState>();
        let Some(Some((id, text))) =
            self.with(|s| s.editor.selected_item().filter(|i| i.kind == ItemKind::Text).map(|i| (i.id.clone(), i.text_style().content)))
        else {
            return;
        };
        if text.trim().is_empty() || state.get_rewriting() {
            return;
        }
        let key = self.context.keys.key();
        let model = self.context.settings.get().openrouter_model;
        state.set_rewriting(true);
        let weak = Rc::downgrade(self);
        crate::tasks::background(
            "ai-rewrite",
            move || {
                let client = vf_ai::openrouter::Client::new(&key, &model)?;
                vf_ai::openrouter::rewrite(&client, &text, "Make it punchier for a short video caption. Keep it about the same length.")
            },
            move |result| {
                let Some(this) = weak.upgrade() else { return };
                if let Some(window) = this.context.window() {
                    window.global::<InspectorState>().set_rewriting(false);
                }
                match result {
                    Ok(content) if !content.is_empty() => this.edit(|e| {
                        e.update_item(&id, true, |item| {
                            if let Some(style) = item.text.as_mut() {
                                style.content = content.clone();
                            }
                            item.name = vf_core::presets::item_name(&content, "Text");
                        })
                    }),
                    Ok(_) => this.context.toasts.error("Rewrite failed", "The AI model returned an empty answer."),
                    Err(error) => this.context.toasts.error("Rewrite failed", &error.to_string()),
                }
            },
        );
    }

    fn refresh_inspector(&self) {
        self.with(|s| s.inspector_key = None);
        self.sync_ui();
    }
}

// ------------------------------------------------------------ edits

fn visual_selection(editor: &Editor) -> Vec<String> {
    editor.selected_items().into_iter().filter(|item| item.kind.is_visual()).map(|item| item.id.clone()).collect()
}

fn current_transform(item: &TimelineItem, time: f64) -> Transform {
    if item.keyframes.is_empty() {
        normalize_transform(item.transform.as_ref())
    } else {
        resolve_transform(item, time - item.start)
    }
}

fn set_transform(editor: &mut Editor, patch: TransformPatch) {
    let updates: Vec<_> = visual_selection(editor).into_iter().map(|id| (id, patch)).collect();
    editor.set_transform(&updates, false);
}

/// Live numeric change (the gesture called `begin` first).
fn set_number(editor: &mut Editor, field: &str, value: f64) {
    if !value.is_finite() {
        return;
    }
    let none = TransformPatch::default();
    match field {
        "transform.x" => set_transform(editor, TransformPatch { x: Some(value), ..none }),
        "transform.y" => set_transform(editor, TransformPatch { y: Some(value), ..none }),
        "transform.scale" => set_transform(editor, TransformPatch { scale: Some(value.clamp(0.05, 10.0)), ..none }),
        "transform.rotation" => set_transform(editor, TransformPatch { rotation: Some(value), ..none }),
        "transform.opacity" => set_transform(editor, TransformPatch { opacity: Some(value.clamp(0.0, 1.0)), ..none }),
        "frame.focus-x" | "frame.focus-y" => editor.update_selected(false, |item| {
            let mut focus = item.focus.unwrap_or_default();
            if field == "frame.focus-x" {
                focus.x = value;
            } else {
                focus.y = value;
            }
            item.focus = Some(focus);
        }),
        "radius" => editor.update_selected(false, |item| item.radius = Some(value.max(0.0))),
        "crop.left" | "crop.right" | "crop.top" | "crop.bottom" => editor.update_selected(false, |item| {
            let crop = item.crop.unwrap_or_default();
            let mut edges = [crop.x * 100.0, (1.0 - crop.x - crop.w) * 100.0, crop.y * 100.0, (1.0 - crop.y - crop.h) * 100.0];
            let index = ["crop.left", "crop.right", "crop.top", "crop.bottom"].iter().position(|f| *f == field).unwrap_or(0);
            edges[index] = value.max(0.0);
            // Opposite edges always leave at least 5% of the source.
            let [left, right, top, bottom] = edges;
            let (left, right) = (left.min(95.0 - right) / 100.0, right.min(95.0 - left) / 100.0);
            let (top, bottom) = (top.min(95.0 - bottom) / 100.0, bottom.min(95.0 - top) / 100.0);
            item.crop = Some(Crop { x: left, y: top, w: 1.0 - left - right, h: 1.0 - top - bottom });
        }),
        "shape.width" | "shape.height" | "shape.blur" => editor.update_selected(false, |item| {
            let Some(shape) = item.shape.as_mut() else { return };
            match field {
                "shape.width" => {
                    if shape.kind == ShapeKind::Circle && shape.width == shape.height {
                        shape.height = value;
                    }
                    shape.width = value;
                }
                "shape.height" => shape.height = value,
                _ => shape.blur = value,
            }
        }),
        "audio.volume" => editor.update_selected(false, |item| item.volume = Some(value.clamp(0.0, 4.0))),
        "audio.fade-in" => editor.update_selected(false, |item| item.fade_in = Some(value.max(0.0))),
        "audio.fade-out" => editor.update_selected(false, |item| item.fade_out = Some(value.max(0.0))),
        "audio.enhance" => editor.update_selected(false, |item| item.voice_enhance = (value > 0.0).then_some(value.min(1.0))),
        "speed" => editor.set_speed(value, false),
        "anim-in.duration" => editor.set_animation_duration(Slot::In, value),
        "anim-out.duration" => editor.set_animation_duration(Slot::Out, value),
        _ if field.starts_with("effect-amount:") => {
            let Some(id) = editor.primary.clone() else { return };
            editor.set_effect_amount(&id, &field["effect-amount:".len()..], value, false);
        }
        "transition.duration" => {
            let Some(id) = editor.primary.clone() else { return };
            let kind = editor.item(&id).and_then(|item| item.transition).map(|t| t.kind).unwrap_or_default();
            if kind != TransitionKind::None {
                editor.set_transition(&id, kind, Some(value), false);
            }
        }
        _ if field.starts_with("adjust.") => editor.update_selected(false, |item| {
            let mut adjust = item.adjust.unwrap_or_default();
            let slot = match &field[7..] {
                "exposure" => &mut adjust.exposure,
                "brightness" => &mut adjust.brightness,
                "contrast" => &mut adjust.contrast,
                "saturation" => &mut adjust.saturation,
                "temperature" => &mut adjust.temperature,
                "tint" => &mut adjust.tint,
                "hue" => &mut adjust.hue,
                "vignette" => &mut adjust.vignette,
                _ => return,
            };
            *slot = value;
            item.adjust = (!adjust.is_neutral()).then_some(adjust);
        }),
        _ if field.starts_with("text.") => {
            let key = &field[5..];
            editor.update_text(false, None, |text| match key {
                "size" => text.size = value.clamp(8.0, 600.0),
                "letter-spacing" => text.letter_spacing = value,
                "line-height" => text.line_height = value,
                "max-width" => text.max_width = value,
                "stroke-width" => text.stroke_width = value.max(0.0),
                "background-opacity" => text.background_opacity = value.clamp(0.0, 1.0),
                "background-padding" => text.background_padding = value.max(0.0),
                "background-radius" => text.background_radius = value.max(0.0),
                "shadow-opacity" => text.shadow_opacity = value.clamp(0.0, 1.0),
                "shadow-blur" => text.shadow_blur = value.max(0.0),
                "shadow-distance" => text.shadow_distance = value.max(0.0),
                "highlight-index" => text.highlight_index = value.round().max(0.0) as i64,
                _ => {}
            });
        }
        _ => log::debug!("inspector: unknown number field {field}"),
    }
}

/// A choice from a list or segmented control (one undo step).
fn choose(editor: &mut Editor, field: &str, value: &str) {
    match field {
        "blend" => editor.update_selected(true, |item| item.blend = BlendMode::parse(value).filter(|b| *b != BlendMode::Normal)),
        "fit" => editor.update_selected(true, |item| item.fit = Fit::parse(value)),
        "easing" => {
            let Some(easing) = Easing::parse(value) else { return };
            let curve = (easing == Easing::Custom).then(|| current_keyframe(editor).and_then(|k| k.bezier).unwrap_or(DEFAULT_CURVE));
            editor.set_keyframe_easing_at_playhead(easing, curve, true);
        }
        "preset" => {
            if let Some(preset) = text_preset(value) {
                editor.apply_text_preset(preset);
            }
        }
        "font" => editor.update_text(true, None, |text| text.font = value.to_string()),
        "weight" => {
            if let Ok(weight) = value.parse::<f64>() {
                editor.update_text(true, None, |text| text.weight = weight);
            }
        }
        "align" => editor.update_text(true, None, |text| text.align = TextAlign::parse(value).unwrap_or_default()),
        "highlight-mode" => editor.set_highlight_mode(HighlightMode::parse(value).unwrap_or_default()),
        "anim-in" => {
            if let Some(kind) = AnimationInKind::parse(value) {
                editor.set_animation(AnimationChoice::In(kind));
            }
        }
        "anim-out" => {
            if let Some(kind) = AnimationOutKind::parse(value) {
                editor.set_animation(AnimationChoice::Out(kind));
            }
        }
        "transition" => {
            if let Some(id) = editor.primary.clone() {
                editor.set_transition(&id, TransitionKind::parse(value).unwrap_or_default(), None, true);
            }
        }
        "aspect" => {
            if let Some(preset) = ASPECT_PRESETS.iter().find(|p| p.id == value) {
                editor.set_canvas(preset.width, preset.height);
            }
        }
        "fps" => {
            if let Ok(fps) = value.parse::<u32>() {
                if fps != editor.project.fps {
                    editor.begin_change();
                    editor.update_export(|export| export.fps = fps);
                }
            }
        }
        _ => log::debug!("inspector: unknown choice {field}"),
    }
}

fn toggle(editor: &mut Editor, field: &str) {
    let primary = editor.selected_item().cloned();
    match field {
        "flip-x" => {
            let value = !primary.is_some_and(|item| item.flip_x);
            editor.update_selected(true, |item| item.flip_x = value);
        }
        "flip-y" => {
            let value = !primary.is_some_and(|item| item.flip_y);
            editor.update_selected(true, |item| item.flip_y = value);
        }
        "italic" | "uppercase" => {
            let style = primary.map(|item| item.text_style()).unwrap_or_default();
            let value = if field == "italic" { !style.italic } else { !style.uppercase };
            editor.update_text(true, None, |text| {
                if field == "italic" {
                    text.italic = value;
                } else {
                    text.uppercase = value;
                }
            });
        }
        "muted" => editor.toggle_item_flag(Flag::Muted),
        _ => {}
    }
}

/// Live colour change; `begin` was called by the picker (or by `hex`).
fn set_color(editor: &mut Editor, field: &str, hex: &str) {
    let hex = hex.to_string();
    match field {
        "project.background" => editor.set_background(&hex, false),
        "shape.color" => editor.update_selected(false, |item| {
            if let Some(shape) = item.shape.as_mut() {
                shape.color = hex.clone();
            }
        }),
        _ if field.starts_with("text.") => {
            let key = field[5..].to_string();
            editor.update_text(false, None, |text| {
                let slot = match key.as_str() {
                    "color" => &mut text.color,
                    "stroke-color" => &mut text.stroke_color,
                    "background-color" => &mut text.background_color,
                    "shadow-color" => &mut text.shadow_color,
                    "highlight-color" => &mut text.highlight_color,
                    "word-color" => &mut text.highlight_text_color,
                    _ => return,
                };
                *slot = hex.clone();
            });
        }
        _ => {}
    }
}

fn current_keyframe(editor: &Editor) -> Option<Keyframe> {
    let item = editor.selected_item()?;
    keyframe_at(item, editor.time - item.start, editor.keyframe_tolerance())
}

fn move_curve_point(editor: &mut Editor, index: usize, x: f64, y: f64) {
    let mut curve = current_keyframe(editor).and_then(|k| k.bezier).unwrap_or(DEFAULT_CURVE);
    let offset = if index == 0 { 0 } else { 2 };
    curve[offset] = x.clamp(0.0, 1.0);
    curve[offset + 1] = y.clamp(-0.6, 1.6);
    editor.set_keyframe_easing_at_playhead(Easing::Custom, Some(curve), false);
}

// ------------------------------------------------------------ colours

pub(super) fn parse_hex(value: &str) -> Option<Color> {
    let digits = value.trim().trim_start_matches('#');
    let expanded: String = match digits.len() {
        3 => digits.chars().flat_map(|c| [c, c]).collect(),
        6 => digits.to_string(),
        _ => return None,
    };
    let number = u32::from_str_radix(&expanded, 16).ok()?;
    Some(Color::from_rgb_u8((number >> 16) as u8, (number >> 8) as u8, number as u8))
}

pub(super) fn hex_of(color: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", color.red(), color.green(), color.blue())
}

fn color_pair(value: &str, fallback: &str) -> (Color, SharedString) {
    let color = parse_hex(value).or_else(|| parse_hex(fallback)).unwrap_or_default();
    (color, hex_of(color).into())
}

// ------------------------------------------------------------ sync

fn strings(list: Vec<String>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(list.into_iter().map(SharedString::from).collect::<Vec<_>>()))
}

fn choice(value: &str, label: &str) -> Choice {
    Choice { value: value.into(), label: label.into(), ..Default::default() }
}

fn choices<'a>(list: impl Iterator<Item = (&'a str, &'a str)>) -> ModelRc<Choice> {
    ModelRc::new(VecModel::from(list.map(|(value, label)| choice(value, label)).collect::<Vec<_>>()))
}

fn tabs_for(kind: ItemKind) -> &'static [&'static str] {
    match kind {
        ItemKind::Video => &["main", "animation", "adjust", "audio", "speed"],
        ItemKind::Image => &["main", "animation", "adjust"],
        ItemKind::Text => &["main", "animation", "transform"],
        ItemKind::Shape => &["main", "animation"],
        ItemKind::Audio => &["main", "speed"],
    }
}

fn kind_index(kind: ItemKind) -> i32 {
    match kind {
        ItemKind::Video => 0,
        ItemKind::Image => 1,
        ItemKind::Audio => 2,
        ItemKind::Text => 3,
        ItemKind::Shape => 4,
    }
}

/// Pushes the selection's properties when anything they depend on changed.
pub(super) fn sync(editor: &Editor, state: &InspectorState, last: &mut Option<SyncKey>) {
    let item = editor.selected_item();
    let frame = match item {
        Some(item) if item.kind.is_visual() => vf_core::time::frame_index(editor.time, editor.fps()),
        _ => -1,
    };
    let key = SyncKey { revision: editor.revision(), selection: editor.selection.clone(), primary: editor.primary.clone(), frame };
    if last.as_ref() == Some(&key) {
        return;
    }
    let kind_changed = last.as_ref().is_none_or(|old| old.primary != key.primary);
    *last = Some(key);

    let project = &editor.project;
    let Some(item) = item else {
        state.set_item(ItemInfo::default());
        sync_project(editor, state);
        return;
    };
    if kind_changed && !tabs_for(item.kind).contains(&state.get_tab().as_str()) {
        state.set_tab("main".into());
    }
    let caption = item.group_id.as_deref().is_some_and(|g| g.starts_with("captions-"));
    state.set_item(ItemInfo {
        has: true,
        count: editor.selection.len() as i32,
        title: item.name.clone().into(),
        kind: kind_index(item.kind),
        caption,
        grouped: item.group_id.is_some(),
    });

    if item.kind.is_visual() {
        let elapsed = editor.time - item.start;
        let tolerance = editor.keyframe_tolerance();
        let transform = current_transform(item, editor.time);
        let keyframe = keyframe_at(item, elapsed, tolerance);
        let curve = keyframe.as_ref().and_then(|k| k.bezier).unwrap_or(DEFAULT_CURVE);
        state.set_transform(TransformData {
            x: transform.x as f32,
            y: transform.y as f32,
            scale: transform.scale as f32,
            rotation: transform.rotation as f32,
            opacity: transform.opacity as f32,
            blend: item.blend.unwrap_or_default().as_str().into(),
            flip_x: item.flip_x,
            flip_y: item.flip_y,
            animated: !item.keyframes.is_empty(),
            inside: elapsed >= -tolerance && elapsed <= item.duration + tolerance,
            keyed: keyframe.is_some(),
            easing: keyframe.map(|k| k.easing).unwrap_or_default().as_str().into(),
            b1: curve[0] as f32,
            b2: curve[1] as f32,
            b3: curve[2] as f32,
            b4: curve[3] as f32,
        });
    }

    match item.kind {
        ItemKind::Video | ItemKind::Image => {
            let crop = item.crop.unwrap_or_default();
            let focus = item.focus.unwrap_or_default();
            state.set_frame(FrameData {
                fit: item.fit.unwrap_or_default().as_str().into(),
                focus_x: focus.x as f32,
                focus_y: focus.y as f32,
                radius: item.radius() as f32,
                left: (crop.x * 100.0) as f32,
                right: ((1.0 - crop.x - crop.w) * 100.0) as f32,
                top: (crop.y * 100.0) as f32,
                bottom: ((1.0 - crop.y - crop.h) * 100.0) as f32,
                cropped: item.crop.is_some() || item.focus.is_some(),
            });
            let adjust = item.adjust.unwrap_or_default();
            state.set_adjust(AdjustData {
                exposure: adjust.exposure as f32,
                brightness: adjust.brightness as f32,
                contrast: adjust.contrast as f32,
                saturation: adjust.saturation as f32,
                temperature: adjust.temperature as f32,
                tint: adjust.tint as f32,
                hue: adjust.hue as f32,
                vignette: adjust.vignette as f32,
                changed: !adjust.is_neutral(),
            });
        }
        ItemKind::Text => sync_text(item, state),
        ItemKind::Shape => {
            let shape = item.shape.clone().unwrap_or_default();
            let (color, hex) = color_pair(&shape.color, "#ffffff");
            state.set_shape(ShapeData {
                kind: shape.kind.as_str().into(),
                color,
                hex,
                width: shape.width as f32,
                height: shape.height as f32,
                radius: item.radius() as f32,
                blur: shape.blur as f32,
                square: shape.kind == ShapeKind::Circle && shape.width == shape.height,
            });
        }
        ItemKind::Audio => {}
    }

    if item.kind.has_sound() {
        let silent = item.kind == ItemKind::Video && project.asset(item.asset_id.as_deref()).and_then(|a| a.has_audio) == Some(false);
        state.set_audio(AudioData {
            volume: item.volume() as f32,
            fade_in: item.fade_in() as f32,
            fade_out: item.fade_out() as f32,
            max_fade: (item.duration / 2.0).clamp(0.1, 10.0) as f32,
            enhance: item.voice_enhance.unwrap_or(0.0) as f32,
            muted: item.muted,
            silent,
            can_detach: editor.can_detach_audio(item),
        });
        state.set_speed(SpeedData { speed: item.speed() as f32, duration: format::duration(item.duration).into() });
    }

    if item.kind.is_visual() {
        sync_animation(editor, item, state);
        sync_effects(item, state);
    }
}

/// The clip's effects, row by row: a slider being dragged keeps its row.
fn sync_effects(item: &TimelineItem, state: &InspectorState) {
    let rows: Vec<EffectRow> = item
        .active_effects()
        .map(|effect| EffectRow {
            id: effect.id.clone().into(),
            label: effect_label(effect.kind).into(),
            amount: (effect.amount() * 100.0) as f32,
            adjustable: effect_preset(effect.kind).is_some_and(|p| p.adjustable),
        })
        .collect();
    let model = state.get_clip_effects();
    if model.row_count() != rows.len() || model.iter().zip(&rows).any(|(old, new)| old.id != new.id) {
        state.set_clip_effects(ModelRc::new(VecModel::from(rows)));
        return;
    }
    for (index, row) in rows.into_iter().enumerate() {
        if model.row_data(index).as_ref() != Some(&row) {
            model.set_row_data(index, row);
        }
    }
}

fn sync_text(item: &TimelineItem, state: &InspectorState) {
    let text = item.text_style();
    let (color, color_hex) = color_pair(&text.color, "#ffffff");
    let (stroke_color, stroke_hex) = color_pair(&text.stroke_color, "#000000");
    let (background_color, background_hex) = color_pair(&text.background_color, "#000000");
    let (shadow_color, shadow_hex) = color_pair(&text.shadow_color, "#000000");
    let (highlight_color, highlight_hex) = color_pair(&text.highlight_color, "#ffe14d");
    let word = if text.highlight_text_color.is_empty() { text.color.as_str() } else { text.highlight_text_color.as_str() };
    let (word_color, word_hex) = color_pair(word, "#ffffff");
    state.set_text(TextData {
        content: text.content.clone().into(),
        font: text.font.clone().into(),
        size: text.size as f32,
        weight: format!("{}", (text.weight / 100.0).round() as i32 * 100).into(),
        italic: text.italic,
        uppercase: text.uppercase,
        color,
        color_hex,
        align: text.align.as_str().into(),
        letter_spacing: text.letter_spacing as f32,
        line_height: text.line_height as f32,
        max_width: text.max_width as f32,
        stroke_color,
        stroke_hex,
        stroke_width: text.stroke_width as f32,
        background_color,
        background_hex,
        background_opacity: text.background_opacity as f32,
        background_padding: text.background_padding as f32,
        background_radius: text.background_radius as f32,
        shadow_color,
        shadow_hex,
        shadow_opacity: text.shadow_opacity as f32,
        shadow_blur: text.shadow_blur as f32,
        shadow_distance: text.shadow_distance as f32,
        highlight_mode: text.highlight_mode.as_str().into(),
        highlight_color,
        highlight_hex,
        word_color,
        word_hex,
        highlight_index: text.highlight_index.max(0) as i32,
        words: text.content.split_whitespace().count() as i32,
    });
}

fn sync_animation(editor: &Editor, item: &TimelineItem, state: &InspectorState) {
    let Some(span) = editor.motion_span_of(&item.id) else { return };
    let windows = animation_windows(&span);
    let anim_in = span.anim_in.filter(|a| a.kind != AnimationInKind::None);
    let anim_out = span.anim_out.filter(|a| a.kind != AnimationOutKind::None);
    let seam = editor
        .find(&item.id)
        .and_then(|(t, _)| track_seams(&editor.project.tracks[t], editor.fps()).into_iter().find(|seam| seam.to == item.id).map(|seam| (t, seam)));
    let (has_seam, seam_kind, seam_duration, seam_max) = match &seam {
        Some((t, seam)) => {
            let from = editor.project.tracks[*t].items.iter().find(|c| c.id == seam.from).map(|c| c.duration).unwrap_or(MAX_TRANSITION);
            (true, seam.kind.as_str(), seam.duration, MAX_TRANSITION.min(from).min(item.duration))
        }
        None => (false, "none", 0.0, MAX_TRANSITION),
    };
    state.set_animation(AnimationData {
        in_kind: windows.in_kind.as_str().into(),
        in_duration: anim_in.map(|a| a.duration).unwrap_or(0.0) as f32,
        out_kind: windows.out_kind.as_str().into(),
        out_duration: anim_out.map(|a| a.duration).unwrap_or(0.0) as f32,
        max: span.duration as f32,
        has_seam,
        seam_kind: seam_kind.into(),
        seam_duration: seam_duration as f32,
        seam_max: seam_max as f32,
    });
}

fn sync_project(editor: &Editor, state: &InspectorState) {
    let project = &editor.project;
    let preset = ASPECT_PRESETS.iter().find(|p| p.width == project.width && p.height == project.height);
    let mut aspects: Vec<Choice> = ASPECT_PRESETS
        .iter()
        .map(|p| Choice { value: p.id.into(), label: format!("{} - {}×{}", p.label, p.width, p.height).into(), hint: p.hint.into(), ..Default::default() })
        .collect();
    if preset.is_none() {
        aspects.insert(0, choice("custom", &format!("{}×{}", project.width, project.height)));
    }
    if state.get_aspects().row_count() != aspects.len() || state.get_aspects().row_data(0) != aspects.first().cloned() {
        state.set_aspects(ModelRc::new(VecModel::from(aspects)));
    }
    let (background, background_hex) = color_pair(&project.background, "#000000");
    state.set_project(ProjectData {
        name: project.name.clone().into(),
        aspect: preset.map(|p| p.id).unwrap_or("custom").into(),
        fps: project.fps.to_string().into(),
        background,
        background_hex,
    });
    let clips: usize = project.tracks.iter().map(|t| t.items.len()).sum();
    let stats = vec![
        Stat { label: "Length".into(), value: format::duration(editor.playback_end()).into() },
        Stat { label: "Clips".into(), value: clips.to_string().into() },
        Stat { label: "Tracks".into(), value: project.tracks.len().to_string().into() },
        Stat { label: "Media files".into(), value: project.assets.len().to_string().into() },
    ];
    state.set_stats(ModelRc::new(VecModel::from(stats)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        assert_eq!(hex_of(parse_hex("#FFE14D").unwrap()), "#ffe14d");
        assert_eq!(hex_of(parse_hex("abc").unwrap()), "#aabbcc");
        assert!(parse_hex("#12").is_none());
        assert!(parse_hex("zzzzzz").is_none());
    }
}
