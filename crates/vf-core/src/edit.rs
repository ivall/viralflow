//! Every change to an open project goes through `Editor`: it keeps tracks free
//! of overlaps, transitions on real cuts, keyframes inside their clips, and an
//! undo history of whole-document snapshots (sharing what did not change).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use crate::engine::animation::{caption_run, default_in_duration, default_out_duration, motion_span, MotionSpan};
use crate::engine::effects::{effect_window, MIN_EFFECT};
use crate::engine::keyframes::{
    adjacent_keyframe, normalize_transform, patch_keyframe_at, retime_keyframes, set_keyframe_easing, sorted_keyframes, toggle_keyframe, trim_keyframes,
};
use crate::engine::math::{clamp, uid};
use crate::engine::scene::content_end;
use crate::engine::transitions::{repair_transitions, transition_duration, DEFAULT_TRANSITION};
use crate::model::*;
use crate::presets::{create_track, item_name, media_item, preset_text_style, TextPreset};
use crate::timeline::{place_selection, place_span, repair_layout, ripple_resize, spans_of, trim_span, Edge, SelectionSpan, Span, HORIZON, MIN_CLIP_DURATION};

const HISTORY_LIMIT: usize = 120;
/// Effects one clip can carry.
pub const MAX_EFFECTS: usize = 8;

/// Effects go on pictures and words, not on sound or shapes.
pub fn can_have_effects(item: &TimelineItem) -> bool {
    !matches!(item.kind, ItemKind::Audio | ItemKind::Shape)
}

/// Where a new clip should go.
#[derive(Debug, Clone, Default)]
pub struct Placement {
    pub track_id: Option<String>,
    pub start: Option<f64>,
    /// Always create a fresh track at this index (dropping between tracks).
    pub new_track_index: Option<usize>,
}

/// A partial transform; `None` keeps the current value.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TransformPatch {
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub scale: Option<f64>,
    pub rotation: Option<f64>,
    pub opacity: Option<f64>,
}

impl TransformPatch {
    pub fn apply(&self, t: Transform) -> Transform {
        Transform {
            x: self.x.unwrap_or(t.x),
            y: self.y.unwrap_or(t.y),
            scale: self.scale.unwrap_or(t.scale),
            rotation: self.rotation.unwrap_or(t.rotation),
            opacity: self.opacity.unwrap_or(t.opacity),
        }
    }
}

/// Clip state when a trim gesture began.
#[derive(Debug, Clone)]
pub struct TrimOrigin {
    pub start: f64,
    pub duration: f64,
    pub source_in: f64,
    /// Each effect's range, by effect id.
    pub effect_ranges: Vec<(String, Option<EffectRange>)>,
}

impl TrimOrigin {
    pub fn of(item: &TimelineItem) -> Self {
        TrimOrigin {
            start: item.start,
            duration: item.duration,
            source_in: item.source_in(),
            effect_ranges: item.effects.iter().map(|e| (e.id.clone(), e.range)).collect(),
        }
    }
}

/// A clip's effects split at `offset` seconds into it: the ones (or the parts
/// of them) before the cut, then the ones after, relative to each half.
fn split_effects(effects: &[ClipEffect], duration: f64, offset: f64) -> (Vec<ClipEffect>, Vec<ClipEffect>) {
    let (mut left, mut right) = (Vec::new(), Vec::new());
    for effect in effects {
        if effect.range.is_none() {
            left.push(effect.clone());
            right.push(ClipEffect { id: uid("fx"), ..effect.clone() });
            continue;
        }
        let window = effect_window(duration, effect.range);
        let left_end = offset.min(window.start + window.duration);
        if left_end - window.start >= MIN_EFFECT {
            left.push(ClipEffect { range: Some(EffectRange { start: window.start, duration: left_end - window.start }), ..effect.clone() });
        }
        let right_start = (window.start - offset).max(0.0);
        let right_end = window.start + window.duration - offset;
        if right_end - right_start >= MIN_EFFECT {
            right.push(ClipEffect { id: uid("fx"), range: Some(EffectRange { start: right_start, duration: right_end - right_start }), ..effect.clone() });
        }
    }
    (left, right)
}

/// The range to store: None when it covers the whole clip.
fn stored_range(duration: f64, window: EffectRange) -> Option<EffectRange> {
    (!(window.start <= 1e-6 && window.duration >= duration - 1e-6)).then_some(window)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    Locked,
    Hidden,
    Muted,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnimationChoice {
    In(AnimationInKind),
    Out(AnimationOutKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    In,
    Out,
}

#[derive(Debug, Clone, Default)]
pub struct TrackPatch {
    pub name: Option<String>,
    pub locked: Option<bool>,
    pub muted: Option<bool>,
    pub hidden: Option<bool>,
}

pub struct CaptionOptions {
    pub source_id: String,
    /// Timeline time of the source's first second.
    pub offset: f64,
    /// Playback speed of the source clip.
    pub speed: f64,
    pub style: TextStyle,
    /// Vertical position in % of the canvas.
    pub y: f64,
}

/// One undo step: the document and what was selected at that moment, so undo
/// also brings back the selection (an undone duplicate selects the original).
#[derive(Clone)]
struct Step {
    document: Snapshot,
    selection: Vec<String>,
    primary: Option<String>,
}

/// A document in the history. Tracks, the asset list and the settings are
/// shared with the neighbouring snapshot while unchanged, so a step holds
/// only what its change touched (moving one caption copies one track).
#[derive(Clone)]
struct Snapshot {
    settings: Arc<Project>,
    tracks: Vec<Arc<Track>>,
    assets: Arc<Vec<MediaAsset>>,
}

impl Snapshot {
    fn capture(project: &Project, near: Option<&Snapshot>) -> Self {
        let tracks = project
            .tracks
            .iter()
            .map(|track| near.and_then(|n| n.tracks.iter().find(|t| t.id == track.id && ***t == *track)).cloned().unwrap_or_else(|| Arc::new(track.clone())))
            .collect();
        let assets = near.filter(|n| *n.assets == project.assets).map(|n| n.assets.clone()).unwrap_or_else(|| Arc::new(project.assets.clone()));
        let settings = Project {
            version: project.version,
            id: project.id.clone(),
            name: project.name.clone(),
            created_at: project.created_at.clone(),
            updated_at: project.updated_at.clone(),
            width: project.width,
            height: project.height,
            fps: project.fps,
            background: project.background.clone(),
            tracks: Vec::new(),
            assets: Vec::new(),
            export: project.export.clone(),
            origin: project.origin.clone(),
            extra: project.extra.clone(),
        };
        let settings = near.filter(|n| *n.settings == settings).map(|n| n.settings.clone()).unwrap_or_else(|| Arc::new(settings));
        Snapshot { settings, tracks, assets }
    }

    fn project(&self) -> Project {
        Project { tracks: self.tracks.iter().map(|t| (**t).clone()).collect(), assets: (*self.assets).clone(), ..(*self.settings).clone() }
    }
}

pub struct Editor {
    pub project: Project,
    /// Playhead in seconds.
    pub time: f64,
    pub selection: Vec<String>,
    pub primary: Option<String>,
    past: Vec<Step>,
    future: Vec<Step>,
    clipboard: Vec<TimelineItem>,
    revision: u64,
    last_nudge: Option<Instant>,
}

impl Editor {
    /// Opens a (migrated) project and repairs its tracks.
    pub fn new(project: Project) -> Self {
        let mut editor =
            Editor { project, time: 0.0, selection: vec![], primary: None, past: vec![], future: vec![], clipboard: vec![], revision: 0, last_nudge: None };
        for index in 0..editor.project.tracks.len() {
            editor.repair_track(index);
        }
        editor
    }

    // ------------------------------------------------------------ queries

    pub fn fps(&self) -> u32 {
        self.project.fps.max(1)
    }

    pub fn frame(&self) -> f64 {
        1.0 / self.fps() as f64
    }

    pub fn keyframe_tolerance(&self) -> f64 {
        0.5 / self.fps() as f64
    }

    /// Increments on every change; the UI and autosave watch it.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn playback_end(&self) -> f64 {
        content_end(&self.project)
    }

    /// Visible length of the timeline: the content plus room to grow.
    pub fn timeline_length(&self) -> f64 {
        (self.playback_end() * 1.25 + 10.0).max(30.0)
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    pub fn has_clipboard(&self) -> bool {
        !self.clipboard.is_empty()
    }

    pub fn find(&self, id: &str) -> Option<(usize, usize)> {
        self.project.locate(id)
    }

    /// Location of an item that may be changed (neither it nor its track is locked).
    pub fn editable(&self, id: &str) -> Option<(usize, usize)> {
        let (t, i) = self.find(id)?;
        let track = &self.project.tracks[t];
        (!track.locked && !track.items[i].locked).then_some((t, i))
    }

    pub fn item(&self, id: &str) -> Option<&TimelineItem> {
        self.project.item(id)
    }

    fn at(&mut self, (t, i): (usize, usize)) -> &mut TimelineItem {
        &mut self.project.tracks[t].items[i]
    }

    pub fn selected_item(&self) -> Option<&TimelineItem> {
        self.primary.as_deref().and_then(|id| self.item(id))
    }

    pub fn selected_track(&self) -> Option<usize> {
        self.primary.as_deref().and_then(|id| self.find(id)).map(|(t, _)| t)
    }

    pub fn selected_items(&self) -> Vec<&TimelineItem> {
        self.selection.iter().filter_map(|id| self.item(id)).collect()
    }

    fn editable_selection(&self) -> Vec<(usize, usize)> {
        self.selection.iter().filter_map(|id| self.editable(id)).collect()
    }

    pub fn asset(&self, id: Option<&str>) -> Option<&MediaAsset> {
        self.project.asset(id)
    }

    // ------------------------------------------------------------ history

    fn snapshot(&mut self) {
        if self.past.len() >= HISTORY_LIMIT {
            self.past.remove(0);
        }
        let step = self.step(self.past.last().map(|s| &s.document));
        self.past.push(step);
        self.future.clear();
        self.last_nudge = None;
    }

    /// Call before a continuous change (drag, slider) so the gesture is one undo step.
    pub fn begin_change(&mut self) {
        self.snapshot();
    }

    fn touched(&mut self) {
        self.project.updated_at = crate::presets::now_iso();
        self.revision += 1;
    }

    fn restore(&mut self, project: Project) {
        self.project = project;
        let selection = self.selection.clone();
        self.select_many(&selection);
        self.touched();
    }

    /// The current state, sharing unchanged parts with `near` (its neighbour in the history).
    fn step(&self, near: Option<&Snapshot>) -> Step {
        Step { document: Snapshot::capture(&self.project, near), selection: self.selection.clone(), primary: self.primary.clone() }
    }

    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.past.pop() else { return false };
        let current = self.step(Some(&previous.document));
        self.future.push(current);
        self.restore_step(previous);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.future.pop() else { return false };
        let current = self.step(Some(&next.document));
        self.past.push(current);
        self.restore_step(next);
        true
    }

    fn restore_step(&mut self, step: Step) {
        self.selection = step.selection;
        self.primary = step.primary;
        self.restore(step.document.project());
    }

    // ------------------------------------------------------------ playhead

    /// Moves the playhead, snapped to a frame.
    pub fn seek(&mut self, time: f64) {
        let value = clamp(time, 0.0, self.timeline_length().max(self.playback_end()));
        self.time = crate::time::snap_to_frame(value, self.fps());
    }

    pub fn step_frames(&mut self, count: i32) {
        self.seek(self.time + count as f64 * self.frame());
    }

    // ------------------------------------------------------------ selection

    pub fn select(&mut self, id: Option<&str>, additive: bool) {
        let Some(id) = id else {
            self.selection.clear();
            self.primary = None;
            return;
        };
        if additive {
            if let Some(position) = self.selection.iter().position(|value| value == id) {
                self.selection.remove(position);
                self.primary = self.selection.last().cloned();
            } else {
                self.selection.push(id.to_string());
                self.primary = Some(id.to_string());
            }
            return;
        }
        // Captions share a group id for bulk styling, but a click still selects one caption.
        let group = self.item(id).and_then(|item| item.group_id.clone()).filter(|g| !g.starts_with("captions-"));
        self.selection = match group {
            Some(group) => self.project.items().filter(|item| item.group_id.as_deref() == Some(group.as_str())).map(|item| item.id.clone()).collect(),
            None => vec![id.to_string()],
        };
        self.primary = Some(id.to_string());
    }

    pub fn select_many(&mut self, ids: &[String]) {
        let existing: HashSet<&str> = self.project.items().map(|item| item.id.as_str()).collect();
        let mut seen = HashSet::new();
        let selection: Vec<String> = ids.iter().filter(|id| existing.contains(id.as_str()) && seen.insert(id.as_str())).cloned().collect();
        let primary = match &self.primary {
            Some(primary) if selection.contains(primary) => Some(primary.clone()),
            _ => selection.last().cloned(),
        };
        self.selection = selection;
        self.primary = primary;
    }

    pub fn select_all(&mut self) {
        let ids: Vec<String> = self.project.items().map(|item| item.id.clone()).collect();
        self.select_many(&ids);
    }

    // ------------------------------------------------------------ track helpers

    fn sort_track(&mut self, t: usize) {
        self.project.tracks[t].items.sort_by(|a, b| a.start.total_cmp(&b.start).then_with(|| a.id.cmp(&b.id)));
    }

    fn repair_track(&mut self, t: usize) {
        let fps = self.fps();
        let starts = repair_layout(&spans_of(&self.project.tracks[t].items));
        for item in &mut self.project.tracks[t].items {
            item.start = starts.get(&item.id).copied().unwrap_or(item.start);
            item.duration = item.duration.max(MIN_CLIP_DURATION);
            if !item.keyframes.is_empty() {
                item.keyframes = sorted_keyframes(item);
            }
        }
        self.sort_track(t);
        repair_transitions(&mut self.project.tracks[t], fps);
    }

    fn repair_all(&mut self, tracks: impl IntoIterator<Item = usize>) {
        let fps = self.fps();
        let unique: HashSet<usize> = tracks.into_iter().collect();
        for t in unique {
            if t < self.project.tracks.len() {
                self.sort_track(t);
                repair_transitions(&mut self.project.tracks[t], fps);
            }
        }
    }

    fn repair_transitions_everywhere(&mut self) {
        let fps = self.fps();
        for track in &mut self.project.tracks {
            repair_transitions(track, fps);
        }
    }

    fn track_index(&self, id: &str) -> Option<usize> {
        self.project.tracks.iter().position(|track| track.id == id)
    }

    /// Inserts a track: text at the top, audio at the bottom, video above the
    /// first video track (or at `near`).
    fn insert_track(&mut self, kind: TrackKind, near: Option<&str>) -> usize {
        let track = create_track(kind, &self.project.tracks);
        let list = &mut self.project.tracks;
        let near_index = near.and_then(|id| list.iter().position(|t| t.id == id));
        let index = match kind {
            TrackKind::Audio => list.len(),
            TrackKind::Text => 0,
            TrackKind::Video => near_index
                .or_else(|| list.iter().position(|t| t.kind == TrackKind::Video))
                .or_else(|| list.iter().position(|t| t.kind == TrackKind::Audio))
                .unwrap_or(list.len()),
        };
        list.insert(index, track);
        index
    }

    /// Finds a track with room for the span, creating one when needed.
    fn place_on_track(&mut self, kind: ItemKind, span: &Span, preferred: Option<&str>, exact: bool) -> (usize, f64) {
        let track_kind = kind.track_kind();
        let preferred_index = preferred.and_then(|id| self.project.tracks.iter().position(|t| t.id == id && t.kind == track_kind && !t.locked));
        let mut candidates: Vec<usize> = preferred_index.into_iter().collect();
        candidates.extend(
            (0..self.project.tracks.len())
                .rev()
                .filter(|&t| Some(t) != preferred_index && self.project.tracks[t].kind == track_kind && !self.project.tracks[t].locked),
        );
        for t in candidates {
            let placement = place_span(&spans_of(&self.project.tracks[t].items), span, HORIZON, &[]);
            if let Some(placement) = placement {
                if !exact || (placement.start - span.start).abs() < 1e-6 || Some(t) == preferred_index {
                    return (t, placement.start);
                }
            }
        }
        let near = preferred_index.map(|t| self.project.tracks[t].id.clone()).or(preferred.map(str::to_string));
        let t = self.insert_track(track_kind, near.as_deref());
        (t, span.start.max(0.0))
    }

    // ------------------------------------------------------------ adding

    /// Adds a clip built from `template` (its id, track, start are assigned).
    pub fn add_item(&mut self, template: TimelineItem, placement: Placement) -> Option<String> {
        self.snapshot();
        Some(self.add_item_unrecorded(template, placement))
    }

    fn add_item_unrecorded(&mut self, template: TimelineItem, placement: Placement) -> String {
        let id = uid(template.kind.as_str());
        let start = placement.start.unwrap_or(self.time).max(0.0);
        let duration = template.duration.max(MIN_CLIP_DURATION);
        let (t, start) = if let Some(index) = placement.new_track_index {
            let track = create_track(template.kind.track_kind(), &self.project.tracks);
            let index = index.min(self.project.tracks.len());
            self.project.tracks.insert(index, track);
            (index, start)
        } else {
            let exact = placement.track_id.is_none();
            self.place_on_track(template.kind, &Span::new(id.clone(), start, duration), placement.track_id.as_deref(), exact)
        };
        let track_id = self.project.tracks[t].id.clone();
        let name = if template.name.is_empty() { template.kind.as_str().to_string() } else { template.name.clone() };
        let item = TimelineItem { id: id.clone(), track_id, start, duration, name, ..template };
        self.project.tracks[t].items.push(item);
        self.repair_all([t]);
        self.select(Some(&id), false);
        self.touched();
        id
    }

    pub fn add_asset(&mut self, asset: &MediaAsset, placement: Placement) -> Option<String> {
        let template = media_item(asset, &self.project);
        self.add_item(template, placement)
    }

    /// Adds several assets one after another (one undo step); audio starts with the visuals.
    pub fn add_assets_sequence(&mut self, assets: &[MediaAsset], placement: Placement) -> Vec<String> {
        if assets.is_empty() {
            return vec![];
        }
        self.snapshot();
        let start = placement.start.unwrap_or(self.time);
        let mut visual = (start, placement.track_id.clone(), placement.new_track_index);
        let mut audio: (f64, Option<String>, Option<usize>) = (start, None, None);
        let mut created = Vec::new();
        for asset in assets {
            let cursor = if asset.kind == MediaKind::Audio { &mut audio } else { &mut visual };
            let template = media_item(asset, &self.project);
            let id = self.add_item_unrecorded(template, Placement { track_id: cursor.1.clone(), start: Some(cursor.0), new_track_index: cursor.2 });
            let item = self.item(&id).cloned().unwrap_or_default();
            let cursor = if asset.kind == MediaKind::Audio { &mut audio } else { &mut visual };
            *cursor = (item.end(), Some(item.track_id), None);
            created.push(id);
        }
        self.select_many(&created);
        created
    }

    /// Adds assets to the project library (files already there are skipped).
    pub fn register_assets(&mut self, assets: Vec<MediaAsset>) {
        let known: HashSet<String> = self.project.assets.iter().map(|asset| asset.path.clone()).collect();
        let fresh: Vec<MediaAsset> = assets.into_iter().filter(|asset| !known.contains(&asset.path)).collect();
        if fresh.is_empty() {
            return;
        }
        self.project.assets.extend(fresh);
        self.touched();
    }

    pub fn update_asset(&mut self, id: &str, change: impl FnOnce(&mut MediaAsset)) {
        let Some(asset) = self.project.assets.iter_mut().find(|asset| asset.id == id) else { return };
        change(asset);
        self.touched();
    }

    pub fn remove_asset(&mut self, id: &str) {
        self.snapshot();
        self.project.assets.retain(|asset| asset.id != id);
        for track in &mut self.project.tracks {
            track.items.retain(|item| item.asset_id.as_deref() != Some(id));
        }
        self.repair_transitions_everywhere();
        let selection = self.selection.clone();
        self.select_many(&selection);
        self.touched();
    }

    // ------------------------------------------------------------ updating

    fn after_change(&mut self, item: (usize, usize)) {
        let item = self.at(item);
        if let Some(text) = &item.text {
            item.name = item_name(&text.content, &item.name.clone());
        }
    }

    /// Applies `change` to every editable selected clip.
    pub fn update_selected(&mut self, record: bool, mut change: impl FnMut(&mut TimelineItem)) {
        let targets = self.editable_selection();
        if targets.is_empty() {
            return;
        }
        if record {
            self.snapshot();
        }
        for &target in &targets {
            change(self.at(target));
            self.after_change(target);
        }
        self.repair_all(targets.iter().map(|(t, _)| *t));
        self.touched();
    }

    pub fn update_item(&mut self, id: &str, record: bool, change: impl FnOnce(&mut TimelineItem)) {
        let Some(target) = self.editable(id) else { return };
        if record {
            self.snapshot();
        }
        change(self.at(target));
        self.after_change(target);
        self.repair_all([target.0]);
        self.touched();
    }

    /// Style changes go to every selected text; `content` only to the primary one.
    /// Selected text clips, with the other word clips of each word-highlight
    /// caption: a caption line is one clip per word, but is styled as one text.
    fn text_targets(&self) -> Vec<(usize, usize)> {
        let mut targets: Vec<(usize, usize)> = Vec::new();
        for (t, i) in self.editable_selection() {
            let items = &self.project.tracks[t].items;
            if items[i].kind != ItemKind::Text {
                continue;
            }
            for j in caption_run(i, items) {
                if !targets.contains(&(t, j)) {
                    targets.push((t, j));
                }
            }
        }
        targets
    }

    pub fn update_text(&mut self, record: bool, content: Option<&str>, change: impl Fn(&mut TextStyle)) {
        let targets = self.text_targets();
        if targets.is_empty() {
            return;
        }
        if record {
            self.snapshot();
        }
        let primary = self.primary.clone();
        for target in targets {
            let item = self.at(target);
            let own = content.is_some() && Some(&item.id) == primary.as_ref();
            let mut text = item.text.clone().unwrap_or_default();
            change(&mut text);
            if own {
                text.content = content.unwrap_or_default().to_string();
                item.name = item_name(&text.content, "Text");
            }
            item.text = Some(text);
        }
        self.touched();
    }

    /// Turns word highlighting on or off for the selected texts. A caption line
    /// that is one clip is split into one clip per word (timed by the words'
    /// length), so the highlight moves along it as the words are spoken.
    pub fn set_highlight_mode(&mut self, mode: HighlightMode) {
        self.update_text(true, None, |text| {
            text.highlight_mode = mode;
            if text.highlight_index < 0 {
                text.highlight_index = 0;
            }
        });
        if mode == HighlightMode::None {
            return;
        }
        let lines: Vec<(usize, usize)> = self
            .text_targets()
            .into_iter()
            .filter(|&(t, i)| {
                let items = &self.project.tracks[t].items;
                let item = &items[i];
                item.group_id.as_deref().is_some_and(|g| g.starts_with("captions-"))
                    && caption_run(i, items).len() == 1
                    && item.text_style().content.split_whitespace().count() > 1
            })
            .collect();
        let mut split = Vec::new();
        for (t, i) in lines.into_iter().rev() {
            let item = self.project.tracks[t].items.remove(i);
            let style = item.text_style();
            let cue = CaptionCue { text: style.content.clone(), start: item.start, duration: item.duration, highlight_index: None };
            let words = crate::captions::highlight_cues(&[cue], &[]);
            for (k, word) in words.iter().enumerate() {
                let mut clip = item.clone();
                if k > 0 {
                    clip.id = uid("item");
                }
                clip.start = word.start;
                clip.duration = word.duration;
                let text = clip.text.get_or_insert_with(Default::default);
                text.highlight_index = word.highlight_index.unwrap_or(0);
                split.push(clip.id.clone());
                self.project.tracks[t].items.insert(i + k, clip);
            }
        }
        if !split.is_empty() {
            for track in &mut self.project.tracks {
                track.items.sort_by(|a, b| a.start.total_cmp(&b.start));
            }
            self.touched();
        }
    }

    /// Gives every selected text the preset's look; each keeps its own words and layout.
    pub fn apply_text_preset(&mut self, preset: &TextPreset) {
        let targets = self.text_targets();
        if targets.is_empty() {
            return;
        }
        self.snapshot();
        for target in targets {
            let item = self.at(target);
            // Word clips of a caption keep which word they light up.
            let index = item.text_style().highlight_index;
            let mut style = preset_text_style(&item.text_style(), preset);
            style.highlight_index = index;
            item.text = Some(style);
            item.anim_in = preset.animation();
        }
        self.touched();
    }

    /// Transform edits write into a keyframe when the clip is animated.
    pub fn set_transform(&mut self, updates: &[(String, TransformPatch)], record: bool) {
        if record {
            self.snapshot();
        }
        let tolerance = self.keyframe_tolerance();
        let time = self.time;
        for (id, patch) in updates {
            let Some(target) = self.editable(id) else { continue };
            let item = self.at(target);
            let elapsed = time - item.start;
            let inside = elapsed >= -tolerance && elapsed <= item.duration + tolerance;
            if !item.keyframes.is_empty() && inside {
                item.keyframes = patch_keyframe_at(item, elapsed, |t| patch.apply(t), tolerance);
            } else {
                item.transform = Some(normalize_transform(Some(&patch.apply(normalize_transform(item.transform.as_ref())))));
            }
        }
        self.touched();
    }

    fn set_speed_of(&mut self, (t, i): (usize, usize), value: f64) {
        let speed = clamp(if value.is_finite() { value } else { 1.0 }, 0.1, 10.0);
        let item = &self.project.tracks[t].items[i];
        let current = item.speed();
        if (speed - current).abs() < 1e-6 {
            return;
        }
        let before = item.duration;
        let source_length = item.duration * current;
        let layout = ripple_resize(&spans_of(&self.project.tracks[t].items), &item.id.clone(), source_length / speed);
        let item = &mut self.project.tracks[t].items[i];
        item.speed = Some(speed);
        item.duration = layout.duration;
        if !item.keyframes.is_empty() {
            item.keyframes = retime_keyframes(&item.keyframes, before, item.duration);
        }
        if before > 0.0 {
            let ratio = item.duration / before;
            for effect in &mut item.effects {
                if let Some(range) = effect.range {
                    effect.range = Some(EffectRange { start: range.start * ratio, duration: range.duration * ratio });
                }
            }
        }
        for neighbour in &mut self.project.tracks[t].items {
            if let Some(start) = layout.starts.get(&neighbour.id) {
                neighbour.start = *start;
            }
        }
    }

    /// Speed of the selected clips; their length follows and later clips make room.
    pub fn set_speed(&mut self, value: f64, record: bool) {
        let targets = self.editable_selection();
        if targets.is_empty() {
            return;
        }
        if record {
            self.snapshot();
        }
        let ids: Vec<String> = targets.iter().map(|&(t, i)| self.project.tracks[t].items[i].id.clone()).collect();
        for id in ids {
            if let Some(target) = self.find(&id) {
                self.set_speed_of(target, value);
            }
        }
        let tracks: Vec<usize> = self.editable_selection().into_iter().map(|(t, _)| t).collect();
        self.repair_all(tracks);
        self.touched();
    }

    /// Sets the start and/or length of a clip, staying clear of its neighbours.
    pub fn set_timing(&mut self, id: &str, start: Option<f64>, duration: Option<f64>, record: bool) {
        let Some((t, i)) = self.editable(id) else { return };
        if record {
            self.snapshot();
        }
        let item = self.project.tracks[t].items[i].clone();
        let candidate = Span::new(id, start.unwrap_or(item.start).max(0.0), duration.unwrap_or(item.duration).max(MIN_CLIP_DURATION));
        let spans = spans_of(&self.project.tracks[t].items);
        let placement = if start.is_some() {
            place_span(&spans, &candidate, HORIZON, &[])
        } else {
            Some(trim_span(&spans, &Span::new(id, item.start, item.duration), &candidate, Edge::End, HORIZON))
        };
        if let Some(placement) = placement {
            let target = &mut self.project.tracks[t].items[i];
            target.start = placement.start;
            target.duration = placement.duration;
            if !item.keyframes.is_empty() && target.duration < item.duration - 1e-6 {
                target.keyframes = trim_keyframes(&item, 0.0, target.duration);
            }
        }
        self.repair_all([t]);
        self.touched();
    }

    // ------------------------------------------------------------ moving & trimming

    pub fn move_item(&mut self, id: &str, start: f64, target_track: Option<&str>) -> bool {
        let Some((t, i)) = self.editable(id) else { return false };
        let kind = self.project.tracks[t].kind;
        let target = match target_track {
            Some(track_id) if track_id != self.project.tracks[t].id => {
                match self.project.tracks.iter().position(|c| c.id == track_id && c.kind == kind && !c.locked) {
                    Some(index) => index,
                    None => return false,
                }
            }
            _ => t,
        };
        let duration = self.project.tracks[t].items[i].duration;
        let ignore: Vec<&str> = if target == t { vec![id] } else { vec![] };
        let Some(placement) = place_span(&spans_of(&self.project.tracks[target].items), &Span::new(id, start, duration), HORIZON, &ignore) else {
            return false;
        };
        self.project.tracks[t].items[i].start = placement.start;
        if target != t {
            let mut item = self.project.tracks[t].items.remove(i);
            item.track_id = self.project.tracks[target].id.clone();
            self.project.tracks[target].items.push(item);
        }
        self.repair_all([t, target]);
        self.touched();
        true
    }

    pub fn move_items(&mut self, moves: &[(String, f64)]) {
        let entries: Vec<(String, f64, (usize, usize))> =
            moves.iter().filter_map(|(id, start)| self.editable(id).map(|loc| (id.clone(), *start, loc))).collect();
        let tracks: Vec<(String, Vec<Span>)> = self.project.tracks.iter().map(|track| (track.id.clone(), spans_of(&track.items))).collect();
        let candidates: Vec<SelectionSpan> = entries
            .iter()
            .map(|(id, start, (t, i))| SelectionSpan {
                id: id.clone(),
                track_id: self.project.tracks[*t].id.clone(),
                start: *start,
                duration: self.project.tracks[*t].items[*i].duration,
            })
            .collect();
        let Some(placements) = place_selection(&tracks, &candidates, HORIZON) else { return };
        for (id, start) in placements {
            if let Some(target) = self.find(&id) {
                self.at(target).start = start;
            }
        }
        self.repair_all(entries.iter().map(|(_, _, (t, _))| *t));
        self.touched();
    }

    /// Trims one edge. `edge_time` is the requested timeline position of that edge.
    pub fn trim_item(&mut self, id: &str, edge: Edge, edge_time: f64, original: TrimOrigin) {
        let Some((t, i)) = self.editable(id) else { return };
        let item = self.project.tracks[t].items[i].clone();
        let speed = item.speed();
        let is_media = matches!(item.kind, ItemKind::Video | ItemKind::Audio);
        let source_duration = self.asset(item.asset_id.as_deref()).and_then(|a| a.duration).unwrap_or(f64::INFINITY);
        let before = TimelineItem { start: original.start, duration: original.duration, ..item.clone() };
        let spans = spans_of(&self.project.tracks[t].items);
        let current = Span::new(id, original.start, original.duration);
        let target = &mut self.project.tracks[t].items[i];
        match edge {
            Edge::Start => {
                let min_start = if is_media { original.start - original.source_in / speed } else { 0.0 };
                let requested = clamp(edge_time, min_start.max(0.0), original.start + original.duration - MIN_CLIP_DURATION);
                let placement = trim_span(&spans, &current, &Span::new(id, requested, original.start + original.duration - requested), Edge::Start, HORIZON);
                target.start = placement.start;
                target.duration = placement.duration;
                if is_media || target.kind == ItemKind::Image {
                    target.source_in = Some((original.source_in + (placement.start - original.start) * speed).max(0.0));
                }
                if !before.keyframes.is_empty() {
                    target.keyframes = trim_keyframes(&before, (placement.start - original.start).max(0.0), placement.duration);
                }
            }
            Edge::End => {
                let max_end = if is_media { original.start + (source_duration - original.source_in) / speed } else { f64::INFINITY };
                let requested = clamp(edge_time, original.start + MIN_CLIP_DURATION, max_end);
                let placement = trim_span(&spans, &current, &Span::new(id, original.start, requested - original.start), Edge::End, HORIZON);
                target.duration = placement.duration;
                if !before.keyframes.is_empty() && placement.duration < original.duration {
                    target.keyframes = trim_keyframes(&before, 0.0, placement.duration);
                }
            }
        }
        // Ranged effects stay where they are on the timeline while the clip edge moves.
        let shift = target.start - original.start;
        for effect in &mut target.effects {
            let Some(Some(range)) = original.effect_ranges.iter().find(|(id, _)| *id == effect.id).map(|(_, range)| *range) else { continue };
            let moved = EffectRange { start: range.start - shift, duration: range.duration + (range.start - shift).min(0.0) };
            effect.range = Some(effect_window(target.duration, Some(moved)));
        }
        self.repair_all([t]);
        self.touched();
    }

    /// Moves the selection by `delta`; quick repeats share one undo step.
    pub fn nudge_selected(&mut self, delta: f64) {
        let targets = self.editable_selection();
        if targets.is_empty() {
            return;
        }
        let now = Instant::now();
        if self.last_nudge.is_none_or(|last| now.duration_since(last).as_millis() > 500) {
            self.snapshot();
        }
        self.last_nudge = Some(now);
        let moves: Vec<(String, f64)> = targets
            .iter()
            .map(|&(t, i)| {
                let item = &self.project.tracks[t].items[i];
                (item.id.clone(), (item.start + delta).max(0.0))
            })
            .collect();
        self.move_items(&moves);
    }

    pub fn trim_to_playhead(&mut self, edge: Edge) {
        let Some(id) = self.primary.clone() else { return };
        let Some(target) = self.editable(&id) else { return };
        let item = self.project.tracks[target.0].items[target.1].clone();
        if self.time <= item.start + MIN_CLIP_DURATION || self.time >= item.end() - MIN_CLIP_DURATION {
            return;
        }
        self.snapshot();
        self.trim_item(&id, edge, self.time, TrimOrigin::of(&item));
    }

    // ------------------------------------------------------------ structural edits

    /// Splits the selected clips (or every clip under the playhead) at `time`.
    pub fn split_at(&mut self, time: f64) {
        let candidates: Vec<(usize, usize)> = if self.selection.is_empty() {
            self.project
                .tracks
                .iter()
                .enumerate()
                .filter(|(_, track)| !track.locked && !track.hidden)
                .flat_map(|(t, track)| track.items.iter().enumerate().filter(|(_, item)| !item.locked).map(move |(i, _)| (t, i)))
                .collect()
        } else {
            self.editable_selection()
        };
        let targets: Vec<String> = candidates
            .into_iter()
            .map(|(t, i)| &self.project.tracks[t].items[i])
            .filter(|item| time > item.start + MIN_CLIP_DURATION && time < item.end() - MIN_CLIP_DURATION)
            .map(|item| item.id.clone())
            .collect();
        if targets.is_empty() {
            return;
        }
        self.snapshot();
        let mut created = Vec::new();
        for id in targets {
            let Some((t, i)) = self.find(&id) else { continue };
            let before = self.project.tracks[t].items[i].clone();
            let offset = time - before.start;
            let mut right = before.clone();
            right.id = uid(before.kind.as_str());
            right.start = time;
            right.duration = before.duration - offset;
            right.source_in = Some(before.source_in() + offset * before.speed());
            right.keyframes = if before.keyframes.is_empty() { vec![] } else { trim_keyframes(&before, offset, right.duration) };
            right.transition = None;
            if right.anim_in.is_some() {
                right.anim_in = Some(Animation { kind: AnimationInKind::None, duration: 0.0 });
            }
            let left = &mut self.project.tracks[t].items[i];
            if left.anim_out.is_some() {
                left.anim_out = Some(Animation { kind: AnimationOutKind::None, duration: 0.0 });
            }
            if left.fade_out() > 0.0 {
                right.fade_out = left.fade_out;
                left.fade_out = Some(0.0);
            }
            if right.fade_in() > 0.0 {
                right.fade_in = Some(0.0);
            }
            (left.effects, right.effects) = split_effects(&before.effects, before.duration, offset);
            left.duration = offset;
            left.keyframes = if before.keyframes.is_empty() { vec![] } else { trim_keyframes(&before, 0.0, offset) };
            created.push(right.id.clone());
            self.project.tracks[t].items.push(right);
            self.repair_all([t]);
        }
        self.select_many(&created);
        self.touched();
    }

    pub fn delete_selected(&mut self, ripple: bool) {
        let targets = self.editable_selection();
        if targets.is_empty() {
            return;
        }
        self.snapshot();
        let ids: HashSet<String> = targets.iter().map(|&(t, i)| self.project.tracks[t].items[i].id.clone()).collect();
        let tracks: HashSet<usize> = targets.iter().map(|(t, _)| *t).collect();
        for t in tracks {
            let track = &mut self.project.tracks[t];
            let mut removed: Vec<(f64, f64)> = track.items.iter().filter(|item| ids.contains(&item.id)).map(|item| (item.start, item.duration)).collect();
            track.items.retain(|item| !ids.contains(&item.id));
            if ripple {
                removed.sort_by(|a, b| b.0.total_cmp(&a.0));
                for (start, duration) in removed {
                    for item in &mut track.items {
                        if item.start >= start + duration - 1e-6 {
                            item.start = (item.start - duration).max(0.0);
                        }
                    }
                }
                self.repair_track(t);
            } else {
                self.repair_all([t]);
            }
        }
        self.select(None, false);
        self.touched();
    }

    fn clone_items(&mut self, sources: Vec<TimelineItem>, offset: impl Fn(&TimelineItem) -> f64) {
        let mut groups: HashMap<String, String> = HashMap::new();
        let mut created = Vec::new();
        let mut changed = Vec::new();
        for source in sources {
            let mut copy = source.clone();
            copy.id = uid(source.kind.as_str());
            copy.transition = None;
            if let Some(group) = &copy.group_id {
                copy.group_id = Some(groups.entry(group.clone()).or_insert_with(|| uid("group")).clone());
            }
            let (t, start) = self.place_on_track(copy.kind, &Span::new(copy.id.clone(), offset(&source), copy.duration), Some(&source.track_id), false);
            copy.start = start;
            copy.track_id = self.project.tracks[t].id.clone();
            created.push(copy.id.clone());
            self.project.tracks[t].items.push(copy);
            changed.push(t);
        }
        self.repair_all(changed);
        self.select_many(&created);
        self.touched();
    }

    pub fn duplicate_selected(&mut self) {
        let sources: Vec<TimelineItem> = self.editable_selection().into_iter().map(|(t, i)| self.project.tracks[t].items[i].clone()).collect();
        if sources.is_empty() {
            return;
        }
        self.snapshot();
        let end = sources.iter().map(TimelineItem::end).fold(f64::NEG_INFINITY, f64::max);
        let first = sources.iter().map(|item| item.start).fold(f64::INFINITY, f64::min);
        self.clone_items(sources, |item| end + (item.start - first));
    }

    pub fn copy_selected(&mut self) -> usize {
        self.clipboard = self.selected_items().into_iter().cloned().collect();
        self.clipboard.len()
    }

    pub fn cut_selected(&mut self) {
        if self.copy_selected() > 0 {
            self.delete_selected(false);
        }
    }

    pub fn paste(&mut self) {
        if self.clipboard.is_empty() {
            return;
        }
        self.snapshot();
        let sources = self.clipboard.clone();
        let first = sources.iter().map(|item| item.start).fold(f64::INFINITY, f64::min);
        let time = self.time;
        self.clone_items(sources, |item| time + item.start - first);
    }

    pub fn group_selected(&mut self) {
        if self.selection.len() < 2 {
            return;
        }
        self.snapshot();
        let group = uid("group");
        for id in self.selection.clone() {
            if let Some(item) = self.project.item_mut(&id) {
                item.group_id = Some(group.clone());
            }
        }
        self.touched();
    }

    pub fn ungroup_selected(&mut self) {
        let grouped: Vec<String> = self.selected_items().into_iter().filter(|item| item.group_id.is_some()).map(|item| item.id.clone()).collect();
        if grouped.is_empty() {
            return;
        }
        self.snapshot();
        for id in grouped {
            if let Some(item) = self.project.item_mut(&id) {
                item.group_id = None;
            }
        }
        self.touched();
    }

    pub fn toggle_item_flag(&mut self, flag: Flag) {
        let targets: Vec<String> =
            self.selected_items().into_iter().filter(|item| flag != Flag::Muted || item.kind.has_sound()).map(|item| item.id.clone()).collect();
        if targets.is_empty() {
            return;
        }
        self.snapshot();
        let get = |item: &TimelineItem| match flag {
            Flag::Locked => item.locked,
            Flag::Hidden => item.hidden,
            Flag::Muted => item.muted,
        };
        let value = !targets.iter().all(|id| self.item(id).is_some_and(get));
        for id in targets {
            if let Some(item) = self.project.item_mut(&id) {
                match flag {
                    Flag::Locked => item.locked = value,
                    Flag::Hidden => item.hidden = value,
                    Flag::Muted => item.muted = value,
                }
            }
        }
        self.repair_transitions_everywhere();
        self.touched();
    }

    // ------------------------------------------------------------ keyframes

    pub fn toggle_keyframe_at_playhead(&mut self) {
        let Some(id) = self.primary.clone() else { return };
        let Some(target) = self.editable(&id) else { return };
        let tolerance = self.keyframe_tolerance();
        let item = &self.project.tracks[target.0].items[target.1];
        let elapsed = self.time - item.start;
        if item.kind == ItemKind::Audio || elapsed < 0.0 || elapsed > item.duration {
            return;
        }
        self.snapshot();
        let item = self.at(target);
        item.keyframes = toggle_keyframe(item, elapsed, tolerance);
        self.touched();
    }

    pub fn jump_keyframe(&mut self, direction: i32) {
        let Some(item) = self.selected_item() else { return };
        if let Some(keyframe) = adjacent_keyframe(item, self.time - item.start, direction, self.keyframe_tolerance()) {
            let time = item.start + keyframe.time;
            self.seek(time);
        }
    }

    /// `record` = false while dragging a curve handle (call `begin_change` first).
    pub fn set_keyframe_easing_at_playhead(&mut self, easing: Easing, bezier: Option<Bezier>, record: bool) {
        let Some(id) = self.primary.clone() else { return };
        let Some(target) = self.editable(&id) else { return };
        if self.project.tracks[target.0].items[target.1].keyframes.is_empty() {
            return;
        }
        if record {
            self.snapshot();
        }
        let (time, tolerance) = (self.time, self.keyframe_tolerance());
        let item = self.at(target);
        item.keyframes = set_keyframe_easing(item, time - item.start, easing, tolerance, bezier);
        self.touched();
    }

    pub fn move_keyframe(&mut self, item_id: &str, keyframe_id: &str, time: f64) {
        let Some(target) = self.editable(item_id) else { return };
        let (fps, tolerance) = (self.fps(), self.keyframe_tolerance());
        let item = self.at(target);
        if item.keyframes.is_empty() {
            return;
        }
        let snapped = clamp(crate::time::snap_to_frame(time, fps), 0.0, item.duration);
        item.keyframes.retain(|k| k.id == keyframe_id || (k.time - snapped).abs() > tolerance);
        for keyframe in &mut item.keyframes {
            if keyframe.id == keyframe_id {
                keyframe.time = snapped;
            }
        }
        item.keyframes.sort_by(|a, b| a.time.total_cmp(&b.time));
        self.touched();
    }

    pub fn clear_keyframes(&mut self) {
        let Some(id) = self.primary.clone() else { return };
        let Some(target) = self.editable(&id) else { return };
        if self.project.tracks[target.0].items[target.1].keyframes.is_empty() {
            return;
        }
        self.snapshot();
        let item = self.at(target);
        item.transform = Some(normalize_transform(item.transform.as_ref()));
        item.keyframes.clear();
        self.touched();
    }

    // ------------------------------------------------------------ animations & transitions

    /// The animation span of the primary clip (a whole word-highlight caption).
    pub fn motion_span_of(&self, id: &str) -> Option<MotionSpan> {
        let (t, i) = self.find(id)?;
        Some(motion_span(i, &self.project.tracks[t].items))
    }

    /// Selected clips grouped with the other word clips of their caption, which
    /// animate as one text and so all get the same animation.
    fn animation_targets(&self) -> Vec<(MotionSpan, Vec<(usize, usize)>)> {
        let mut seen = HashSet::new();
        let mut groups = Vec::new();
        for id in &self.selection {
            let Some((t, i)) = self.editable(id) else { continue };
            if self.project.tracks[t].items[i].kind == ItemKind::Audio || seen.contains(id) {
                continue;
            }
            let siblings = &self.project.tracks[t].items;
            let run = caption_run(i, siblings);
            for &index in &run {
                seen.insert(siblings[index].id.clone());
            }
            let members = run.into_iter().filter(|&index| !siblings[index].locked).map(|index| (t, index)).collect();
            groups.push((motion_span(i, siblings), members));
        }
        groups
    }

    pub fn set_animation(&mut self, choice: AnimationChoice) {
        let groups = self.animation_targets();
        if groups.is_empty() {
            return;
        }
        self.snapshot();
        for (span, members) in groups {
            for target in members {
                let item = self.at(target);
                match choice {
                    AnimationChoice::In(kind) => {
                        let duration = span
                            .anim_in
                            .filter(|a| a.kind != AnimationInKind::None && a.duration > 0.0)
                            .map(|a| a.duration)
                            .unwrap_or_else(|| default_in_duration(kind));
                        item.anim_in = Some(Animation { kind, duration });
                    }
                    AnimationChoice::Out(kind) => {
                        let duration = span
                            .anim_out
                            .filter(|a| a.kind != AnimationOutKind::None && a.duration > 0.0)
                            .map(|a| a.duration)
                            .unwrap_or_else(|| default_out_duration(kind));
                        item.anim_out = Some(Animation { kind, duration });
                    }
                }
            }
        }
        self.touched();
    }

    /// Continuous change (slider): call `begin_change` first.
    pub fn set_animation_duration(&mut self, slot: Slot, duration: f64) {
        for (span, members) in self.animation_targets() {
            for target in members {
                let item = self.at(target);
                match slot {
                    Slot::In => {
                        if let Some(current) = span.anim_in.filter(|a| a.kind != AnimationInKind::None) {
                            item.anim_in = Some(Animation { kind: current.kind, duration });
                        }
                    }
                    Slot::Out => {
                        if let Some(current) = span.anim_out.filter(|a| a.kind != AnimationOutKind::None) {
                            item.anim_out = Some(Animation { kind: current.kind, duration });
                        }
                    }
                }
            }
        }
        self.touched();
    }

    pub fn set_transition(&mut self, item_id: &str, kind: TransitionKind, duration: Option<f64>, record: bool) {
        let Some((t, i)) = self.editable(item_id) else { return };
        if record {
            self.snapshot();
        }
        let fps = self.fps();
        if kind == TransitionKind::None {
            self.project.tracks[t].items[i].transition = None;
        } else {
            let tolerance = 0.5 / fps as f64;
            let item = &self.project.tracks[t].items[i];
            let requested = duration.or(item.transition.map(|tr| tr.duration)).unwrap_or(DEFAULT_TRANSITION);
            let mut next = TimelineItem { transition: Some(Animation { kind, duration: requested }), ..item.clone() };
            let previous = self.project.tracks[t].items.iter().find(|c| (c.end() - item.start).abs() < tolerance && c.id != item_id);
            if let Some(previous) = previous {
                let length = transition_duration(&next, previous);
                next.transition = Some(Animation { kind, duration: length });
            }
            self.project.tracks[t].items[i].transition = next.transition;
        }
        repair_transitions(&mut self.project.tracks[t], fps);
        self.touched();
    }

    /// Applies the same transition to every cut of the track.
    pub fn set_transition_for_track(&mut self, track_id: &str, kind: TransitionKind, duration: f64) {
        let Some(t) = self.track_index(track_id) else { return };
        if self.project.tracks[t].locked {
            return;
        }
        self.snapshot();
        for item in &mut self.project.tracks[t].items {
            if !item.locked {
                item.transition = Some(Animation { kind, duration });
            }
        }
        let fps = self.fps();
        repair_transitions(&mut self.project.tracks[t], fps);
        self.touched();
    }

    fn effect_targets(&self, ids: &[String]) -> Vec<(usize, usize)> {
        ids.iter().filter_map(|id| self.editable(id)).filter(|&(t, i)| can_have_effects(&self.project.tracks[t].items[i])).collect()
    }

    /// Adds an effect to the top of each clip's stack, covering the whole clip
    /// or `range`. A clip that has the effect already keeps it (moved to `range`
    /// when one is given) rather than getting it twice. Returns the effect's id
    /// on each clip, as (clip, effect).
    pub fn add_effect(&mut self, ids: &[String], kind: EffectKind, range: Option<EffectRange>) -> Vec<(String, String)> {
        let targets = self.effect_targets(ids);
        if targets.is_empty() || kind == EffectKind::None {
            return vec![];
        }
        self.snapshot();
        let mut added = Vec::new();
        for target in targets {
            let item = self.at(target);
            let window = range.map(|r| effect_window(item.duration, Some(r)));
            let duration = item.duration;
            let full = item.effects.len() >= MAX_EFFECTS;
            let id = match item.effects.iter_mut().find(|e| e.kind == kind) {
                Some(existing) => {
                    if let Some(window) = window {
                        existing.range = stored_range(duration, window);
                    }
                    existing.id.clone()
                }
                None if full => continue,
                None => {
                    let effect = ClipEffect { id: uid("fx"), kind, range: window.and_then(|w| stored_range(duration, w)), ..Default::default() };
                    let id = effect.id.clone();
                    item.effects.push(effect);
                    id
                }
            };
            added.push((item.id.clone(), id));
        }
        self.touched();
        added
    }

    /// Takes one effect off a clip.
    pub fn remove_effect(&mut self, item_id: &str, effect_id: &str) {
        let Some(target) = self.editable(item_id) else { return };
        if !self.project.tracks[target.0].items[target.1].effects.iter().any(|e| e.id == effect_id) {
            return;
        }
        self.snapshot();
        self.at(target).effects.retain(|e| e.id != effect_id);
        self.touched();
    }

    /// Takes every effect off the clips.
    pub fn clear_effects(&mut self, ids: &[String]) {
        let targets: Vec<_> = self.effect_targets(ids).into_iter().filter(|&(t, i)| !self.project.tracks[t].items[i].effects.is_empty()).collect();
        if targets.is_empty() {
            return;
        }
        self.snapshot();
        for target in targets {
            self.at(target).effects.clear();
        }
        self.touched();
    }

    fn effect_mut(&mut self, item_id: &str, effect_id: &str, record: bool) -> Option<(f64, &mut ClipEffect)> {
        let target = self.editable(item_id)?;
        self.project.tracks[target.0].items[target.1].effects.iter().position(|e| e.id == effect_id)?;
        if record {
            self.snapshot();
        }
        let item = self.at(target);
        let duration = item.duration;
        item.effects.iter_mut().find(|e| e.id == effect_id).map(|effect| (duration, effect))
    }

    /// Strength of one effect, 0..1.
    pub fn set_effect_amount(&mut self, item_id: &str, effect_id: &str, amount: f64, record: bool) {
        if !amount.is_finite() {
            return;
        }
        let Some((_, effect)) = self.effect_mut(item_id, effect_id, record) else { return };
        effect.amount = clamp(amount, 0.0, 1.0);
        self.touched();
    }

    /// Moves or stretches the part of the clip an effect covers (relative to the clip start).
    pub fn set_effect_range(&mut self, item_id: &str, effect_id: &str, range: EffectRange, record: bool) {
        let Some((duration, effect)) = self.effect_mut(item_id, effect_id, record) else { return };
        effect.range = stored_range(duration, effect_window(duration, Some(range)));
        self.touched();
    }

    // ------------------------------------------------------------ tracks

    pub fn add_track(&mut self, kind: TrackKind) {
        self.snapshot();
        self.insert_track(kind, None);
        self.touched();
    }

    pub fn update_track(&mut self, track_id: &str, patch: TrackPatch) {
        let Some(t) = self.track_index(track_id) else { return };
        self.snapshot();
        let track = &mut self.project.tracks[t];
        if let Some(name) = patch.name {
            if !name.trim().is_empty() {
                track.name = name.trim().to_string();
            }
        }
        if let Some(value) = patch.locked {
            track.locked = value;
        }
        if let Some(value) = patch.muted {
            track.muted = value;
        }
        if let Some(value) = patch.hidden {
            track.hidden = value;
        }
        self.touched();
    }

    pub fn remove_track(&mut self, track_id: &str) {
        let Some(t) = self.track_index(track_id) else { return };
        if self.project.tracks[t].locked {
            return;
        }
        self.snapshot();
        self.project.tracks.remove(t);
        let selection = self.selection.clone();
        self.select_many(&selection);
        self.touched();
    }

    pub fn move_track(&mut self, track_id: &str, to_index: usize) {
        let Some(from) = self.track_index(track_id) else { return };
        if from == to_index {
            return;
        }
        self.snapshot();
        let track = self.project.tracks.remove(from);
        let index = to_index.min(self.project.tracks.len());
        self.project.tracks.insert(index, track);
        self.touched();
    }

    fn remove_empty_tracks(&mut self) {
        self.project.tracks.retain(|track| !track.items.is_empty());
    }

    // ------------------------------------------------------------ captions

    /// Replaces the captions generated from `options.source_id` with new cues.
    pub fn apply_captions(&mut self, cues: &[CaptionCue], options: CaptionOptions) -> usize {
        self.snapshot();
        let group = format!("captions-{}", options.source_id);
        for track in &mut self.project.tracks {
            track.items.retain(|item| item.group_id.as_deref() != Some(group.as_str()));
        }
        let mut t = match self.project.tracks.iter().position(|c| c.kind == TrackKind::Text && c.name.starts_with("Captions") && !c.locked) {
            Some(index) => index,
            None => self.new_caption_track(),
        };
        let speed = if options.speed > 0.0 { options.speed } else { 1.0 };
        let items: Vec<TimelineItem> = cues
            .iter()
            .map(|cue| TimelineItem {
                id: uid("caption"),
                kind: ItemKind::Text,
                name: cue.text.chars().take(40).collect(),
                start: options.offset + cue.start / speed,
                duration: (cue.duration / speed).max(MIN_CLIP_DURATION),
                group_id: Some(group.clone()),
                transform: Some(normalize_transform(Some(&Transform { x: 50.0, y: options.y, ..Default::default() }))),
                text: Some(TextStyle { content: cue.text.clone(), highlight_index: cue.highlight_index.unwrap_or(-1), ..options.style.clone() }),
                // Word clips of a highlight caption share it; it plays once, on the first word.
                anim_in: Some(Animation { kind: AnimationInKind::Pop, duration: 0.18 }),
                ..Default::default()
            })
            .collect();
        let occupied = self.project.tracks[t].items.iter().any(|item| items.iter().any(|cue| item.start < cue.end() && cue.start < item.end()));
        if occupied {
            t = self.new_caption_track();
        }
        let track_id = self.project.tracks[t].id.clone();
        let count = items.len();
        self.project.tracks[t].items.extend(items.into_iter().map(|item| TimelineItem { track_id: track_id.clone(), ..item }));
        self.repair_track(t);
        self.remove_empty_tracks();
        self.touched();
        count
    }

    fn new_caption_track(&mut self) -> usize {
        let mut track = create_track(TrackKind::Text, &self.project.tracks);
        track.name = "Captions".into();
        self.project.tracks.insert(0, track);
        0
    }

    /// Copies the text style of the primary caption to every caption of its group.
    pub fn apply_style_to_group(&mut self) -> usize {
        let Some(item) = self.selected_item().cloned() else { return 0 };
        let (Some(text), Some(group)) = (item.text.clone(), item.group_id.clone()) else { return 0 };
        self.snapshot();
        let mut count = 0;
        for track in &mut self.project.tracks {
            for candidate in &mut track.items {
                if candidate.group_id.as_deref() != Some(group.as_str()) || candidate.id == item.id || candidate.kind != ItemKind::Text {
                    continue;
                }
                // Word-highlight captions keep which word is lit.
                let current = candidate.text_style();
                candidate.text = Some(TextStyle { content: current.content, highlight_index: current.highlight_index, ..text.clone() });
                candidate.transform = Some(normalize_transform(item.transform.as_ref()));
                candidate.anim_in = item.anim_in;
                candidate.anim_out = item.anim_out;
                count += 1;
            }
        }
        self.touched();
        count
    }

    // ------------------------------------------------------------ project settings

    pub fn rename(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() || name == self.project.name {
            return;
        }
        self.snapshot();
        self.project.name = name.into();
        self.touched();
    }

    pub fn set_canvas(&mut self, width: u32, height: u32) {
        if self.project.width == width && self.project.height == height {
            return;
        }
        self.snapshot();
        self.project.width = width;
        self.project.height = height;
        self.project.export.resolution = if width.min(height) >= 1080 { 1080 } else { width.min(height) };
        self.touched();
    }

    pub fn set_background(&mut self, color: &str, record: bool) {
        if record {
            self.snapshot();
        }
        self.project.background = color.into();
        self.touched();
    }

    pub fn update_export(&mut self, change: impl FnOnce(&mut ExportSettings)) {
        change(&mut self.project.export);
        if self.project.export.fps != self.project.fps && FPS_CHOICES.contains(&self.project.export.fps) {
            self.project.fps = self.project.export.fps;
            self.repair_transitions_everywhere();
        }
        self.touched();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::{create_project, text_item};

    fn editor() -> Editor {
        Editor::new(create_project("Test", 1080, 1920, 30))
    }

    fn add_text(editor: &mut Editor, start: f64) -> String {
        editor.add_item(TimelineItem { duration: 2.0, ..text_item("Hi", None) }, Placement { start: Some(start), ..Default::default() }).unwrap()
    }

    #[test]
    fn adding_creates_a_track_and_selects() {
        let mut e = editor();
        let id = add_text(&mut e, 1.0);
        assert_eq!(e.project.tracks.len(), 1);
        assert_eq!(e.project.tracks[0].kind, TrackKind::Text);
        assert_eq!(e.primary.as_deref(), Some(id.as_str()));
        assert_eq!(e.item(&id).unwrap().start, 1.0);
    }

    #[test]
    fn overlapping_add_goes_to_a_new_track() {
        let mut e = editor();
        add_text(&mut e, 0.0);
        add_text(&mut e, 1.0);
        assert_eq!(e.project.tracks.len(), 2);
    }

    #[test]
    fn undo_and_redo_restore_documents() {
        let mut e = editor();
        let id = add_text(&mut e, 0.0);
        assert!(e.undo());
        assert!(e.item(&id).is_none());
        assert!(e.redo());
        assert!(e.item(&id).is_some());
        assert!(!e.redo());
    }

    #[test]
    fn split_makes_two_clips() {
        let mut e = editor();
        add_text(&mut e, 0.0);
        e.select(None, false);
        e.split_at(0.5);
        let items = &e.project.tracks[0].items;
        assert_eq!(items.len(), 2);
        assert!((items[0].duration - 0.5).abs() < 1e-9);
        assert!((items[1].start - 0.5).abs() < 1e-9);
        assert!((items[1].duration - 1.5).abs() < 1e-9);
    }

    #[test]
    fn effects_stack_and_come_off_one_by_one() {
        let mut e = editor();
        let id = add_text(&mut e, 0.0);
        let ids = vec![id.clone()];
        let blur = e.add_effect(&ids, EffectKind::Blur, None)[0].1.clone();
        let glitch = e.add_effect(&ids, EffectKind::Glitch, Some(EffectRange { start: 0.5, duration: 1.0 }))[0].1.clone();
        // The same effect again is the one already there.
        assert_eq!(e.add_effect(&ids, EffectKind::Blur, None)[0].1, blur);
        let item = e.item(&id).unwrap();
        assert_eq!(item.effects.iter().map(|x| x.kind).collect::<Vec<_>>(), [EffectKind::Blur, EffectKind::Glitch]);
        assert_eq!(item.effects[1].range, Some(EffectRange { start: 0.5, duration: 1.0 }));
        e.set_effect_amount(&id, &blur, 0.9, true);
        assert_eq!(e.item(&id).unwrap().effects[0].amount, 0.9);
        // A range over the whole clip is stored as no range.
        e.set_effect_range(&id, &glitch, EffectRange { start: 0.0, duration: 5.0 }, true);
        assert_eq!(e.item(&id).unwrap().effects[1].range, None);
        e.remove_effect(&id, &blur);
        assert_eq!(e.item(&id).unwrap().effects.len(), 1);
        assert!(e.undo());
        assert_eq!(e.item(&id).unwrap().effects.len(), 2);
        e.clear_effects(&ids);
        assert!(e.item(&id).unwrap().effects.is_empty());
    }

    #[test]
    fn splitting_divides_each_effect() {
        let mut e = editor();
        let id = add_text(&mut e, 0.0);
        let ids = vec![id.clone()];
        e.add_effect(&ids, EffectKind::Mono, None);
        e.add_effect(&ids, EffectKind::Blur, Some(EffectRange { start: 0.2, duration: 0.6 }));
        e.add_effect(&ids, EffectKind::Glitch, Some(EffectRange { start: 1.2, duration: 0.5 }));
        e.select(None, false);
        e.split_at(1.0);
        let items = &e.project.tracks[0].items;
        let kinds = |i: usize| items[i].effects.iter().map(|x| x.kind).collect::<Vec<_>>();
        assert_eq!(kinds(0), [EffectKind::Mono, EffectKind::Blur]);
        assert_eq!(kinds(1), [EffectKind::Mono, EffectKind::Glitch]);
        let tail = items[1].effects[1].range.unwrap();
        assert!((tail.start - 0.2).abs() < 1e-9 && (tail.duration - 0.5).abs() < 1e-9, "{tail:?}");
        assert_ne!(items[0].effects[0].id, items[1].effects[0].id, "each half has its own effect");
    }

    #[test]
    fn effects_stay_off_sound_and_shapes() {
        let mut e = editor();
        let shape = e.add_item(crate::presets::shape_item(ShapeKind::Rect), Placement::default()).unwrap();
        assert!(e.add_effect(std::slice::from_ref(&shape), EffectKind::Blur, None).is_empty());
        assert!(e.item(&shape).unwrap().effects.is_empty());
    }

    #[test]
    fn ripple_delete_closes_the_gap() {
        let mut e = editor();
        let first = add_text(&mut e, 0.0);
        let second = e
            .add_item(
                TimelineItem { duration: 2.0, ..text_item("B", None) },
                Placement { start: Some(2.0), track_id: Some(e.project.tracks[0].id.clone()), ..Default::default() },
            )
            .unwrap();
        e.select(Some(&first), false);
        e.delete_selected(true);
        assert_eq!(e.item(&second).unwrap().start, 0.0);
    }

    #[test]
    fn speed_change_rescales_and_pushes() {
        let mut e = editor();
        let track = {
            add_text(&mut e, 0.0);
            e.project.tracks[0].id.clone()
        };
        let next = e
            .add_item(TimelineItem { duration: 1.0, ..text_item("B", None) }, Placement { start: Some(2.0), track_id: Some(track), ..Default::default() })
            .unwrap();
        let first = e.project.tracks[0].items[0].id.clone();
        e.select(Some(&first), false);
        e.set_speed(0.5, true);
        assert!((e.item(&first).unwrap().duration - 4.0).abs() < 1e-9);
        assert!((e.item(&next).unwrap().start - 4.0).abs() < 1e-9);
    }

    #[test]
    fn captions_replace_previous_ones() {
        let mut e = editor();
        let cues = vec![CaptionCue { text: "hello world".into(), start: 0.0, duration: 1.0, highlight_index: None }];
        let options = || CaptionOptions { source_id: "v".into(), offset: 2.0, speed: 1.0, style: TextStyle::default(), y: 72.0 };
        assert_eq!(e.apply_captions(&cues, options()), 1);
        assert_eq!(e.apply_captions(&cues, options()), 1);
        let captions: Vec<&TimelineItem> = e.project.items().filter(|i| i.group_id.as_deref() == Some("captions-v")).collect();
        assert_eq!(captions.len(), 1);
        assert_eq!(captions[0].start, 2.0);
    }

    #[test]
    fn animation_applies_to_the_whole_caption() {
        let mut e = editor();
        let cues: Vec<CaptionCue> =
            (0..3).map(|i| CaptionCue { text: "a b c".into(), start: i as f64 * 0.5, duration: 0.5, highlight_index: Some(i) }).collect();
        e.apply_captions(&cues, CaptionOptions { source_id: "v".into(), offset: 0.0, speed: 1.0, style: TextStyle::default(), y: 72.0 });
        let middle = e.project.tracks[0].items[1].id.clone();
        e.select(Some(&middle), false);
        e.set_animation(AnimationChoice::Out(AnimationOutKind::Fade));
        assert!(e.project.tracks[0].items.iter().all(|i| i.anim_out.map(|a| a.kind) == Some(AnimationOutKind::Fade)));
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;
    use crate::presets::{create_project, text_item};

    #[test]
    fn undo_brings_back_the_selection_from_before_the_change() {
        let mut editor = Editor::new(create_project("History", 1080, 1920, 30));
        let first = editor.add_item(text_item("One", None), Placement::default()).unwrap();
        assert_eq!(editor.primary.as_deref(), Some(first.as_str()));
        editor.duplicate_selected();
        let copy = editor.primary.clone().unwrap();
        assert_ne!(copy, first);
        editor.undo();
        assert_eq!(editor.primary.as_deref(), Some(first.as_str()), "the original is selected again");
        assert_eq!(editor.selection, vec![first.clone()]);
        editor.redo();
        assert_eq!(editor.primary.as_deref(), Some(copy.as_str()), "redo selects the copy again");
    }

    #[test]
    fn steps_share_the_tracks_a_change_did_not_touch() {
        let mut editor = Editor::new(create_project("History", 1080, 1920, 30));
        for word in ["One", "Two", "Three"] {
            editor.add_item(text_item(word, None), Placement::default()).unwrap();
        }
        editor.add_track(TrackKind::Video);
        let before = editor.project.clone();
        // Two changes to the text track only.
        editor.select_many(&[before.tracks.iter().find(|t| t.kind == TrackKind::Text).unwrap().items[0].id.clone()]);
        editor.nudge_selected(1.0);
        editor.begin_change();
        editor.update_selected(false, |item| item.volume = Some(0.5));
        let [older, newer] = [&editor.past[editor.past.len() - 2].document, &editor.past[editor.past.len() - 1].document];
        let video = |s: &Snapshot| s.tracks.iter().find(|t| t.kind == TrackKind::Video).unwrap().clone();
        assert!(Arc::ptr_eq(&video(older), &video(newer)), "the untouched track is stored once");
        assert!(Arc::ptr_eq(&older.assets, &newer.assets));
        // Undo all the way gives back exactly the document.
        while editor.undo() {}
        let first = editor.project.clone();
        while editor.redo() {}
        editor.undo();
        editor.undo();
        assert_eq!(editor.project.tracks, before.tracks);
        assert_ne!(first.tracks, before.tracks);
    }

    #[test]
    fn styling_one_word_styles_its_whole_caption_line() {
        use crate::model::HighlightMode;
        // A whisper caption line: one clip per word, the same words, the lit word moving on.
        let word = |id: &str, content: &str, start: f64, duration: f64, index: i64| {
            let mut item = text_item(content, None);
            item.id = id.into();
            item.start = start;
            item.duration = duration;
            item.group_id = Some("captions-v".into());
            let text = item.text.as_mut().unwrap();
            text.highlight_mode = HighlightMode::Color;
            text.highlight_index = index;
            item
        };
        let mut project = create_project("Words", 1080, 1920, 30);
        let mut track = crate::presets::create_track(TrackKind::Text, &[]);
        track.items = vec![
            word("w0", "one two three", 0.0, 0.4, 0),
            word("w1", "one two three", 0.4, 0.4, 1),
            word("w2", "one two three", 0.8, 0.4, 2),
            word("n0", "next line", 1.2, 0.6, 0),
        ];
        project.tracks.push(track);
        let mut editor = Editor::new(project);
        let ids: Vec<String> = ["w0", "w1", "w2"].map(String::from).to_vec();
        let other = "n0".to_string();
        editor.select_many(&[ids[1].clone()]);
        editor.update_text(true, None, |text| text.highlight_color = "#ff0000".into());
        for (index, id) in ids.iter().enumerate() {
            let style = editor.project.item(id).unwrap().text_style();
            assert_eq!(style.highlight_color, "#ff0000", "word {index} of the line");
            assert_eq!(style.highlight_index, index as i64, "each clip still lights its own word");
        }
        assert_ne!(editor.project.item(&other).unwrap().text_style().highlight_color, "#ff0000", "the next line is its own caption");
        // A style preset also covers the line and keeps the lit words.
        editor.apply_text_preset(&crate::presets::TEXT_PRESETS[0]);
        let fonts: Vec<String> = ids.iter().map(|id| editor.project.item(id).unwrap().text_style().font).collect();
        assert!(fonts.iter().all(|f| *f == fonts[0]));
        assert_eq!(editor.project.item(&ids[2]).unwrap().text_style().highlight_index, 2);
    }

    #[test]
    fn turning_highlight_on_splits_a_caption_line_into_words() {
        use crate::model::HighlightMode;
        let mut project = create_project("Line", 1080, 1920, 30);
        let mut track = crate::presets::create_track(TrackKind::Text, &[]);
        let mut line = text_item("say it loud", None);
        line.id = "line".into();
        line.start = 1.0;
        line.duration = 1.2;
        line.group_id = Some("captions-v".into());
        track.items = vec![line];
        project.tracks.push(track);
        let mut editor = Editor::new(project);
        editor.select_many(&["line".to_string()]);
        editor.set_highlight_mode(HighlightMode::Color);
        let words = &editor.project.tracks.iter().find(|t| t.kind == TrackKind::Text).unwrap().items;
        assert_eq!(words.len(), 3, "one clip per word");
        assert_eq!(words.iter().map(|w| w.text_style().highlight_index).collect::<Vec<_>>(), [0, 1, 2]);
        assert!((words[0].start - 1.0).abs() < 1e-9 && (words[2].end() - 2.2).abs() < 1e-9, "the line keeps its time");
        assert_eq!(caption_run(1, words).len(), 3, "the words are still one caption");
        // One undo brings the single clip back.
        editor.undo();
        assert_eq!(editor.project.items().count(), 1);
    }
}
