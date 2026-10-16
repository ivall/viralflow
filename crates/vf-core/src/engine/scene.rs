//! What is on screen at a moment: one `Layer` per visible clip, bottom first,
//! with every animation, effect and transition already applied. The preview
//! and the export both draw exactly these layers.

use super::animation::{motion_span, resolve_motion};
use super::color::{adjust_matrix, is_identity, multiply, ColorMatrix, IDENTITY};
use super::effects::{resolve_item_effect, EffectState};
use super::keyframes::resolve_transform;
use super::math::clamp;
use super::transitions::{resolve_transition, track_connections, Connection, TransitionState};
use crate::model::{BlendMode, Crop, Fit, ItemKind, MediaAsset, Point, Project, ShapeKind, TimelineItem, TrackKind};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MediaLayout {
    /// Source rectangle in source pixels.
    pub sx: f64,
    pub sy: f64,
    pub sw: f64,
    pub sh: f64,
    /// Size of the layer box in project pixels.
    pub box_w: f64,
    pub box_h: f64,
}

pub fn media_layout(source_w: f64, source_h: f64, crop: Option<&Crop>, fit: Fit, focus: Option<&Point>, canvas_w: f64, canvas_h: f64) -> MediaLayout {
    let c = crop.copied().unwrap_or_default();
    let sw = source_w.max(1.0);
    let sh = source_h.max(1.0);
    let cx = clamp(c.x, 0.0, 1.0) * sw;
    let cy = clamp(c.y, 0.0, 1.0) * sh;
    let cw = (sw - cx).min(clamp(c.w, 0.01, 1.0) * sw).max(1.0);
    let ch = (sh - cy).min(clamp(c.h, 0.01, 1.0) * sh).max(1.0);
    match fit {
        Fit::Fill => MediaLayout { sx: cx, sy: cy, sw: cw, sh: ch, box_w: canvas_w, box_h: canvas_h },
        Fit::Cover => {
            let k = (canvas_w / cw).max(canvas_h / ch);
            let vw = cw.min(canvas_w / k);
            let vh = ch.min(canvas_h / k);
            let focus = focus.copied().unwrap_or_default();
            let fx = clamp(focus.x, 0.0, 100.0) / 100.0;
            let fy = clamp(focus.y, 0.0, 100.0) / 100.0;
            MediaLayout { sx: cx + (cw - vw) * fx, sy: cy + (ch - vh) * fy, sw: vw, sh: vh, box_w: canvas_w, box_h: canvas_h }
        }
        Fit::Contain => {
            let k = (canvas_w / cw).min(canvas_h / ch);
            MediaLayout { sx: cx, sy: cy, sw: cw, sh: ch, box_w: cw * k, box_h: ch * k }
        }
    }
}

pub fn shape_box(item: &TimelineItem, canvas_w: f64, canvas_h: f64) -> (f64, f64) {
    let shape = item.shape.clone().unwrap_or_default();
    let width = clamp(shape.width, 0.2, 400.0) / 100.0 * canvas_w;
    let height = if shape.kind == ShapeKind::Circle && shape.width == shape.height { width } else { clamp(shape.height, 0.1, 400.0) / 100.0 * canvas_h };
    (width, height)
}

/// Seconds into the source file shown at timeline `time`.
pub fn media_time_at(item: &TimelineItem, time: f64, fps: u32, asset_duration: Option<f64>) -> f64 {
    let fps = fps.max(1) as f64;
    let elapsed = clamp(time - item.start, 0.0, (item.duration - 1.0 / fps).max(0.0));
    let mut value = item.source_in() + elapsed * item.speed();
    if let Some(duration) = asset_duration.filter(|d| *d > 0.0) {
        value = value.min((duration - 0.5 / fps).max(0.0));
    }
    value.max(0.0)
}

pub fn item_active_at(item: &TimelineItem, time: f64) -> bool {
    !item.hidden && time >= item.start && time < item.end()
}

/// Volume envelope of the fade-in and fade-out at `time`.
pub fn fade_gain(item: &TimelineItem, time: f64) -> f64 {
    let elapsed = time - item.start;
    let mut gain = 1.0;
    if item.fade_in() > 0.0 {
        gain *= clamp(elapsed / item.fade_in(), 0.0, 1.0);
    }
    if item.fade_out() > 0.0 {
        gain *= clamp((item.duration - elapsed) / item.fade_out(), 0.0, 1.0);
    }
    gain
}

pub fn asset_has_audio(asset: Option<&MediaAsset>) -> bool {
    asset.and_then(|asset| asset.has_audio).unwrap_or(true)
}

/// Last moment that shows pixels or plays sound.
pub fn content_end(project: &Project) -> f64 {
    let mut end: f64 = 0.0;
    for track in &project.tracks {
        if track.hidden && track.kind != TrackKind::Audio {
            continue;
        }
        for item in &track.items {
            if item.hidden {
                continue;
            }
            if item.kind == ItemKind::Audio && (track.muted || item.muted || item.volume() <= 0.0) {
                continue;
            }
            end = end.max(item.end());
        }
    }
    (end * 1000.0).round() / 1000.0
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub track: usize,
    pub index: usize,
    pub kind: ItemKind,
    pub box_w: f64,
    pub box_h: f64,
    /// Source rectangle (video and images).
    pub source: Option<MediaLayout>,
    /// Seconds into the source (video).
    pub media_time: Option<f64>,
    /// Centre in project pixels.
    pub cx: f64,
    pub cy: f64,
    pub scale: f64,
    /// Degrees, clockwise.
    pub rotation: f64,
    pub flip_x: bool,
    pub flip_y: bool,
    pub opacity: f64,
    /// Gaussian blur in project pixels.
    pub blur: f64,
    pub blend: BlendMode,
    pub radius: f64,
    pub matrix: Option<ColorMatrix>,
    pub vignette: f64,
    pub clip_left: f64,
    pub clip_right: f64,
    pub grain: f64,
    pub scanlines: bool,
    pub glitch: Vec<f64>,
}

/// Size of text boxes in project pixels; text layout lives in the renderer.
pub trait TextMeasure {
    fn text_box(&self, item: &TimelineItem, canvas_w: f64) -> (f64, f64);
}

pub fn item_box(project: &Project, item: &TimelineItem, text: &dyn TextMeasure) -> Option<((f64, f64), Option<MediaLayout>)> {
    let (w, h) = (project.width as f64, project.height as f64);
    match item.kind {
        ItemKind::Video | ItemKind::Image => {
            let asset = project.asset(item.asset_id.as_deref())?;
            let layout = media_layout(asset.width?, asset.height?, item.crop.as_ref(), item.fit.unwrap_or_default(), item.focus.as_ref(), w, h);
            Some(((layout.box_w, layout.box_h), Some(layout)))
        }
        ItemKind::Shape => Some((shape_box(item, w, h), None)),
        ItemKind::Text => Some((text.text_box(item, w), None)),
        ItemKind::Audio => None,
    }
}

pub struct LayerOptions<'a> {
    pub connection: Option<&'a Connection>,
    /// Static geometry only: no animations, effects or transitions (selection handles).
    pub static_only: bool,
}

pub fn build_layer(project: &Project, track: usize, index: usize, time: f64, text: &dyn TextMeasure, options: LayerOptions) -> Option<Layer> {
    let siblings = &project.tracks[track].items;
    let item = &siblings[index];
    let ((box_w, box_h), source) = item_box(project, item, text)?;
    let elapsed = time - item.start;
    let transform = resolve_transform(item, elapsed);
    let (motion, effect, transition) = if options.static_only {
        (super::animation::IDLE_MOTION, EffectState::default(), TransitionState::default())
    } else {
        let span = motion_span(index, siblings);
        let effect = if item.kind == ItemKind::Shape { EffectState::default() } else { resolve_item_effect(item, elapsed) };
        (resolve_motion(&span, time - span.start), effect, resolve_transition(options.connection, &item.id, time))
    };

    let mut matrix = IDENTITY;
    if matches!(item.kind, ItemKind::Video | ItemKind::Image) {
        matrix = adjust_matrix(&item.adjust.unwrap_or_default());
    }
    matrix = multiply(&effect.matrix, &matrix);
    matrix = multiply(&transition.matrix, &matrix);

    let asset_duration = project.asset(item.asset_id.as_deref()).and_then(|asset| asset.duration);
    let (w, h) = (project.width as f64, project.height as f64);
    let visual = matches!(item.kind, ItemKind::Video | ItemKind::Image);
    Some(Layer {
        track,
        index,
        kind: item.kind,
        box_w,
        box_h,
        source,
        media_time: (item.kind == ItemKind::Video).then(|| media_time_at(item, time, project.fps, asset_duration)),
        cx: (transform.x + motion.dx + effect.dx + transition.dx) / 100.0 * w,
        cy: (transform.y + motion.dy + effect.dy + transition.dy) / 100.0 * h,
        scale: transform.scale * motion.scale * effect.scale * transition.scale,
        rotation: transform.rotation + motion.rotation + transition.rotation,
        flip_x: item.flip_x,
        flip_y: item.flip_y,
        opacity: clamp(transform.opacity * motion.opacity * transition.opacity, 0.0, 1.0),
        blur: motion.blur.max(effect.blur).max(transition.blur),
        blend: item.blend.unwrap_or_default(),
        radius: if item.kind == ItemKind::Shape { 0.0 } else { item.radius() },
        matrix: (!is_identity(&matrix)).then_some(matrix),
        vignette: if visual { clamp(item.adjust.map(|a| a.vignette).unwrap_or(0.0), 0.0, 1.0) } else { 0.0 },
        clip_left: transition.clip_left,
        clip_right: transition.clip_right.min(motion.reveal),
        grain: effect.grain,
        scanlines: effect.scanlines,
        glitch: effect.glitch,
    })
}

/// Transition connections of every track (indexed like `project.tracks`).
pub fn scene_connections(project: &Project) -> Vec<Vec<Connection>> {
    project.tracks.iter().map(|track| track_connections(track, project.fps)).collect()
}

/// Visual layers at `time`, bottom first. Track 0 is the top track.
pub fn resolve_scene(project: &Project, connections: &[Vec<Connection>], time: f64, text: &dyn TextMeasure) -> Vec<Layer> {
    let mut layers = Vec::new();
    for track_index in (0..project.tracks.len()).rev() {
        let track = &project.tracks[track_index];
        if track.hidden || track.kind == TrackKind::Audio {
            continue;
        }
        let active = connections.get(track_index).and_then(|list| list.iter().find(|c| time >= c.start && time <= c.end));
        let mut entries: Vec<(usize, Option<&Connection>)> = Vec::new();
        if let Some(connection) = active {
            for id in [&connection.from, &connection.to] {
                if let Some(position) = track.items.iter().position(|item| &item.id == id) {
                    entries.push((position, Some(connection)));
                }
            }
        }
        let mut others: Vec<usize> = track
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.kind.is_visual() && item_active_at(item, time))
            .filter(|(_, item)| active.map(|c| item.id != c.from && item.id != c.to).unwrap_or(true))
            .map(|(index, _)| index)
            .collect();
        others.sort_by(|a, b| track.items[*a].start.total_cmp(&track.items[*b].start));
        entries.extend(others.into_iter().map(|index| (index, None)));
        for (index, connection) in entries {
            if let Some(layer) = build_layer(project, track_index, index, time, text, LayerOptions { connection, static_only: false }) {
                if layer.opacity > 0.001 {
                    layers.push(layer);
                }
            }
        }
    }
    layers
}

/// Corners of a layer box in project pixels (clockwise from top-left).
pub fn layer_quad(layer: &Layer) -> [(f64, f64); 4] {
    let (sin, cos) = layer.rotation.to_radians().sin_cos();
    let hw = layer.box_w * layer.scale / 2.0;
    let hh = layer.box_h * layer.scale / 2.0;
    [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)].map(|(x, y)| (layer.cx + x * cos - y * sin, layer.cy + x * sin + y * cos))
}
