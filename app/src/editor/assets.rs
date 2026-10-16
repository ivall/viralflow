//! The asset panels (media library, text styles, stickers, effects,
//! transitions) and dragging from them onto the timeline or the preview.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use slint::{ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};
use vf_core::edit::{Editor, Placement};
use vf_core::engine::effects::EFFECTS;
use vf_core::engine::transitions::{track_seams, TRANSITIONS};
use vf_core::model::*;
use vf_core::presets::{media_item, shape_item, sticker_item, text_item, text_preset, PresetCategory, SHAPE_PRESETS, STICKER_GROUPS, TEXT_PRESETS};
use vf_render::TextEngine;

use super::inspector::parse_hex;
use super::timeline::{DropKind, DropTarget};
use super::EditorController;
use crate::{format, media, AssetTile, AssetsState, Choice, DragState, EditorState, PresetCard, ShapeCard, StickerGroup};

/// A press turns into a drag after this many pixels.
const DRAG_THRESHOLD: f64 = 5.0;

pub(super) enum Payload {
    Asset(MediaAsset),
    Item(Box<TimelineItem>),
    Effect(EffectKind),
    Transition(TransitionKind),
    Files(Vec<PathBuf>),
}

impl Payload {
    fn drop_kind(&self, project: &Project) -> DropKind {
        match self {
            Payload::Asset(asset) => DropKind::Media { kind: media_item(asset, project).kind, duration: media_item(asset, project).duration },
            Payload::Item(item) => DropKind::Media { kind: item.kind, duration: item.duration },
            Payload::Effect(_) => DropKind::Effect,
            Payload::Transition(_) => DropKind::Transition,
            Payload::Files(_) => DropKind::Media { kind: ItemKind::Video, duration: 5.0 },
        }
    }

    /// Effects and transitions only land on clips and cuts.
    fn fits_preview(&self) -> bool {
        matches!(self, Payload::Asset(_) | Payload::Item(_) | Payload::Files(_))
    }
}

pub(super) struct Drag {
    payload: Payload,
    label: String,
    origin: (f64, f64),
    pub(super) started: bool,
    /// Target element and the pointer in its coordinates.
    over: Option<(String, f64, f64)>,
}

/// What the media list last showed.
#[derive(Default, PartialEq)]
pub(super) struct ListKey {
    revision: u64,
    filter: SharedString,
    query: SharedString,
    category: SharedString,
    thumbs: u64,
    selection: Vec<String>,
}

impl EditorController {
    pub(super) fn bind_assets(self: &Rc<Self>, state: &AssetsState, drag: &DragState) {
        macro_rules! on {
            ($target:ident, $method:ident, |$this:ident $(, $arg:ident)*| $body:expr) => {{
                let weak = Rc::downgrade(self);
                $target.$method(move |$($arg),*| {
                    if let Some($this) = weak.upgrade() {
                        $body
                    }
                });
            }};
        }
        on!(state, on_press, |this, kind, id, x, y| this.press(&kind, &id, x as f64, y as f64));
        on!(state, on_drag_to, |this, x, y| this.drag_to(x as f64, y as f64));
        on!(state, on_release, |this, x, y| this.release(x as f64, y as f64));
        on!(state, on_add, |this, kind, id| this.add_from_panel(&kind, &id));
        on!(state, on_import, |this| {
            this.import_media(false);
        });
        on!(state, on_filter_changed, |this| this.sync_ui());
        on!(state, on_asset_menu, |this, action, id| this.asset_menu(&action, &id));
        on!(state, on_action, |this, action| this.panel_action(&action));
        on!(drag, on_over, |this, target, x, y| this.drag_over(&target, x as f64, y as f64));
        on!(drag, on_leave, |this, target| this.drag_leave(&target));

        state.set_text_presets(ModelRc::new(VecModel::from(text_preset_cards(&self.text))));
        let categories: Vec<SharedString> = std::iter::once("All").chain(PresetCategory::ALL.iter().map(|c| c.label())).map(SharedString::from).collect();
        state.set_text_categories(ModelRc::new(VecModel::from(categories)));
        let shapes: Vec<ShapeCard> = SHAPE_PRESETS
            .iter()
            .map(|p| ShapeCard { kind: p.kind.as_str().into(), label: p.label.into(), color: parse_hex(p.color).unwrap_or_default() })
            .collect();
        state.set_shapes(ModelRc::new(VecModel::from(shapes)));
        let stickers: Vec<StickerGroup> = STICKER_GROUPS
            .iter()
            .map(|(title, emoji)| StickerGroup {
                title: (*title).into(),
                emoji: ModelRc::new(VecModel::from(emoji.iter().map(|e| SharedString::from(*e)).collect::<Vec<_>>())),
            })
            .collect();
        state.set_stickers(ModelRc::new(VecModel::from(stickers)));
        let effects: Vec<Choice> = EFFECTS.iter().filter(|e| e.kind != EffectKind::None).map(|e| choice(e.kind.as_str(), e.label)).collect();
        state.set_effects(ModelRc::new(VecModel::from(effects)));
        let transitions: Vec<Choice> = TRANSITIONS.iter().map(|t| choice(t.kind.as_str(), t.label)).collect();
        state.set_transitions(ModelRc::new(VecModel::from(transitions)));
    }

    // ------------------------------------------------------------ dragging

    fn payload(&self, kind: &str, id: &str) -> Option<(Payload, String)> {
        match kind {
            "asset" => {
                let asset = self.with(|s| s.editor.project.assets.iter().find(|a| a.id == id).cloned()).flatten()?;
                if self.with(|s| s.offline.contains(&asset.id)).unwrap_or(false) {
                    return None;
                }
                let label = asset.name.clone();
                Some((Payload::Asset(asset), label))
            }
            "text" => {
                let preset = text_preset(id);
                let sample = preset.map(|p| p.sample).unwrap_or("Your text");
                Some((Payload::Item(Box::new(text_item(sample, preset))), preset.map(|p| p.label).unwrap_or("Text").to_string()))
            }
            "sticker" => Some((Payload::Item(Box::new(sticker_item(id))), id.to_string())),
            "shape" => {
                let kind = ShapeKind::parse(id)?;
                let label = SHAPE_PRESETS.iter().find(|p| p.kind == kind).map(|p| p.label).unwrap_or("Shape");
                Some((Payload::Item(Box::new(shape_item(kind))), label.to_string()))
            }
            "effect" => {
                let kind = EffectKind::parse(id)?;
                Some((Payload::Effect(kind), EFFECTS.iter().find(|e| e.kind == kind).map(|e| e.label).unwrap_or("Effect").to_string()))
            }
            "transition" => {
                let kind = TransitionKind::parse(id)?;
                Some((Payload::Transition(kind), TRANSITIONS.iter().find(|t| t.kind == kind).map(|t| t.label).unwrap_or("Transition").to_string()))
            }
            _ => None,
        }
    }

    fn press(&self, kind: &str, id: &str, x: f64, y: f64) {
        let Some((payload, label)) = self.payload(kind, id) else { return };
        self.with(|s| s.drag = Some(Drag { payload, label, origin: (x, y), started: false, over: None }));
    }

    /// Files dropped on the window from the system go in at the playhead.
    pub(super) fn install_file_drop(self: &Rc<Self>) {
        use slint::winit_030::{winit::event::WindowEvent, EventResult, WinitWindowAccessor};
        let Some(window) = self.context.window() else { return };
        let weak = Rc::downgrade(self);
        // One event arrives per file; they are collected into one import.
        let pending: Rc<std::cell::RefCell<Vec<PathBuf>>> = Rc::default();
        window.window().on_winit_window_event(move |_, event| {
            if let WindowEvent::DroppedFile(path) = event {
                let first = pending.borrow().is_empty();
                pending.borrow_mut().push(path.clone());
                if first {
                    let (pending, weak) = (pending.clone(), weak.clone());
                    slint::Timer::single_shot(std::time::Duration::from_millis(60), move || {
                        let paths = pending.take();
                        if let Some(this) = weak.upgrade() {
                            this.files_dropped(paths);
                        }
                    });
                }
            }
            EventResult::Propagate
        });
    }

    fn files_dropped(&self, paths: Vec<PathBuf>) {
        let Some(time) = self.with(|s| s.editor.time) else { return };
        self.place(Payload::Files(paths), Placement { start: Some(time), ..Default::default() });
    }

    fn drag_to(&self, x: f64, y: f64) {
        let Some(window) = self.context.window() else { return };
        let started = self.with(|s| {
            let drag = s.drag.as_mut()?;
            if !drag.started && (x - drag.origin.0).hypot(y - drag.origin.1) >= DRAG_THRESHOLD {
                drag.started = true;
            }
            drag.started.then(|| drag.label.clone())
        });
        let Some(Some(label)) = started else { return };
        let state = window.global::<DragState>();
        state.set_label(label.into());
        state.set_x(x as f32);
        state.set_y(y as f32);
        state.set_active(true);
    }

    fn drag_over(&self, target: &str, x: f64, y: f64) {
        let accepted = self.with(|s| {
            let drag = s.drag.as_mut().filter(|d| d.started)?;
            drag.over = Some((target.to_string(), x, y));
            let kind = drag.payload.drop_kind(&s.editor.project);
            match target {
                "timeline" => {
                    let hit = s.timeline.drop_target(&s.editor, kind, x, y);
                    let duration = match kind {
                        DropKind::Media { duration, .. } => duration,
                        _ => 0.0,
                    };
                    s.timeline.show_drop(&s.editor, hit.as_ref(), duration);
                    Some(hit.is_some())
                }
                "preview" => Some(drag.payload.fits_preview()),
                _ => Some(false),
            }
        });
        let accepted = accepted.flatten().unwrap_or(false);
        if let Some(window) = self.context.window() {
            window.global::<DragState>().set_accepted(accepted);
            window.global::<EditorState>().set_drop_preview(accepted && target == "preview");
        }
        self.sync_timeline();
    }

    fn drag_leave(&self, target: &str) {
        self.with(|s| {
            if let Some(drag) = s.drag.as_mut() {
                if drag.over.as_ref().is_some_and(|(t, ..)| t == target) {
                    drag.over = None;
                }
            }
            if target == "timeline" {
                s.timeline.show_drop(&s.editor, None, 0.0);
            }
        });
        if let Some(window) = self.context.window() {
            window.global::<DragState>().set_accepted(false);
            if target == "preview" {
                window.global::<EditorState>().set_drop_preview(false);
            }
        }
        self.sync_timeline();
    }

    /// Ends a drag (Escape, or the pointer left the window).
    pub(super) fn cancel_drag(&self) -> bool {
        let had = self.with(|s| {
            s.timeline.show_drop(&s.editor, None, 0.0);
            s.drag.take().is_some_and(|d| d.started)
        });
        self.reset_drag_ui();
        self.sync_ui();
        had.unwrap_or(false)
    }

    fn reset_drag_ui(&self) {
        if let Some(window) = self.context.window() {
            let state = window.global::<DragState>();
            state.set_active(false);
            state.set_accepted(false);
            window.global::<EditorState>().set_drop_preview(false);
        }
    }

    fn release(&self, _x: f64, _y: f64) {
        let drag = self.with(|s| {
            s.timeline.show_drop(&s.editor, None, 0.0);
            s.drag.take()
        });
        self.reset_drag_ui();
        let Some(Some(drag)) = drag else {
            self.sync_ui();
            return;
        };
        if !drag.started {
            self.click(drag.payload);
            return;
        }
        match drag.over {
            Some((target, x, y)) => self.drop(drag.payload, &target, x, y),
            None => self.sync_ui(),
        }
    }

    fn drop(&self, payload: Payload, target: &str, x: f64, y: f64) {
        if target == "preview" {
            let time = self.with(|s| s.editor.time).unwrap_or(0.0);
            self.place(payload, Placement { start: Some(time), ..Default::default() });
            return;
        }
        let hit = self.with(|s| s.timeline.drop_target(&s.editor, payload.drop_kind(&s.editor.project), x, y)).flatten();
        let Some(hit) = hit else {
            self.sync_ui();
            return;
        };
        match (hit, payload) {
            (DropTarget::Seam(id), Payload::Transition(kind)) => self.edit(|e| {
                e.set_transition(&id, kind, None, true);
                e.select(Some(&id), false);
            }),
            (DropTarget::Clip(id), Payload::Effect(kind)) => self.edit(|e| {
                e.set_effect(std::slice::from_ref(&id), kind, None);
                e.select(Some(&id), false);
            }),
            (DropTarget::Track { track_id, start }, payload) => {
                self.place(payload, Placement { track_id: Some(track_id), start: Some(start), new_track_index: None })
            }
            (DropTarget::New { index, start }, payload) => self.place(payload, Placement { start: Some(start), new_track_index: Some(index), track_id: None }),
            _ => self.sync_ui(),
        }
    }

    /// Adds dragged media or a new clip.
    fn place(&self, payload: Payload, placement: Placement) {
        match payload {
            Payload::Asset(asset) => self.edit(|e| {
                e.add_asset(&asset, placement);
            }),
            Payload::Item(item) => self.edit(|e| {
                e.add_item(*item, placement);
            }),
            Payload::Files(paths) => {
                let assets = self.import_paths(&paths, false);
                // Mixed kinds cannot share one track: they go to the right tracks at that time.
                let track_kind =
                    placement.track_id.as_ref().and_then(|id| self.with(|s| s.editor.project.tracks.iter().find(|t| &t.id == id).map(|t| t.kind)).flatten());
                let same = track_kind.is_some_and(|kind| assets.iter().all(|a| media_kind_track(a.kind) == kind));
                let placement = if same { placement } else { Placement { start: placement.start, ..Default::default() } };
                self.edit(|e| {
                    e.add_assets_sequence(&assets, placement);
                });
            }
            Payload::Effect(_) | Payload::Transition(_) => self.sync_ui(),
        }
    }

    /// A press without a drag.
    fn click(&self, payload: Payload) {
        match payload {
            // Media tiles add on double click (or their + button), like the old library.
            Payload::Asset(_) | Payload::Files(_) => self.sync_ui(),
            Payload::Item(item) => {
                let is_text = item.kind == ItemKind::Text;
                self.edit(|e| {
                    e.add_item(*item, Placement::default());
                });
                if is_text {
                    if let Some(window) = self.context.window() {
                        window.global::<crate::InspectorState>().set_tab("main".into());
                    }
                }
            }
            Payload::Effect(kind) => self.apply_effect(kind),
            Payload::Transition(kind) => self.apply_transition(kind),
        }
    }

    fn add_from_panel(&self, kind: &str, id: &str) {
        match self.payload(kind, id) {
            Some((Payload::Asset(asset), _)) => self.edit(|e| {
                e.add_asset(&asset, Placement::default());
            }),
            Some((payload, _)) => self.click(payload),
            None => {}
        }
    }

    // ------------------------------------------------------------ effects & transitions

    fn apply_effect(&self, kind: EffectKind) {
        let targets = self.with(|s| effect_targets(&s.editor)).unwrap_or_default();
        if targets.is_empty() {
            self.context.toasts.show("Select a video, photo or text clip first");
            return;
        }
        self.edit(|e| e.set_effect(&targets, kind, None));
    }

    fn apply_transition(&self, kind: TransitionKind) {
        let seam = self.with(|s| selected_seam(&s.editor)).flatten();
        let Some(seam) = seam else {
            self.context.toasts.info("Select a clip that touches another clip", "Or drag a transition onto the cut between two clips.");
            return;
        };
        self.edit(|e| e.set_transition(&seam.to, kind, None, true));
    }

    fn panel_action(&self, action: &str) {
        match action {
            "remove-effect" => self.apply_effect(EffectKind::None),
            "remove-transition" => self.apply_transition(TransitionKind::None),
            "transition-all" => {
                let seam = self.with(|s| selected_seam(&s.editor)).flatten();
                if let Some(seam) = seam.filter(|seam| seam.kind != TransitionKind::None) {
                    self.edit(|e| {
                        if let Some(track) = e.find(&seam.to).map(|(t, _)| e.project.tracks[t].id.clone()) {
                            e.set_transition_for_track(&track, seam.kind, seam.duration);
                        }
                    });
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ the media library

    fn asset_menu(&self, action: &str, id: &str) {
        let Some(asset) = self.with(|s| s.editor.project.assets.iter().find(|a| a.id == id).cloned()).flatten() else { return };
        match action {
            "add" => self.add_from_panel("asset", id),
            "reveal" => reveal(Path::new(&asset.path)),
            "remove" => self.edit(|e| e.remove_asset(id)),
            "relink" => self.relink(&asset),
            _ => {}
        }
    }

    fn relink(&self, asset: &MediaAsset) {
        let Some(path) = rfd::FileDialog::new().set_title(format!("Find {}", asset.name)).add_filter("Media", media::MEDIA_EXTENSIONS).pick_file() else {
            return;
        };
        let imported = media::import(std::slice::from_ref(&path), &[]);
        let Some(found) = imported.assets.into_iter().next() else {
            self.context.toasts.error("This file could not be used", &imported.failures.join("\n"));
            return;
        };
        if found.kind != asset.kind {
            self.context.toasts.error("This file is a different kind of media", &format!("{} needs a {} file.", asset.name, asset.kind));
            return;
        }
        let id = asset.id.clone();
        self.edit(|e| {
            e.begin_change();
            e.update_asset(&id, |a| {
                a.path = found.path.clone();
                a.duration = found.duration.or(a.duration);
                a.width = found.width.or(a.width);
                a.height = found.height.or(a.height);
                a.fps = found.fps.or(a.fps);
                a.has_audio = found.has_audio.or(a.has_audio);
                a.size = found.size;
                a.preview_path = None;
            });
        });
        self.check_media();
        self.context.toasts.success("File relinked", &found.name);
    }
}

// ------------------------------------------------------------ sync

pub(super) fn sync(editor: &Editor, overviews: &mut super::overview::Overviews, offline: &HashSet<String>, state: &AssetsState, last: &mut ListKey) {
    let key = ListKey {
        revision: editor.revision(),
        filter: state.get_filter(),
        query: state.get_query(),
        category: state.get_text_category(),
        thumbs: overviews.generation(),
        selection: editor.selection.clone(),
    };
    if *last == key {
        return;
    }
    let project = &editor.project;
    let filter = key.filter.as_str();
    let query = key.query.trim().to_lowercase();
    let count = |kind: Option<MediaKind>| project.assets.iter().filter(|a| kind.is_none_or(|k| a.kind == k)).count();
    let categories = vec![
        Choice { value: "all".into(), label: "All".into(), hint: count(None).to_string().into(), ..Default::default() },
        Choice { value: "video".into(), label: "Video".into(), hint: count(Some(MediaKind::Video)).to_string().into(), ..Default::default() },
        Choice { value: "image".into(), label: "Photos".into(), hint: count(Some(MediaKind::Image)).to_string().into(), ..Default::default() },
        Choice { value: "audio".into(), label: "Audio".into(), hint: count(Some(MediaKind::Audio)).to_string().into(), ..Default::default() },
    ];
    if state.get_categories().iter().ne(categories.iter().cloned()) {
        state.set_categories(ModelRc::new(VecModel::from(categories)));
    }

    if key.revision != last.revision || key.filter != last.filter || key.query != last.query || key.thumbs != last.thumbs {
        let used: HashSet<&str> = project.items().filter_map(|item| item.asset_id.as_deref()).collect();
        let tiles: Vec<AssetTile> = project
            .assets
            .iter()
            .rev()
            .filter(|a| filter == "all" || a.kind.as_str() == filter)
            .filter(|a| query.is_empty() || a.name.to_lowercase().contains(&query))
            .map(|asset| {
                let missing = offline.contains(&asset.id);
                let thumb = if missing || asset.kind == MediaKind::Audio { None } else { overviews.thumbnail(asset) };
                let ratio = match (&thumb, asset.width, asset.height) {
                    (Some(image), _, _) if image.size().height > 0 => image.size().width as f32 / image.size().height as f32,
                    (_, Some(w), Some(h)) if h > 0.0 => (w / h) as f32,
                    _ => 16.0 / 9.0,
                };
                AssetTile {
                    id: asset.id.clone().into(),
                    name: asset.name.clone().into(),
                    kind: match asset.kind {
                        MediaKind::Video => 0,
                        MediaKind::Image => 1,
                        MediaKind::Audio => 2,
                    },
                    thumb: thumb.unwrap_or_default(),
                    duration: asset.duration.filter(|_| asset.kind != MediaKind::Image).map(format::duration).unwrap_or_default().into(),
                    used: used.contains(asset.id.as_str()),
                    missing,
                    ratio,
                }
            })
            .collect();
        let label = match filter {
            "video" => "video",
            "image" => "photos",
            "audio" => "audio",
            _ => "media",
        };
        state.set_has_media(!project.assets.is_empty());
        if project.assets.is_empty() {
            state.set_empty_title("No media yet".into());
            state.set_empty_detail("Import files or drag them from your computer. They are used from their current location - nothing is copied.".into());
        } else if !query.is_empty() {
            state.set_empty_title("Nothing found".into());
            state.set_empty_detail(format!("No {label} matches “{}”.", key.query.trim()).into());
        } else {
            state.set_empty_title(format!("No {label} yet").into());
            state.set_empty_detail("".into());
        }
        state.set_assets(ModelRc::new(VecModel::from(tiles)));
    }

    if key.category != last.category {
        let category = key.category.as_str();
        let all = text_preset_cards_cache();
        let cards: Vec<PresetCard> =
            TEXT_PRESETS.iter().zip(all).filter(|(preset, _)| category == "All" || preset.category.label() == category).map(|(_, card)| card.clone()).collect();
        state.set_text_presets(ModelRc::new(VecModel::from(cards)));
    }

    let targets = effect_targets(editor);
    let current = targets.first().and_then(|id| editor.item(id)).map(|item| item.effect()).unwrap_or_default();
    state.set_effect_targets(targets.len() as i32);
    state.set_current_effect(current.as_str().into());
    let seam = selected_seam(editor);
    state.set_has_seam(seam.is_some());
    state.set_seam_kind(seam.as_ref().map(|s| s.kind.as_str()).unwrap_or("none").into());
    let label = seam
        .as_ref()
        .filter(|s| s.kind != TransitionKind::None)
        .map(|s| format!("{} · {:.1}s", TRANSITIONS.iter().find(|t| t.kind == s.kind).map(|t| t.label).unwrap_or(""), s.duration))
        .unwrap_or_default();
    state.set_seam_label(label.into());
    state.set_can_apply_all(seam.is_some_and(|s| s.kind != TransitionKind::None));
    *last = key;
}

fn effect_targets(editor: &Editor) -> Vec<String> {
    editor
        .selected_items()
        .into_iter()
        .filter(|item| matches!(item.kind, ItemKind::Video | ItemKind::Image | ItemKind::Text))
        .map(|item| item.id.clone())
        .collect()
}

/// The cut before the selected clip, or else the one after it.
fn selected_seam(editor: &Editor) -> Option<vf_core::engine::transitions::Seam> {
    let item = editor.selected_item()?;
    let (t, _) = editor.find(&item.id)?;
    let seams = track_seams(&editor.project.tracks[t], editor.fps());
    seams.iter().find(|s| s.to == item.id).or_else(|| seams.iter().find(|s| s.from == item.id)).cloned()
}

fn media_kind_track(kind: MediaKind) -> TrackKind {
    match kind {
        MediaKind::Audio => TrackKind::Audio,
        _ => TrackKind::Video,
    }
}

fn choice(value: &str, label: &str) -> Choice {
    Choice { value: value.into(), label: label.into(), ..Default::default() }
}

thread_local! {
    static PRESET_CARDS: std::cell::RefCell<Vec<PresetCard>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn text_preset_cards_cache() -> Vec<PresetCard> {
    PRESET_CARDS.with(|cards| cards.borrow().clone())
}

/// Each text style drawn with its sample words (rendered once, shared by the
/// text panel and the inspector's style presets).
pub(super) fn text_preset_previews(text: &TextEngine) -> Vec<PresetCard> {
    let cached = text_preset_cards_cache();
    if cached.is_empty() {
        text_preset_cards(text)
    } else {
        cached
    }
}

/// Renders every text style with its sample words, like the cards of the old panel.
fn text_preset_cards(text: &TextEngine) -> Vec<PresetCard> {
    let cards: Vec<PresetCard> = TEXT_PRESETS
        .iter()
        .map(|preset| {
            let mut style = preset.text_style();
            style.content = preset.sample.to_string();
            if style.highlight_mode != HighlightMode::None {
                style.highlight_index = 1;
            }
            // Long samples are drawn a little smaller so they fit the card.
            let length = preset.sample.chars().count();
            let size = if length > 20 {
                13.0
            } else if length > 12 {
                15.0
            } else {
                18.0
            };
            let dpi = 2.0;
            let scale = size / style.size.max(1.0);
            let wrap = 150.0 / scale;
            let canvas = wrap / (style.max_width.max(1.0) / 100.0);
            let (width, height, pixels) = text.rasterize(&style, canvas, scale * dpi);
            PresetCard { id: preset.id.into(), label: preset.label.into(), preview: premultiplied_image(width, height, &pixels) }
        })
        .collect();
    PRESET_CARDS.with(|cache| *cache.borrow_mut() = cards.clone());
    cards
}

fn premultiplied_image(width: u32, height: u32, pixels: &[u8]) -> Image {
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(width.max(1), height.max(1));
    let out = buffer.make_mut_bytes();
    for (dst, src) in out.chunks_exact_mut(4).zip(pixels.chunks_exact(4)) {
        let a = src[3] as u32;
        if a == 0 {
            dst.copy_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        for c in 0..3 {
            dst[c] = ((src[c] as u32 * 255 + a / 2) / a).min(255) as u8;
        }
        dst[3] = src[3];
    }
    Image::from_rgba8(buffer)
}

/// Shows a file in the system file manager.
pub(super) fn reveal(path: &Path) {
    // Explorer wants the path quoted after the comma, which normal argument
    // quoting does not do: written out as it is.
    #[cfg(target_os = "windows")]
    let result = {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn()
    };
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn();
    if let Err(error) = result {
        log::warn!("could not reveal {}: {error}", path.display());
    }
}

/// Opens a file with its default application.
pub(super) fn open_path(path: &Path) {
    // Explorer opens a file with its default app; `cmd /C start` would read
    // characters such as & in the file name as commands.
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(path).spawn();
    if let Err(error) = result {
        log::warn!("could not open {}: {error}", path.display());
    }
}

#[cfg(test)]
mod preset_preview_tests {
    use super::*;

    #[test]
    fn every_style_preset_card_shows_its_words() {
        let text = TextEngine::new();
        for preset in TEXT_PRESETS {
            let mut style = preset.text_style();
            style.content = preset.sample.to_string();
            let (width, height, pixels) = text.rasterize(&style, 600.0, 1.0);
            let ink = pixels.chunks_exact(4).filter(|p| p[3] > 128).count();
            assert!(width > 0 && height > 0 && ink > 1000, "{}: {width}x{height} with {ink} opaque pixels", preset.id);
        }
    }
}
