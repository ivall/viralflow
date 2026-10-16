//! The timeline: layout, what is visible, and every drag on it (clips, trims,
//! keyframes, effects, transitions, marquee, scrubbing, track order).

use std::collections::HashSet;
use std::rc::Rc;
use std::time::Instant;

use slint::{Color, ModelRc, SharedString, VecModel};
use vf_core::edit::{Editor, TrimOrigin};
use vf_core::engine::effects::{effect_window, EFFECTS, MIN_EFFECT};
use vf_core::engine::keyframes::sorted_keyframes;
use vf_core::engine::math::clamp;
use vf_core::engine::transitions::{track_seams, MAX_TRANSITION, MIN_TRANSITION};
use vf_core::model::{EffectKind, EffectRange, ItemKind, TimelineItem, Track, TrackKind};
use vf_core::presets::TRACK_COLORS;
use vf_core::timeline::Edge;

use super::overview::Overviews;
use crate::models::sync_rows;
use crate::{format, ClipView, KeyMark, MenuEntry, SeamView, Tick, TimelineState, TrackRow};

pub const PAD: f64 = 16.0;
pub const RULER_H: f64 = 30.0;
const GAP: f64 = 4.0;
const TOP: f64 = 8.0;
const SNAP_PX: f64 = 8.0;
const TRIM_PX: f64 = 8.0;
pub const MIN_ZOOM: f64 = 2.0;
pub const MAX_ZOOM: f64 = 600.0;

fn row_height(kind: TrackKind) -> f64 {
    match kind {
        TrackKind::Video => 72.0,
        TrackKind::Audio => 54.0,
        TrackKind::Text => 38.0,
    }
}

/// Where a pointer is, in timeline terms.
#[derive(Debug, Clone, PartialEq)]
pub enum Hit {
    Empty,
    Clip(String),
    Trim(String, Edge),
    Keyframe(String, String, f64),
    Effect(String, EffectPart),
    Seam(String),
    SeamEdge(String, Edge),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectPart {
    Body,
    Start,
    End,
}

#[derive(Debug, Clone)]
enum Drag {
    Scrub,
    Clip {
        item: String,
        ids: Vec<String>,
        origins: Vec<(String, f64)>,
        start_x: f64,
        start_y: f64,
        start_scroll: f64,
        moved: bool,
        was_selected: bool,
        kind: TrackKind,
    },
    Trim {
        item: String,
        edge: Edge,
        origin: TrimOrigin,
        start_x: f64,
        start_scroll: f64,
        moved: bool,
    },
    Keyframe {
        item: String,
        keyframe: String,
        time: f64,
        start_x: f64,
        moved: bool,
    },
    Marquee {
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        additive: bool,
        previous: Vec<String>,
        moved: bool,
    },
    Effect {
        item: String,
        part: EffectPart,
        origin: EffectRange,
        start_x: f64,
        moved: bool,
    },
    Seam {
        item: String,
        edge: Edge,
        cut: f64,
        limit: f64,
        kind: vf_core::model::TransitionKind,
        moved: bool,
    },
    Track {
        id: String,
        start_y: f64,
        index: usize,
        moved: bool,
    },
}

/// What a drag from the panels carries, as far as the timeline cares.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DropKind {
    Media { kind: ItemKind, duration: f64 },
    Effect,
    Transition,
}

/// Where a drag from the panels would land.
#[derive(Debug, Clone, PartialEq)]
pub enum DropTarget {
    /// The cut before this clip.
    Seam(String),
    Clip(String),
    Track {
        track_id: String,
        start: f64,
    },
    New {
        index: usize,
        start: f64,
    },
}

/// Actions the timeline asks the editor page to perform.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    OpenPanel(&'static str),
}

pub struct TimelineView {
    pub zoom: f64,
    pub scroll_x: f64,
    pub scroll_y: f64,
    pub width: f64,
    pub height: f64,
    pub snapping: bool,
    drag: Option<Drag>,
    snap_line: Option<f64>,
    hover_row: Option<String>,
    cursor: i32,
    context: Option<Hit>,
    context_time: f64,
    fitted: bool,
    pub overviews: Overviews,
    pub transition_drag: bool,
    pub drop_hint: Option<(String, f64, f64)>,
    pub drop_new: Option<(f64, f64, f64)>,
    pub drop_clip: Option<String>,
    /// Transition dragged onto this cut (the clip after it).
    pub drop_seam: Option<String>,
    /// Where wheel scrolling is heading; the view glides there (`animate`).
    scroll_target: Option<(f64, f64)>,
    /// Zoom a Ctrl+wheel is heading to, and the viewport x it zooms around.
    zoom_target: Option<(f64, f64)>,
    last_step: Option<Instant>,
    models: Models,
}

/// The models the Slint timeline shows. They live as long as the view and are
/// updated in place, so dragging a track header or a clip never loses its
/// element and nothing is rebuilt on every frame.
struct Models {
    rows: Rc<VecModel<TrackRow>>,
    clips: Rc<VecModel<ClipView>>,
    keyframes: Rc<VecModel<KeyMark>>,
    seams: Rc<VecModel<SeamView>>,
    ticks: Rc<VecModel<Tick>>,
    bound: bool,
}

impl Default for Models {
    fn default() -> Self {
        Models {
            rows: Rc::new(VecModel::default()),
            clips: Rc::new(VecModel::default()),
            keyframes: Rc::new(VecModel::default()),
            seams: Rc::new(VecModel::default()),
            ticks: Rc::new(VecModel::default()),
            bound: false,
        }
    }
}

/// Width of the blocks clip pictures are composed in: scrolling inside a block
/// only moves the picture.
const PICTURE_BLOCK: f64 = 1024.0;
/// How quickly smooth scrolling and zooming settle (per second).
const GLIDE: f64 = 18.0;

impl TimelineView {
    pub fn new(snapping: bool) -> Self {
        TimelineView {
            zoom: 60.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            width: 800.0,
            height: 300.0,
            snapping,
            drag: None,
            snap_line: None,
            hover_row: None,
            cursor: 0,
            context: None,
            context_time: 0.0,
            fitted: false,
            overviews: Overviews::default(),
            transition_drag: false,
            drop_hint: None,
            drop_new: None,
            drop_clip: None,
            drop_seam: None,
            scroll_target: None,
            zoom_target: None,
            last_step: None,
            models: Models::default(),
        }
    }

    // ------------------------------------------------------------ geometry

    fn rows(tracks: &[Track]) -> Vec<(f64, f64)> {
        let mut top = TOP;
        tracks
            .iter()
            .map(|track| {
                let row = (top, row_height(track.kind));
                top += row.1 + GAP;
                row
            })
            .collect()
    }

    fn rows_height(tracks: &[Track]) -> f64 {
        Self::rows(tracks).last().map(|(top, h)| top + h).unwrap_or(0.0) + 48.0
    }

    /// Viewport x of a timeline time.
    pub fn x(&self, time: f64) -> f64 {
        PAD + time * self.zoom - self.scroll_x
    }

    /// Timeline time at a viewport x.
    pub fn time_at(&self, x: f64) -> f64 {
        ((x + self.scroll_x - PAD) / self.zoom).max(0.0)
    }

    fn area_height(&self) -> f64 {
        (self.height - RULER_H).max(1.0)
    }

    fn content_width(&self, editor: &Editor) -> f64 {
        PAD * 2.0 + (editor.timeline_length() * self.zoom).max(self.width)
    }

    pub fn clamp_scroll(&mut self, editor: &Editor) {
        let max_x = (self.content_width(editor) - self.width).max(0.0);
        self.scroll_x = self.scroll_x.clamp(0.0, max_x);
        let max_y = (Self::rows_height(&editor.project.tracks) - self.area_height()).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, max_y);
    }

    pub fn set_size(&mut self, width: f64, height: f64, editor: &Editor) {
        self.width = width.max(1.0);
        self.height = height.max(1.0);
        if !self.fitted && width > 50.0 {
            self.fitted = true;
            self.zoom_to_fit(editor);
        }
        self.clamp_scroll(editor);
    }

    /// Zoom around a viewport x (the playhead when None).
    pub fn set_zoom(&mut self, value: f64, anchor: Option<f64>, editor: &Editor) {
        self.zoom_target = None;
        self.scroll_target = None;
        self.apply_zoom(value, anchor, editor);
    }

    fn apply_zoom(&mut self, value: f64, anchor: Option<f64>, editor: &Editor) {
        let next = clamp(value, MIN_ZOOM, MAX_ZOOM);
        let anchor_x = anchor.unwrap_or_else(|| clamp(self.x(editor.time), 0.0, self.width));
        let anchor_time = (anchor_x + self.scroll_x - PAD) / self.zoom;
        self.zoom = next;
        self.scroll_x = (anchor_time * next + PAD - anchor_x).max(0.0);
        self.clamp_scroll(editor);
    }

    pub fn zoom_to_fit(&mut self, editor: &Editor) {
        let end = editor.playback_end().max(5.0);
        self.zoom = clamp((self.width - PAD * 2.0 - 40.0) / end, MIN_ZOOM, MAX_ZOOM);
        self.scroll_x = 0.0;
        self.zoom_target = None;
        self.scroll_target = None;
    }

    /// While playing, keep the playhead in view.
    pub fn follow(&mut self, time: f64, editor: &Editor) {
        let x = self.x(time);
        if x > self.width - 60.0 || x < 0.0 {
            self.scroll_target = None;
            self.scroll_x = (PAD + time * self.zoom - 80.0).max(0.0);
            self.clamp_scroll(editor);
        }
    }

    fn scroll_limits(&self, editor: &Editor) -> (f64, f64) {
        ((self.content_width(editor) - self.width).max(0.0), (Self::rows_height(&editor.project.tracks) - self.area_height()).max(0.0))
    }

    /// Wheel and touchpad input. It sets where the view goes; `animate` glides there.
    pub fn scroll(&mut self, dx: f64, dy: f64, control: bool, shift: bool, x: f64, editor: &Editor) {
        if control {
            let (zoom, _) = self.zoom_target.unwrap_or((self.zoom, x));
            self.zoom_target = Some((clamp(zoom * (dy * 0.0015 * 3.0).exp(), MIN_ZOOM, MAX_ZOOM), x));
            self.scroll_target = None;
            return;
        }
        let (mut tx, mut ty) = self.scroll_target.unwrap_or((self.scroll_x, self.scroll_y));
        let overflows = Self::rows_height(&editor.project.tracks) > self.area_height() + 1.0;
        if shift || (dx.abs() < dy.abs() && !overflows) {
            tx -= if dy != 0.0 { dy } else { dx };
        } else {
            tx -= dx;
            ty -= dy;
        }
        let (max_x, max_y) = self.scroll_limits(editor);
        self.scroll_target = Some((tx.clamp(0.0, max_x), ty.clamp(0.0, max_y)));
    }

    /// Moves smooth scrolling and zooming one step; true while something moved.
    pub fn animate(&mut self, editor: &Editor) -> bool {
        let now = Instant::now();
        let dt = self.last_step.map(|last| (now - last).as_secs_f64()).unwrap_or(1.0 / 60.0).min(0.05);
        self.last_step = Some(now);
        if self.scroll_target.is_none() && self.zoom_target.is_none() {
            self.last_step = None;
            return false;
        }
        let alpha = 1.0 - (-dt * GLIDE).exp();
        if let Some((target, anchor)) = self.zoom_target {
            let step = (target / self.zoom).ln() * alpha;
            if (target / self.zoom).ln().abs() < 0.002 {
                self.apply_zoom(target, Some(anchor), editor);
                self.zoom_target = None;
            } else {
                self.apply_zoom(self.zoom * step.exp(), Some(anchor), editor);
            }
        }
        if let Some((tx, ty)) = self.scroll_target {
            let (max_x, max_y) = self.scroll_limits(editor);
            let (tx, ty) = (tx.clamp(0.0, max_x), ty.clamp(0.0, max_y));
            if (tx - self.scroll_x).abs() < 0.5 && (ty - self.scroll_y).abs() < 0.5 {
                self.scroll_x = tx;
                self.scroll_y = ty;
                self.scroll_target = None;
            } else {
                self.scroll_x += (tx - self.scroll_x) * alpha;
                self.scroll_y += (ty - self.scroll_y) * alpha;
            }
            self.clamp_scroll(editor);
        }
        true
    }

    // ------------------------------------------------------------ hit testing

    fn row_at(&self, tracks: &[Track], y: f64) -> Option<usize> {
        let y = y + self.scroll_y;
        Self::rows(tracks).iter().position(|(top, h)| y >= top - GAP / 2.0 && y < top + h + GAP / 2.0)
    }

    pub fn hit(&self, editor: &Editor, x: f64, y: f64) -> Hit {
        let tracks = &editor.project.tracks;
        let rows = Self::rows(tracks);
        let Some(t) = self.row_at(tracks, y) else { return Hit::Empty };
        let (top, height) = rows[t];
        let track = &tracks[t];
        let local_y = y + self.scroll_y - top;
        // Transitions sit on top of clip edges.
        if track.kind == TrackKind::Video {
            for seam in track_seams(track, editor.fps()) {
                let cx = self.x(seam.cut);
                if seam.kind != vf_core::model::TransitionKind::None {
                    let half = seam.duration * self.zoom / 2.0;
                    let inside_y = local_y >= height * 0.23 && local_y <= height * 0.77;
                    if inside_y && (x - (cx - half)).abs() <= 4.0 {
                        return Hit::SeamEdge(seam.to, Edge::Start);
                    }
                    if inside_y && (x - (cx + half)).abs() <= 4.0 {
                        return Hit::SeamEdge(seam.to, Edge::End);
                    }
                    if inside_y && x >= cx - half && x <= cx + half {
                        return Hit::Seam(seam.to);
                    }
                } else if self.hover_row.as_deref() == Some(track.id.as_str()) && (x - cx).abs() <= 10.0 && (local_y - height / 2.0).abs() <= 10.0 {
                    return Hit::Seam(seam.to);
                }
            }
        }
        for item in track.items.iter().rev() {
            let left = self.x(item.start);
            let right = self.x(item.end());
            if x < left || x > right {
                continue;
            }
            let editable = !item.locked && !track.locked;
            if item.kind != ItemKind::Audio && local_y >= height - 10.0 {
                for keyframe in sorted_keyframes(item) {
                    if (x - (left + keyframe.time * self.zoom)).abs() <= 5.0 {
                        return Hit::Keyframe(item.id.clone(), keyframe.id.clone(), keyframe.time);
                    }
                }
            }
            if has_effect(item) && local_y >= height - 18.0 {
                let window = effect_window(item.duration, item.effect_range);
                let ex0 = left + window.start * self.zoom;
                let ex1 = left + (window.start + window.duration) * self.zoom;
                if x >= ex0 && x <= ex1.max(ex0 + 4.0) {
                    if editable && (x - ex0) <= 6.0 {
                        return Hit::Effect(item.id.clone(), EffectPart::Start);
                    }
                    if editable && (ex1 - x) <= 6.0 {
                        return Hit::Effect(item.id.clone(), EffectPart::End);
                    }
                    return Hit::Effect(item.id.clone(), EffectPart::Body);
                }
            }
            if editable && x - left <= TRIM_PX && right - left > TRIM_PX * 2.5 {
                return Hit::Trim(item.id.clone(), Edge::Start);
            }
            if editable && right - x <= TRIM_PX && right - left > TRIM_PX * 2.5 {
                return Hit::Trim(item.id.clone(), Edge::End);
            }
            return Hit::Clip(item.id.clone());
        }
        Hit::Empty
    }

    pub fn hover(&mut self, editor: &Editor, x: f64, y: f64) {
        self.hover_row = self.row_at(&editor.project.tracks, y).map(|t| editor.project.tracks[t].id.clone());
        self.cursor = match self.hit(editor, x, y) {
            Hit::Clip(id) => {
                if editor.editable(&id).is_some() {
                    1
                } else {
                    3
                }
            }
            Hit::Trim(..) | Hit::SeamEdge(..) | Hit::Keyframe(..) => 2,
            Hit::Effect(_, EffectPart::Body) => 1,
            Hit::Effect(..) => 2,
            Hit::Seam(..) => 4,
            Hit::Empty => 0,
        };
    }

    // ------------------------------------------------------------ snapping

    fn snap_points(editor: &Editor, exclude: &HashSet<String>) -> Vec<f64> {
        let mut points = vec![0.0, editor.time];
        for item in editor.project.items() {
            if !exclude.contains(&item.id) {
                points.push(item.start);
                points.push(item.end());
            }
        }
        points
    }

    fn snap(&self, value: f64, points: &[f64], disabled: bool) -> (f64, Option<f64>) {
        if !self.snapping || disabled {
            return (value, None);
        }
        let threshold = SNAP_PX / self.zoom;
        let mut best = value;
        let mut distance = threshold;
        for point in points {
            let delta = (point - value).abs();
            if delta < distance {
                distance = delta;
                best = *point;
            }
        }
        (best, (best != value).then_some(best))
    }

    // ------------------------------------------------------------ ruler

    pub fn ruler_pointer(&mut self, editor: &mut Editor, phase: crate::PointerPhase, x: f64, shift: bool) -> bool {
        use crate::PointerPhase as P;
        match phase {
            P::Down => {
                self.drag = Some(Drag::Scrub);
                self.scrub(editor, x, shift);
                true
            }
            P::Move if matches!(self.drag, Some(Drag::Scrub)) => {
                self.scrub(editor, x, shift);
                true
            }
            P::Up => {
                self.drag = None;
                self.snap_line = None;
                true
            }
            _ => false,
        }
    }

    fn scrub(&mut self, editor: &mut Editor, x: f64, shift: bool) {
        let mut points = Self::snap_points(editor, &HashSet::new());
        points.remove(1);
        let (time, line) = self.snap(self.time_at(x), &points, shift);
        self.snap_line = line;
        editor.seek(time);
    }

    // ------------------------------------------------------------ lanes

    /// Handles a pointer event in the lanes; returns whether the project or view changed.
    // The arguments are the Slint callback's, passed straight through.
    #[allow(clippy::too_many_arguments)]
    pub fn lanes_pointer(
        &mut self,
        editor: &mut Editor,
        phase: crate::PointerPhase,
        x: f64,
        y: f64,
        shift: bool,
        control: bool,
        playing: bool,
        requests: &mut Vec<Request>,
    ) -> bool {
        use crate::PointerPhase as P;
        match phase {
            P::Down => self.press(editor, x, y, shift, control, requests),
            P::Move => self.drag_to(editor, x, y, shift),
            P::Up => self.release(editor, x, playing, requests),
            P::Double => false,
        }
    }

    fn press(&mut self, editor: &mut Editor, x: f64, y: f64, shift: bool, control: bool, requests: &mut Vec<Request>) -> bool {
        let additive = shift || control;
        match self.hit(editor, x, y) {
            Hit::Empty => {
                self.drag = Some(Drag::Marquee {
                    x0: x + self.scroll_x,
                    y0: y + self.scroll_y,
                    x1: x + self.scroll_x,
                    y1: y + self.scroll_y,
                    additive,
                    previous: editor.selection.clone(),
                    moved: false,
                });
                false
            }
            Hit::Clip(id) => {
                if additive {
                    editor.select(Some(&id), true);
                    return true;
                }
                let was_selected = editor.selection.contains(&id);
                if !was_selected {
                    editor.select(Some(&id), false);
                } else {
                    editor.primary = Some(id.clone());
                }
                let Some((t, _)) = editor.editable(&id) else { return true };
                let ids: Vec<String> = editor.selection.iter().filter(|s| editor.editable(s).is_some()).cloned().collect();
                let origins = ids.iter().filter_map(|i| editor.item(i).map(|item| (i.clone(), item.start))).collect();
                let kind = editor.project.tracks[t].kind;
                self.drag = Some(Drag::Clip { item: id, ids, origins, start_x: x, start_y: y, start_scroll: self.scroll_x, moved: false, was_selected, kind });
                true
            }
            Hit::Trim(id, edge) => {
                editor.select(Some(&id), false);
                let Some(item) = editor.item(&id) else { return true };
                self.drag = Some(Drag::Trim { item: id.clone(), edge, origin: TrimOrigin::of(item), start_x: x, start_scroll: self.scroll_x, moved: false });
                true
            }
            Hit::Keyframe(item, keyframe, time) => {
                editor.select(Some(&item), false);
                if let Some(start) = editor.item(&item).map(|i| i.start) {
                    editor.seek(start + time);
                }
                self.drag = Some(Drag::Keyframe { item, keyframe, time, start_x: x, moved: false });
                true
            }
            Hit::Effect(id, part) => {
                editor.select(Some(&id), false);
                if editor.editable(&id).is_none() {
                    requests.push(Request::OpenPanel("effects"));
                    return true;
                }
                let Some(item) = editor.item(&id) else { return true };
                let origin = effect_window(item.duration, item.effect_range);
                self.drag = Some(Drag::Effect { item: id, part, origin, start_x: x, moved: false });
                true
            }
            Hit::Seam(to) => {
                editor.select(Some(&to), false);
                requests.push(Request::OpenPanel("transitions"));
                true
            }
            Hit::SeamEdge(to, edge) => {
                editor.select(Some(&to), false);
                let Some((t, i)) = editor.find(&to) else { return true };
                let track = &editor.project.tracks[t];
                let Some(seam) = track_seams(track, editor.fps()).into_iter().find(|s| s.to == to) else { return true };
                let from_duration = editor.item(&seam.from).map(|i| i.duration).unwrap_or(0.0);
                let limit = MAX_TRANSITION.min(from_duration).min(track.items[i].duration);
                self.drag = Some(Drag::Seam { item: to, edge, cut: seam.cut, limit, kind: seam.kind, moved: false });
                true
            }
        }
    }

    fn drag_to(&mut self, editor: &mut Editor, x: f64, y: f64, shift: bool) -> bool {
        // Scroll when the pointer reaches an edge.
        if self.drag.as_ref().is_some_and(|d| !matches!(d, Drag::Scrub | Drag::Track { .. })) {
            self.scroll_target = None;
            if x > self.width - 40.0 {
                self.scroll_x += ((x - (self.width - 40.0)) / 2.0).min(24.0);
            } else if x < 40.0 {
                self.scroll_x -= ((40.0 - x) / 2.0).min(24.0);
            }
            self.clamp_scroll(editor);
        }
        let zoom = self.zoom;
        let scroll_x = self.scroll_x;
        let Some(drag) = self.drag.as_mut() else { return false };
        match drag {
            Drag::Scrub => false,
            Drag::Clip { item, ids, origins, start_x, start_y, start_scroll, moved, kind, .. } => {
                if !*moved {
                    if (x - *start_x).abs() < 3.0 && (y - *start_y).abs() < 3.0 {
                        return false;
                    }
                    *moved = true;
                    editor.begin_change();
                }
                let dt = (x - *start_x + scroll_x - *start_scroll) / zoom;
                let (item, ids, origins, kind) = (item.clone(), ids.clone(), origins.clone(), *kind);
                let Some(duration) = editor.item(&item).map(|i| i.duration) else { return false };
                let origin = origins.iter().find(|(id, _)| *id == item).map(|(_, s)| *s).unwrap_or(0.0);
                let exclude: HashSet<String> = ids.iter().cloned().collect();
                let points = Self::snap_points(editor, &exclude);
                let mut start = (origin + dt).max(0.0);
                let head = self.snap(start, &points, shift);
                let tail = self.snap(start + duration, &points, shift);
                match (head.1, tail.1) {
                    (Some(line), _) if tail.1.is_none() || (head.0 - start).abs() <= (tail.0 - start - duration).abs() => {
                        start = head.0;
                        self.snap_line = Some(line);
                    }
                    (_, Some(line)) => {
                        start = tail.0 - duration;
                        self.snap_line = Some(line);
                    }
                    _ => self.snap_line = None,
                }
                let delta = start - origin;
                if ids.len() == 1 {
                    let target =
                        self.row_at(&editor.project.tracks, y).map(|t| &editor.project.tracks[t]).filter(|t| t.kind == kind && !t.locked).map(|t| t.id.clone());
                    editor.move_item(&item, start, target.as_deref());
                } else {
                    let moves: Vec<(String, f64)> = origins.iter().map(|(id, s)| (id.clone(), (s + delta).max(0.0))).collect();
                    editor.move_items(&moves);
                }
                true
            }
            Drag::Trim { item, edge, origin, start_x, start_scroll, moved } => {
                if !*moved {
                    *moved = true;
                    editor.begin_change();
                }
                let dt = (x - *start_x + scroll_x - *start_scroll) / zoom;
                let raw = match edge {
                    Edge::Start => origin.start + dt,
                    Edge::End => origin.start + origin.duration + dt,
                };
                let (item, edge, origin) = (item.clone(), *edge, *origin);
                let exclude: HashSet<String> = [item.clone()].into_iter().collect();
                let points = Self::snap_points(editor, &exclude);
                let (value, line) = self.snap(raw, &points, shift);
                self.snap_line = line;
                editor.trim_item(&item, edge, value, origin);
                true
            }
            Drag::Keyframe { item, keyframe, time, start_x, moved } => {
                if !*moved {
                    *moved = true;
                    editor.begin_change();
                }
                let Some((start, duration)) = editor.item(item).map(|i| (i.start, i.duration)) else { return false };
                let next = clamp(*time + (x - *start_x) / zoom, 0.0, duration);
                editor.move_keyframe(item, keyframe, next);
                editor.seek(start + next);
                true
            }
            Drag::Marquee { x0, y0, x1, y1, additive, previous, moved } => {
                let (cx, cy) = (x + scroll_x, y + self.scroll_y);
                if !*moved && ((cx - *x0).powi(2) + (cy - *y0).powi(2)).sqrt() < 4.0 {
                    return false;
                }
                *moved = true;
                *x1 = cx;
                *y1 = cy;
                let (left, right) = (x0.min(*x1), x0.max(*x1));
                let (top, bottom) = (y0.min(*y1), y0.max(*y1));
                let mut ids = if *additive { previous.clone() } else { vec![] };
                for ((row_top, row_h), track) in Self::rows(&editor.project.tracks).iter().zip(&editor.project.tracks) {
                    if row_top + row_h < top || *row_top > bottom {
                        continue;
                    }
                    for item in &track.items {
                        let ix0 = PAD + item.start * zoom;
                        let ix1 = PAD + item.end() * zoom;
                        if ix0 < right && ix1 > left {
                            ids.push(item.id.clone());
                        }
                    }
                }
                editor.select_many(&ids);
                true
            }
            Drag::Effect { item, part, origin, start_x, moved } => {
                if !*moved {
                    if (x - *start_x).abs() < 3.0 {
                        return false;
                    }
                    *moved = true;
                    editor.begin_change();
                }
                let dt = (x - *start_x) / zoom;
                let (item, part, origin) = (item.clone(), *part, *origin);
                let Some(clip) = editor.item(&item).cloned() else { return false };
                let points: Vec<f64> =
                    Self::snap_points(editor, &HashSet::new()).into_iter().filter(|p| *p >= clip.start - 1e-6 && *p <= clip.end() + 1e-6).collect();
                let end = origin.start + origin.duration;
                let (mut start, mut duration) = (origin.start, origin.duration);
                match part {
                    EffectPart::Body => {
                        start = clamp(origin.start + dt, 0.0, clip.duration - origin.duration);
                        let head = self.snap(clip.start + start, &points, shift);
                        let tail = self.snap(clip.start + start + duration, &points, shift);
                        if let Some(line) = head.1 {
                            start = head.0 - clip.start;
                            self.snap_line = Some(line);
                        } else if let Some(line) = tail.1 {
                            start = tail.0 - clip.start - duration;
                            self.snap_line = Some(line);
                        } else {
                            self.snap_line = None;
                        }
                        start = clamp(start, 0.0, clip.duration - duration);
                    }
                    EffectPart::Start => {
                        let (value, line) = self.snap(clip.start + clamp(origin.start + dt, 0.0, end - MIN_EFFECT), &points, shift);
                        self.snap_line = line;
                        start = clamp(value - clip.start, 0.0, end - MIN_EFFECT);
                        duration = end - start;
                    }
                    EffectPart::End => {
                        let (value, line) = self.snap(clip.start + clamp(end + dt, origin.start + MIN_EFFECT, clip.duration), &points, shift);
                        self.snap_line = line;
                        duration = clamp(value - clip.start, origin.start + MIN_EFFECT, clip.duration) - origin.start;
                    }
                }
                editor.set_effect_range(&item, EffectRange { start, duration }, false);
                true
            }
            Drag::Seam { item, edge, cut, limit, kind, moved } => {
                if !*moved {
                    *moved = true;
                    editor.begin_change();
                }
                let time = ((x + scroll_x - PAD) / zoom).max(0.0);
                let half = match edge {
                    Edge::End => time - *cut,
                    Edge::Start => *cut - time,
                };
                let duration = clamp((half * 2.0 * 20.0).round() / 20.0, MIN_TRANSITION, limit.max(MIN_TRANSITION));
                editor.set_transition(item, *kind, Some(duration), false);
                true
            }
            Drag::Track { .. } => false,
        }
    }

    fn release(&mut self, editor: &mut Editor, x: f64, playing: bool, requests: &mut Vec<Request>) -> bool {
        self.snap_line = None;
        let Some(drag) = self.drag.take() else { return false };
        match drag {
            Drag::Clip { item, moved, was_selected, .. } => {
                if !moved {
                    let grouped = editor.item(&item).is_some_and(|i| i.group_id.is_some());
                    if was_selected && editor.selection.len() > 1 && !grouped {
                        editor.select(Some(&item), false);
                    }
                    if !playing {
                        if let Some(clip) = editor.item(&item) {
                            if editor.time < clip.start || editor.time >= clip.end() {
                                let start = clip.start;
                                editor.seek(start);
                            }
                        }
                    }
                }
                true
            }
            Drag::Marquee { additive, moved, .. } => {
                if !moved {
                    if !additive {
                        editor.select(None, false);
                    }
                    editor.seek(self.time_at(x));
                }
                true
            }
            Drag::Effect { moved: false, .. } => {
                requests.push(Request::OpenPanel("effects"));
                true
            }
            _ => true,
        }
    }

    /// Escape during a drag: undo what it did.
    pub fn cancel(&mut self, editor: &mut Editor) -> bool {
        let Some(drag) = self.drag.take() else { return false };
        self.snap_line = None;
        let moved = match drag {
            Drag::Clip { moved, .. } | Drag::Trim { moved, .. } | Drag::Keyframe { moved, .. } | Drag::Effect { moved, .. } | Drag::Seam { moved, .. } => moved,
            Drag::Marquee { previous, .. } => {
                editor.select_many(&previous);
                false
            }
            _ => false,
        };
        if moved {
            editor.undo();
        }
        true
    }

    // ------------------------------------------------------------ track headers

    pub fn header_pointer(&mut self, editor: &mut Editor, phase: crate::PointerPhase, id: &str, y: f64) -> bool {
        use crate::PointerPhase as P;
        let rows = Self::rows(&editor.project.tracks);
        let index_at = |y: f64| rows.iter().position(|(top, h)| y + 0.0 < top + h / 2.0).unwrap_or(rows.len());
        match phase {
            P::Down => {
                self.drag = Some(Drag::Track { id: id.into(), start_y: y, index: index_at(y), moved: false });
                false
            }
            P::Move => {
                let Some(Drag::Track { start_y, index, moved, .. }) = self.drag.as_mut() else { return false };
                if !*moved && (y - *start_y).abs() < 4.0 {
                    return false;
                }
                *moved = true;
                *index = index_at(y);
                true
            }
            P::Up => {
                let Some(Drag::Track { id, index, moved, .. }) = self.drag.take() else { return false };
                if !moved {
                    return true;
                }
                let Some(from) = editor.project.tracks.iter().position(|t| t.id == id) else { return true };
                let to = if index > from { index - 1 } else { index };
                if to != from {
                    editor.move_track(&id, to);
                }
                true
            }
            P::Double => false,
        }
    }

    fn track_drop_line(&self, tracks: &[Track]) -> Option<f64> {
        let Some(Drag::Track { id, index, moved: true, .. }) = &self.drag else { return None };
        let from = tracks.iter().position(|t| &t.id == id)?;
        if *index == from || *index == from + 1 {
            return None;
        }
        let rows = Self::rows(tracks);
        let y = match rows.get(*index) {
            Some((top, _)) => top - GAP / 2.0,
            None => rows.last().map(|(top, h)| top + h + GAP / 2.0).unwrap_or(TOP),
        };
        Some(y - self.scroll_y)
    }

    // ------------------------------------------------------------ drops from the panels

    /// Target of a panel drag at lanes point (`x`, `y`).
    pub fn drop_target(&self, editor: &Editor, kind: DropKind, x: f64, y: f64) -> Option<DropTarget> {
        let tracks = &editor.project.tracks;
        let rows = Self::rows(tracks);
        let row = self.row_at(tracks, y);
        let area_y = y + self.scroll_y;
        let inside = |t: usize| area_y >= rows[t].0 && area_y <= rows[t].0 + rows[t].1;
        let time = self.time_at(x);
        match kind {
            DropKind::Transition => {
                let t = row?;
                let track = &tracks[t];
                if track.locked {
                    return None;
                }
                track_seams(track, editor.fps())
                    .into_iter()
                    .find(|seam| {
                        (seam.cut - time).abs() * self.zoom < 40.0
                            && editor.item(&seam.to).is_some_and(|i| !i.locked)
                            && editor.item(&seam.from).is_some_and(|i| !i.locked)
                    })
                    .map(|seam| DropTarget::Seam(seam.to))
            }
            DropKind::Effect => {
                let t = row.filter(|&t| inside(t))?;
                let track = &tracks[t];
                if track.locked {
                    return None;
                }
                track
                    .items
                    .iter()
                    .find(|item| {
                        time >= item.start && time < item.end() && !item.locked && matches!(item.kind, ItemKind::Video | ItemKind::Image | ItemKind::Text)
                    })
                    .map(|item| DropTarget::Clip(item.id.clone()))
            }
            DropKind::Media { kind, .. } => {
                if let Some(t) = row.filter(|&t| inside(t)) {
                    let track = &tracks[t];
                    if track.kind == kind.track_kind() && !track.locked {
                        return Some(DropTarget::Track { track_id: track.id.clone(), start: time });
                    }
                }
                let index = rows.iter().position(|(top, h)| area_y < top + h / 2.0).unwrap_or(rows.len());
                Some(DropTarget::New { index, start: time })
            }
        }
    }

    /// Shows (or with `None` hides) where a panel drag would land.
    pub fn show_drop(&mut self, editor: &Editor, target: Option<&DropTarget>, duration: f64) {
        self.drop_hint = None;
        self.drop_new = None;
        self.drop_clip = None;
        self.drop_seam = None;
        let tracks = &editor.project.tracks;
        match target {
            Some(DropTarget::Seam(id)) => self.drop_seam = Some(id.clone()),
            Some(DropTarget::Clip(id)) => self.drop_clip = Some(id.clone()),
            Some(DropTarget::Track { track_id, start }) => {
                // Shown where the clip will really go (it avoids overlaps).
                let start = tracks
                    .iter()
                    .find(|t| &t.id == track_id)
                    .and_then(|track| {
                        vf_core::timeline::place_span(
                            &vf_core::timeline::spans_of(&track.items),
                            &vf_core::timeline::Span::new("drop", *start, duration),
                            vf_core::timeline::HORIZON,
                            &[],
                        )
                    })
                    .map(|span| span.start)
                    .unwrap_or(*start);
                self.drop_hint = Some((track_id.clone(), start, duration));
            }
            Some(DropTarget::New { index, start }) => {
                let rows = Self::rows(tracks);
                let y = match rows.get(*index) {
                    Some((top, _)) => top - GAP / 2.0,
                    None => rows.last().map(|(top, h)| top + h + GAP / 2.0).unwrap_or(TOP),
                };
                self.drop_new = Some((y - self.scroll_y, *start, duration));
            }
            None => {}
        }
    }

    // ------------------------------------------------------------ context menu

    pub fn context(&mut self, editor: &mut Editor, x: f64, y: f64) -> Vec<MenuEntry> {
        let hit = self.hit(editor, x, y);
        self.context_time = self.time_at(x);
        let entry = |id: &str, label: &str, shortcut: &str, enabled: bool, danger: bool| MenuEntry {
            id: id.into(),
            label: label.into(),
            shortcut: crate::shortcut_label(shortcut).into(),
            enabled,
            danger,
            separator: false,
        };
        let separator = || MenuEntry { separator: true, ..Default::default() };
        let entries = match &hit {
            Hit::Empty => {
                vec![entry("paste", "Paste here", "Ctrl+V", editor.has_clipboard(), false), entry("select-all", "Select all", "Ctrl+A", true, false)]
            }
            Hit::Clip(id) | Hit::Trim(id, _) | Hit::Keyframe(id, ..) | Hit::Effect(id, _) => {
                if !editor.selection.contains(id) {
                    editor.select(Some(id), false);
                }
                let item = editor.item(id).cloned().unwrap_or_default();
                let editable = editor.editable(id).is_some();
                let multiple = editor.selection.len() > 1;
                let mut list = vec![
                    entry("split", "Split at playhead", "S", editable, false),
                    entry("copy", "Copy", "Ctrl+C", true, false),
                    entry("cut", "Cut", "Ctrl+X", editable, false),
                    entry("duplicate", "Duplicate", "Ctrl+D", editable, false),
                    separator(),
                ];
                if multiple {
                    list.push(entry("group", "Group", "Ctrl+G", true, false));
                }
                if item.group_id.is_some() {
                    list.push(entry("ungroup", "Ungroup", "Ctrl+Shift+G", true, false));
                }
                if item.kind.has_sound() {
                    list.push(entry("mute", if item.muted { "Unmute" } else { "Mute" }, "", editable, false));
                }
                list.push(entry("hide", if item.hidden { "Show" } else { "Hide" }, "H", editable, false));
                list.push(entry("lock", if item.locked { "Unlock" } else { "Lock" }, "", true, false));
                if has_effect(&item) {
                    list.push(entry("remove-effect", "Remove effect", "", editable, false));
                }
                list.push(separator());
                list.push(entry("delete", "Delete", "Del", editable, true));
                list.push(entry("ripple-delete", "Delete and close gap", "Shift+Del", editable, true));
                list
            }
            Hit::Seam(to) | Hit::SeamEdge(to, _) => {
                let has = editor.item(to).and_then(|i| i.transition).is_some_and(|t| t.kind != vf_core::model::TransitionKind::None);
                vec![entry("transition", "Transitions…", "", true, false), entry("remove-transition", "Remove transition", "", has, true)]
            }
        };
        self.context = Some(hit);
        entries
    }

    pub fn context_action(&mut self, editor: &mut Editor, action: &str, requests: &mut Vec<Request>) {
        use vf_core::edit::Flag;
        let hit = self.context.take().unwrap_or(Hit::Empty);
        match action {
            "paste" => {
                editor.time = self.context_time;
                editor.paste();
            }
            "select-all" => editor.select_all(),
            "split" => {
                let time = editor.time;
                editor.split_at(time);
            }
            "copy" => {
                editor.copy_selected();
            }
            "cut" => editor.cut_selected(),
            "duplicate" => editor.duplicate_selected(),
            "group" => editor.group_selected(),
            "ungroup" => editor.ungroup_selected(),
            "mute" => editor.toggle_item_flag(Flag::Muted),
            "hide" => editor.toggle_item_flag(Flag::Hidden),
            "lock" => editor.toggle_item_flag(Flag::Locked),
            "remove-effect" => {
                let ids = editor.selection.clone();
                editor.set_effect(&ids, EffectKind::None, None);
            }
            "delete" => editor.delete_selected(false),
            "ripple-delete" => editor.delete_selected(true),
            "transition" => requests.push(Request::OpenPanel("transitions")),
            "remove-transition" => {
                if let Hit::Seam(to) | Hit::SeamEdge(to, _) = hit {
                    editor.set_transition(&to, vf_core::model::TransitionKind::None, None, true);
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ models

    /// Pushes everything the timeline shows to Slint.
    pub fn sync(&mut self, editor: &Editor, state: &TimelineState) {
        let project = &editor.project;
        let tracks = &project.tracks;
        let rows = Self::rows(tracks);
        let selection: HashSet<&str> = editor.selection.iter().map(String::as_str).collect();
        let dragging_track = match &self.drag {
            Some(Drag::Track { id, moved: true, .. }) => Some(id.clone()),
            _ => None,
        };
        let row_models: Vec<TrackRow> = tracks
            .iter()
            .zip(&rows)
            .map(|(track, (top, height))| TrackRow {
                id: track.id.clone().into(),
                name: track.name.clone().into(),
                kind: match track.kind {
                    TrackKind::Video => 0,
                    TrackKind::Text => 1,
                    TrackKind::Audio => 2,
                },
                top: (top - self.scroll_y) as f32,
                height: *height as f32,
                hidden: track.hidden,
                muted: track.muted,
                locked: track.locked,
                dragging: dragging_track.as_deref() == Some(track.id.as_str()),
                drop_hint: self.drop_hint.as_ref().is_some_and(|(id, ..)| id == &track.id),
            })
            .collect();

        let view_start = self.time_at(0.0) - 2.0;
        let view_end = self.time_at(self.width) + 2.0;
        let tolerance = editor.keyframe_tolerance();
        let mut clips = Vec::new();
        let mut keyframes = Vec::new();
        let mut seams = Vec::new();
        for ((track, (top, height)), t) in tracks.iter().zip(&rows).zip(0..) {
            let y = top - self.scroll_y;
            if y + height < 0.0 || y > self.area_height() {
                continue;
            }
            for item in &track.items {
                if item.end() < view_start || item.start > view_end {
                    continue;
                }
                let left = self.x(item.start);
                let width = (item.duration * self.zoom).max(2.0);
                let asset = project.asset(item.asset_id.as_deref());
                let color = clip_color(item.kind);
                // Only the visible part of long clips gets a picture, composed in
                // blocks so that scrolling inside a block reuses it.
                let visible_from = (-left).max(0.0);
                let visible_to = (self.width - left).min(width).max(visible_from + 1.0);
                let from = (visible_from / PICTURE_BLOCK).floor() * PICTURE_BLOCK;
                let to = ((visible_to / PICTURE_BLOCK).ceil() * PICTURE_BLOCK).min(width).min(from + 4096.0).max(from + 1.0);
                let picture = if matches!(item.kind, ItemKind::Video | ItemKind::Image | ItemKind::Audio) {
                    self.overviews.picture(item, asset, self.zoom, from, to, height - 2.0)
                } else {
                    None
                };
                let window = effect_window(item.duration, item.effect_range);
                let label = if item.kind == ItemKind::Text {
                    item.text.as_ref().map(|t| t.content.lines().next().unwrap_or("").to_string()).unwrap_or_default()
                } else {
                    item.name.clone()
                };
                clips.push(ClipView {
                    id: item.id.clone().into(),
                    x: left as f32,
                    y: y as f32,
                    width: width as f32,
                    height: *height as f32,
                    kind: match item.kind {
                        ItemKind::Video => 0,
                        ItemKind::Image => 1,
                        ItemKind::Audio => 2,
                        ItemKind::Text => 3,
                        ItemKind::Shape => 4,
                    },
                    label: if label.is_empty() { item.name.clone().into() } else { label.into() },
                    color,
                    selected: selection.contains(item.id.as_str()),
                    locked: item.locked || track.locked,
                    hidden: item.hidden,
                    muted: item.muted,
                    missing: item.asset_id.is_some() && asset.is_none(),
                    speed: if (item.speed() - 1.0).abs() > 1e-6 { format!("{}×", (item.speed() * 100.0).round() / 100.0).into() } else { SharedString::new() },
                    fade_in: (item.fade_in() * self.zoom) as f32,
                    fade_out: (item.fade_out() * self.zoom) as f32,
                    has_picture: picture.is_some(),
                    picture: picture.unwrap_or_default(),
                    picture_x: from as f32,
                    picture_width: (to - from) as f32,
                    effect: has_effect(item),
                    effect_x: (window.start * self.zoom) as f32,
                    effect_width: (window.duration * self.zoom) as f32,
                    effect_label: EFFECTS.iter().find(|e| e.kind == item.effect()).map(|e| e.label).unwrap_or("Effect").into(),
                    drop_target: self.drop_clip.as_deref() == Some(item.id.as_str()),
                });
                if item.kind != ItemKind::Audio {
                    for keyframe in sorted_keyframes(item) {
                        keyframes.push(KeyMark {
                            x: (left + keyframe.time * self.zoom) as f32,
                            y: (y + height - 6.0) as f32,
                            active: (editor.time - item.start - keyframe.time).abs() <= tolerance,
                        });
                    }
                }
            }
            if track.kind == TrackKind::Video {
                let show_empty = self.transition_drag || self.hover_row.as_deref() == Some(track.id.as_str());
                for seam in track_seams(track, editor.fps()) {
                    if seam.cut < view_start || seam.cut > view_end {
                        continue;
                    }
                    let has = seam.kind != vf_core::model::TransitionKind::None;
                    if !has && !show_empty {
                        continue;
                    }
                    let active = editor.primary.as_deref() == Some(seam.to.as_str())
                        || matches!(&self.drag, Some(Drag::Seam { item, .. }) if item == &seam.to)
                        || self.drop_seam.as_deref() == Some(seam.to.as_str());
                    let (x, width, sy, sh) = if has {
                        let w = (seam.duration * self.zoom).max(14.0);
                        (self.x(seam.cut) - w / 2.0, w, y + height * 0.23, height * 0.54)
                    } else {
                        (self.x(seam.cut) - 10.0, 20.0, y + height / 2.0 - 10.0, 20.0)
                    };
                    let label = if has && (width > 56.0 || active) { format!("{:.1}s", seam.duration) } else { String::new() };
                    let editable = !track.locked && editor.item(&seam.to).is_some_and(|i| !i.locked);
                    seams.push(SeamView {
                        item: seam.to.clone().into(),
                        x: x as f32,
                        y: sy as f32,
                        width: width as f32,
                        height: sh as f32,
                        has,
                        label: label.into(),
                        active,
                        editable,
                    });
                }
            }
            let _ = t;
        }

        if !self.models.bound {
            self.models.bound = true;
            state.set_rows(ModelRc::from(self.models.rows.clone()));
            state.set_clips(ModelRc::from(self.models.clips.clone()));
            state.set_keyframes(ModelRc::from(self.models.keyframes.clone()));
            state.set_seams(ModelRc::from(self.models.seams.clone()));
            state.set_ticks(ModelRc::from(self.models.ticks.clone()));
        }
        let ticks = self.ticks(editor);
        sync_rows(&self.models.rows, row_models);
        sync_rows(&self.models.clips, clips);
        sync_rows(&self.models.keyframes, keyframes);
        sync_rows(&self.models.seams, seams);
        sync_rows(&self.models.ticks, ticks);
        state.set_rows_height(Self::rows_height(tracks) as f32);
        state.set_playhead(self.x(editor.time) as f32);
        state.set_snap_line(self.snap_line.map(|t| self.x(t) as f32).unwrap_or(-100.0));
        match &self.drag {
            Some(Drag::Marquee { x0, y0, x1, y1, moved: true, .. }) => {
                state.set_marquee_x((x0.min(*x1) - self.scroll_x) as f32);
                state.set_marquee_y((y0.min(*y1) - self.scroll_y) as f32);
                state.set_marquee_width((x1 - x0).abs() as f32);
                state.set_marquee_height((y1 - y0).abs() as f32);
            }
            _ => state.set_marquee_width(0.0),
        }
        match (&self.drop_hint, &self.drop_new) {
            (Some((track, start, duration)), _) => {
                if let Some(t) = tracks.iter().position(|c| &c.id == track) {
                    state.set_drop_x(self.x(*start) as f32);
                    state.set_drop_y((rows[t].0 - self.scroll_y) as f32);
                    state.set_drop_width((duration * self.zoom) as f32);
                    state.set_drop_height(rows[t].1 as f32);
                    state.set_drop_line(false);
                }
            }
            (None, Some((y, start, duration))) => {
                state.set_drop_x(self.x(*start) as f32);
                state.set_drop_y(*y as f32);
                state.set_drop_width((duration * self.zoom) as f32);
                state.set_drop_line(true);
            }
            _ => state.set_drop_x(-1.0),
        }
        state.set_track_drop_line(self.track_drop_line(tracks).map(|v| v as f32).unwrap_or(-100.0));
        state.set_snapping(self.snapping);
        state.set_zoom_log(self.zoom.ln() as f32);
        state.set_has_selection(!editor.selection.is_empty());
        state.set_has_primary(editor.primary.is_some());
        state.set_primary_visual(editor.selected_item().is_some_and(|i| i.kind != ItemKind::Audio));
        state.set_can_undo(editor.can_undo());
        state.set_can_redo(editor.can_redo());
        state.set_cursor(self.cursor);
        state.set_transition_drag(self.transition_drag);
    }

    fn ticks(&self, editor: &Editor) -> Vec<Tick> {
        let steps = [1.0 / 30.0, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0];
        let major = steps.into_iter().find(|step| step * self.zoom >= 90.0).unwrap_or(600.0);
        let minor = major
            / if major >= 1.0 && (major % 5.0).abs() < 1e-9 {
                5.0
            } else if major >= 1.0 {
                2.0
            } else {
                5.0
            };
        let start = self.time_at(0.0) - 2.0;
        let end = self.time_at(self.width) + 2.0;
        let from = (start / minor).floor().max(0.0) as i64;
        let to = (end / minor).ceil() as i64;
        let mut ticks = Vec::new();
        for index in from..=to {
            if ticks.len() >= 600 {
                break;
            }
            let time = index as f64 * minor;
            let is_major = ((time / major) - (time / major).round()).abs() < 1e-6;
            let label = if !is_major {
                String::new()
            } else if major < 1.0 {
                format::time(time, Some(editor.fps()))
            } else {
                format::time(time, None)
            };
            ticks.push(Tick { x: self.x(time) as f32, major: is_major, label: label.into() });
        }
        ticks
    }
}

pub fn has_effect(item: &TimelineItem) -> bool {
    item.effect() != EffectKind::None && !matches!(item.kind, ItemKind::Audio | ItemKind::Shape)
}

/// Clip colour: the kind colour mixed into the panel colour (30% for media with pictures).
fn clip_color(kind: ItemKind) -> Color {
    let hex = TRACK_COLORS.iter().find(|(k, _)| *k == kind).map(|(_, c)| *c).unwrap_or("#4f7cff");
    let [r, g, b] = vf_core::engine::color::hex_to_rgb(hex);
    let amount = if matches!(kind, ItemKind::Video | ItemKind::Image) { 0.30 } else { 0.62 };
    let panel = [0x17 as f64 / 255.0, 0x17 as f64 / 255.0, 0x1b as f64 / 255.0];
    let mix = |c: f64, p: f64| ((c * amount + p * (1.0 - amount)) * 255.0).round() as u8;
    Color::from_rgb_u8(mix(r, panel[0]), mix(g, panel[1]), mix(b, panel[2]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vf_core::model::{Project, TimelineItem};

    fn editor() -> Editor {
        let mut project = Project::default();
        let mut track = vf_core::presets::create_track(TrackKind::Video, &[]);
        track.items.push(TimelineItem { id: "a".into(), track_id: track.id.clone(), kind: ItemKind::Video, start: 0.0, duration: 120.0, ..Default::default() });
        project.tracks.push(track);
        Editor::new(project)
    }

    fn settle(view: &mut TimelineView, editor: &Editor) -> usize {
        let mut steps = 0;
        while view.animate(editor) && steps < 500 {
            steps += 1;
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        steps
    }

    #[test]
    fn wheel_glides_instead_of_jumping() {
        let editor = editor();
        let mut view = TimelineView::new(true);
        view.set_size(800.0, 300.0, &editor);
        view.set_zoom(20.0, Some(0.0), &editor);
        view.scroll(0.0, -300.0, false, true, 0.0, &editor);
        // Nothing moves until the frame loop steps the glide.
        assert_eq!(view.scroll_x, 0.0);
        assert!(view.animate(&editor));
        assert!(view.scroll_x > 0.0 && view.scroll_x < 300.0, "{}", view.scroll_x);
        let steps = settle(&mut view, &editor);
        assert!(steps > 2);
        assert!((view.scroll_x - 300.0).abs() < 1e-6, "{}", view.scroll_x);
    }

    #[test]
    fn wheel_zoom_keeps_the_time_under_the_pointer() {
        let editor = editor();
        let mut view = TimelineView::new(true);
        view.set_size(800.0, 300.0, &editor);
        view.set_zoom(20.0, Some(0.0), &editor);
        let anchor = 400.0;
        let time = view.time_at(anchor);
        view.scroll(0.0, 120.0, true, false, anchor, &editor);
        settle(&mut view, &editor);
        assert!(view.zoom > 20.0);
        assert!((view.time_at(anchor) - time).abs() < 0.05, "{} vs {}", view.time_at(anchor), time);
    }

    #[test]
    fn direct_moves_stop_a_glide() {
        let editor = editor();
        let mut view = TimelineView::new(true);
        view.set_size(800.0, 300.0, &editor);
        view.scroll(0.0, -500.0, false, true, 0.0, &editor);
        view.zoom_to_fit(&editor);
        assert!(!view.animate(&editor));
        assert_eq!(view.scroll_x, 0.0);
    }
}
