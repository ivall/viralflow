//! Turns a project at a moment into pixels: resolves the scene, gathers the
//! pictures each layer needs (video frames, images, text) and composites them.
//!
//! In `Mode::Preview` pictures are fetched by background workers and a frame
//! may use the newest picture available (the UI never waits); video frames
//! come from a shared cache the workers fill ahead of the playhead. In
//! `Mode::Export` every picture is exact and fetched synchronously.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};

use vf_core::engine::color::hex_to_rgb;
use vf_core::engine::scene::{resolve_scene, scene_connections, Layer, TextMeasure};
use vf_core::model::{ItemKind, Project, ShapeKind as ModelShape, TimelineItem};
use vf_media::video::VideoDecoder;
use vf_media::RgbaImage;

use crate::compositor::{upload, write, Compositor, Content, DrawLayer, ShapeKind};
use crate::frames::{FrameCache, FrameSource};
use crate::{Gpu, TextEngine};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Preview,
    Export,
}

struct Picture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
    used: u64,
}

impl Picture {
    fn new(gpu: &Gpu, image: &RgbaImage, used: u64, label: &str) -> Self {
        let texture = upload(gpu, image.width, image.height, &image.data, label);
        let view = texture.create_view(&Default::default());
        Picture { texture, view, width: image.width, height: image.height, used }
    }

    /// Replaces the pixels, reusing the texture when the size matches.
    fn update(&mut self, gpu: &Gpu, image: &RgbaImage, label: &str) {
        if image.width == self.width && image.height == self.height {
            write(gpu, &self.texture, image.width, image.height, &image.data);
        } else {
            *self = Picture::new(gpu, image, self.used, label);
        }
    }
}

/// A video clip decoding in the background (preview) or inline (export).
struct VideoSlot {
    path: PathBuf,
    size: (u32, u32),
    source: Option<FrameSource>,
    decoder: Option<VideoDecoder>,
    /// Picture in the texture: media time (export) or cache entry (preview).
    shown: Option<f64>,
    shown_entry: Option<u64>,
    failed: bool,
    /// Render tick that last needed this clip.
    used: u64,
}

impl VideoSlot {
    fn new(path: PathBuf, size: (u32, u32), used: u64) -> Self {
        VideoSlot { path, size, source: None, decoder: None, shown: None, shown_entry: None, failed: false, used }
    }
}

struct ImageSlot {
    /// Size last decoded for: a clip shown much larger decodes again.
    size: (u32, u32),
    pending: Option<Receiver<Option<RgbaImage>>>,
}

/// Size of text boxes, cached by style (shaping is not free).
pub struct CachedMeasure<'a> {
    text: &'a TextEngine,
    cache: &'a Mutex<HashMap<u64, (f64, f64)>>,
}

impl TextMeasure for CachedMeasure<'_> {
    fn text_box(&self, item: &TimelineItem, canvas_w: f64) -> (f64, f64) {
        let style = item.text_style();
        let key = text_key(&style, canvas_w, 0.0);
        if let Some(size) = self.cache.lock().ok().and_then(|cache| cache.get(&key).copied()) {
            return size;
        }
        let size = self.text.measure(&style, canvas_w);
        if let Ok(mut cache) = self.cache.lock() {
            if cache.len() > 4096 {
                cache.clear();
            }
            cache.insert(key, size);
        }
        size
    }
}

fn text_key(style: &vf_core::model::TextStyle, canvas_w: f64, scale: f64) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(style).unwrap_or_default().hash(&mut hasher);
    canvas_w.to_bits().hash(&mut hasher);
    scale.to_bits().hash(&mut hasher);
    hasher.finish()
}

pub struct RenderStats {
    /// Every layer had its exact picture (no picture still decoding).
    pub complete: bool,
}

pub struct FrameRenderer {
    gpu: Gpu,
    mode: Mode,
    compositor: Compositor,
    text: Arc<TextEngine>,
    measures: Mutex<HashMap<u64, (f64, f64)>>,
    pictures: HashMap<String, Picture>,
    videos: HashMap<String, VideoSlot>,
    images: HashMap<String, ImageSlot>,
    /// Longest edge decoded for the preview.
    max_decode: u32,
    tick: u64,
    frames: Arc<Mutex<FrameCache>>,
    playing: bool,
}

impl FrameRenderer {
    pub fn new(gpu: &Gpu, text: Arc<TextEngine>, mode: Mode) -> Self {
        FrameRenderer {
            gpu: gpu.clone(),
            mode,
            compositor: Compositor::new(gpu),
            text,
            measures: Mutex::default(),
            pictures: HashMap::new(),
            videos: HashMap::new(),
            images: HashMap::new(),
            max_decode: if mode == Mode::Preview { 1920 } else { 8192 },
            tick: 0,
            frames: FrameCache::shared(),
            playing: false,
        }
    }

    /// While playing, the preview decodes further ahead.
    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    pub fn measure(&self) -> CachedMeasure<'_> {
        CachedMeasure { text: &self.text, cache: &self.measures }
    }

    pub fn text_engine(&self) -> &Arc<TextEngine> {
        &self.text
    }

    /// Visual layers at `time` (with the same text measuring the render uses).
    pub fn layers(&self, project: &Project, time: f64) -> Vec<Layer> {
        resolve_scene(project, &scene_connections(project), time, &self.measure())
    }

    /// Renders `project` at `time` into `target`; `k` = target px per project px.
    pub fn render(&mut self, project: &Project, time: f64, target: &wgpu::Texture, k: f64) -> RenderStats {
        self.tick += 1;
        let layers = self.layers(project, time);
        let mut complete = true;
        // Fetch every picture first, then draw.
        for layer in &layers {
            let item = &project.tracks[layer.track].items[layer.index];
            let ready = match item.kind {
                ItemKind::Video => self.video_picture(project, item, layer, k),
                ItemKind::Image => self.image_picture(project, item, layer, k),
                ItemKind::Text => self.text_picture(project, item, k),
                _ => true,
            };
            complete &= ready;
        }
        let mut draws = Vec::with_capacity(layers.len());
        for layer in &layers {
            let item = &project.tracks[layer.track].items[layer.index];
            let content = match item.kind {
                ItemKind::Shape => {
                    let shape = item.shape.clone().unwrap_or_default();
                    match shape.kind {
                        ModelShape::Blur => Content::BlurRegion { strength: shape.blur.max(0.0) },
                        kind => {
                            let [r, g, b] = hex_to_rgb(&shape.color);
                            let kind = match kind {
                                ModelShape::Circle => ShapeKind::Circle,
                                ModelShape::Line => ShapeKind::Line,
                                _ => ShapeKind::Rect,
                            };
                            Content::Shape { kind, color: [r as f32, g as f32, b as f32], radius: item.radius() }
                        }
                    }
                }
                ItemKind::Video | ItemKind::Image => {
                    let key = if item.kind == ItemKind::Video { item.id.clone() } else { image_key(item) };
                    let Some(picture) = self.pictures.get(&key) else { continue };
                    let asset = project.asset(item.asset_id.as_deref());
                    let source_size =
                        (asset.and_then(|a| a.width).unwrap_or(picture.width as f64), asset.and_then(|a| a.height).unwrap_or(picture.height as f64));
                    Content::Texture { view: &picture.view, source_size }
                }
                ItemKind::Text => {
                    let key = text_picture_key(project, item, k);
                    let Some(picture) = self.pictures.get(&key) else { continue };
                    Content::Texture { view: &picture.view, source_size: (picture.width as f64, picture.height as f64) }
                }
                ItemKind::Audio => continue,
            };
            draws.push(DrawLayer { layer, content });
        }
        let [r, g, b] = hex_to_rgb(&project.background);
        let frame = vf_core::time::frame_index(time, project.fps);
        self.compositor.render(target, k, [r as f32, g as f32, b as f32], &draws, frame);
        self.evict();
        RenderStats { complete }
    }

    /// Size a clip needs to be decoded at to look sharp at its largest.
    fn decode_size(&self, project: &Project, item: &TimelineItem, layer: &Layer, k: f64) -> (u32, u32) {
        let asset = project.asset(item.asset_id.as_deref());
        let (aw, ah) = (asset.and_then(|a| a.width).unwrap_or(1920.0), asset.and_then(|a| a.height).unwrap_or(1080.0));
        let mut scale = item.transform.map(|t| t.scale).unwrap_or(1.0);
        for keyframe in &item.keyframes {
            scale = scale.max(keyframe.transform.scale);
        }
        let shown = (layer.box_w * k * scale.max(1.0) * 1.15).max(layer.box_h * k * scale.max(1.0) * 1.15);
        let crop = layer.source.map(|s| (aw / s.sw.max(1.0)).max(ah / s.sh.max(1.0))).unwrap_or(1.0);
        let longest = (shown * crop).min(self.max_decode as f64).max(64.0);
        let ratio = longest / aw.max(ah);
        vf_media::fit_size(aw as u32, ah as u32, ((aw * ratio).ceil() as u32).max(2), ((ah * ratio).ceil() as u32).max(2))
    }

    fn video_picture(&mut self, project: &Project, item: &TimelineItem, layer: &Layer, k: f64) -> bool {
        let Some(asset) = project.asset(item.asset_id.as_deref()) else { return true };
        let Some(media_time) = layer.media_time else { return true };
        let size = self.decode_size(project, item, layer, k);
        let path = PathBuf::from(&asset.path);
        let mode = self.mode;
        let tick = self.tick;
        let slot = self.videos.entry(item.id.clone()).or_insert_with(|| VideoSlot::new(path.clone(), size, tick));
        slot.used = tick;
        // A different file, or a much sharper picture needed: start over.
        if slot.path != path || size.0 > slot.size.0 * 3 / 2 || size.1 > slot.size.1 * 3 / 2 {
            *slot = VideoSlot::new(path, size, tick);
        }
        if slot.failed {
            return true;
        }
        let image = match mode {
            Mode::Preview => {
                let key = (slot.path.clone(), slot.size.0, slot.size.1);
                let source = slot.source.get_or_insert_with(|| FrameSource::spawn(key.clone(), self.frames.clone()));
                if source.failed() {
                    slot.failed = true;
                    return true;
                }
                source.request(media_time, self.playing);
                let Some((entry, image)) = self.frames.lock().ok().and_then(|mut cache| cache.get(&key, media_time)) else {
                    // Still decoding: keep showing the last picture.
                    if let Some(picture) = self.pictures.get_mut(&item.id) {
                        picture.used = tick;
                    }
                    return false;
                };
                if slot.shown_entry == Some(entry) && self.pictures.contains_key(&item.id) {
                    self.pictures.get_mut(&item.id).expect("picture").used = tick;
                    return true;
                }
                slot.shown_entry = Some(entry);
                image
            }
            Mode::Export => {
                if slot.decoder.is_none() {
                    match VideoDecoder::open(&slot.path, slot.size.0, slot.size.1) {
                        Ok(decoder) => slot.decoder = Some(decoder),
                        Err(error) => {
                            log::warn!("{error}");
                            slot.failed = true;
                            return true;
                        }
                    }
                }
                match slot.decoder.as_mut().expect("decoder").frame_at(media_time) {
                    Ok(Some(image)) => {
                        slot.shown = Some(media_time);
                        Arc::new(image)
                    }
                    _ => return slot.shown.is_some(),
                }
            }
        };
        match self.pictures.get_mut(&item.id) {
            Some(picture) => {
                picture.update(&self.gpu, &image, "video");
                picture.used = tick;
            }
            None => {
                self.pictures.insert(item.id.clone(), Picture::new(&self.gpu, &image, tick, "video"));
            }
        }
        true
    }

    fn image_picture(&mut self, project: &Project, item: &TimelineItem, layer: &Layer, k: f64) -> bool {
        let Some(asset) = project.asset(item.asset_id.as_deref()) else { return true };
        let key = image_key(item);
        let size = self.decode_size(project, item, layer, k);
        if let Some(picture) = self.pictures.get_mut(&key) {
            picture.used = self.tick;
        }
        // Not decoded yet, or a much sharper picture needed: decode (again).
        let sharp = self.images.get(&key).is_some_and(|slot| size.0 <= slot.size.0 * 3 / 2 && size.1 <= slot.size.1 * 3 / 2);
        if !sharp {
            let path = PathBuf::from(&asset.path);
            let pending = match self.mode {
                Mode::Export => {
                    if let Ok(image) = vf_media::video::load_image(&path, size.0, size.1) {
                        self.pictures.insert(key.clone(), Picture::new(&self.gpu, &image, self.tick, "image"));
                    }
                    None
                }
                Mode::Preview => {
                    let (tx, rx) = channel();
                    std::thread::spawn(move || {
                        let _ = tx.send(vf_media::video::load_image(&path, size.0, size.1).ok());
                    });
                    Some(rx)
                }
            };
            self.images.insert(key.clone(), ImageSlot { size, pending });
        }
        let slot = self.images.get_mut(&key).expect("image slot");
        let Some(pending) = &slot.pending else { return true };
        match pending.try_recv() {
            Ok(loaded) => {
                slot.pending = None;
                if let Some(image) = loaded {
                    self.pictures.insert(key, Picture::new(&self.gpu, &image, self.tick, "image"));
                }
                true
            }
            // Still decoding: the smaller picture, if any, stands in.
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                slot.pending = None;
                true
            }
        }
    }

    fn text_picture(&mut self, project: &Project, item: &TimelineItem, k: f64) -> bool {
        let key = text_picture_key(project, item, k);
        if let Some(picture) = self.pictures.get_mut(&key) {
            picture.used = self.tick;
            return true;
        }
        let (width, height, data) = self.text.rasterize(&item.text_style(), project.width as f64, k * text_detail(item));
        let image = RgbaImage { width, height, data };
        self.pictures.insert(key, Picture::new(&self.gpu, &image, self.tick, "text"));
        true
    }

    /// Drops pictures not used for a while, and decoders of clips gone from view.
    fn evict(&mut self) {
        let tick = self.tick;
        let keep = if self.mode == Mode::Preview { 240 } else { 30 };
        self.pictures.retain(|_, picture| tick - picture.used < keep);
        // Decoders live as long as their clip is on screen recently.
        self.videos.retain(|_, slot| tick - slot.used < keep);
        let pictures = &self.pictures;
        self.images.retain(|key, slot| slot.pending.is_some() || pictures.contains_key(key));
    }
}

fn image_key(item: &TimelineItem) -> String {
    format!("image:{}", item.asset_id.clone().unwrap_or_default())
}

fn text_picture_key(project: &Project, item: &TimelineItem, k: f64) -> String {
    format!("text:{:x}", text_key(&item.text_style(), project.width as f64, k * text_detail(item)))
}

/// How much sharper than 1:1 a text is rasterized: enough for the largest it
/// gets (its scale, keyframes and zooming animations), in a few steps so that
/// scaling it does not re-rasterize on every frame.
pub fn text_detail(item: &TimelineItem) -> f64 {
    use vf_core::model::{AnimationInKind, AnimationOutKind, EffectKind};
    let mut peak = item.transform.map(|t| t.scale).unwrap_or(1.0);
    for keyframe in &item.keyframes {
        peak = peak.max(keyframe.transform.scale);
    }
    // Effects that zoom: about 1.2× at their default strength, more when turned up.
    let zoom = item
        .active_effects()
        .filter(|e| matches!(e.kind, EffectKind::Punch | EffectKind::ZoomIn | EffectKind::ZoomOut | EffectKind::Pulse | EffectKind::Sway))
        .map(|e| 1.0 + 0.4 * e.amount())
        .fold(1.0, f64::max);
    let grows = item.anim_in.is_some_and(|a| a.kind == AnimationInKind::ZoomOut) || item.anim_out.is_some_and(|a| a.kind == AnimationOutKind::Grow);
    if grows {
        peak *= 1.8;
    } else if item.anim_in.is_some_and(|a| a.kind != AnimationInKind::None) {
        peak *= zoom.max(1.2);
    } else {
        peak *= zoom;
    }
    [1.0, 1.5, 2.0, 3.0, 4.0, 6.0].into_iter().find(|step| peak <= step * 1.02).unwrap_or(6.0)
}

#[cfg(test)]
mod image_tests {
    use super::*;
    use vf_core::model::{Fit, MediaAsset, MediaKind, Track, TrackKind, Transform};

    fn photo() -> Option<PathBuf> {
        let path = std::env::temp_dir().join("vf-render-fixtures").join("photo-1024.png");
        if !path.is_file() {
            std::fs::create_dir_all(path.parent()?).ok()?;
            let made = std::process::Command::new("ffmpeg")
                .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=s=1024x1024", "-frames:v", "1"])
                .arg(&path)
                .status()
                .is_ok_and(|s| s.success());
            if !made {
                return None;
            }
        }
        Some(path)
    }

    /// A photo first shown small is decoded again, sharper, once it is shown large.
    #[test]
    fn images_sharpen_when_shown_larger() {
        let Some(path) = photo() else { return };
        let Ok(gpu) = Gpu::new() else { return };
        let mut project = vf_core::presets::create_project("Test", 1024, 1024, 30);
        project.assets.push(MediaAsset {
            id: "a".into(),
            kind: MediaKind::Image,
            name: "photo".into(),
            path: path.display().to_string(),
            width: Some(1024.0),
            height: Some(1024.0),
            size: 1,
            ..Default::default()
        });
        let image = TimelineItem {
            id: "i".into(),
            kind: ItemKind::Image,
            start: 0.0,
            duration: 4.0,
            asset_id: Some("a".into()),
            fit: Some(Fit::Fill),
            transform: Some(Transform::default()),
            ..Default::default()
        };
        project.tracks = vec![Track { id: "t".into(), kind: TrackKind::Video, items: vec![image], ..Default::default() }];
        for mode in [Mode::Preview, Mode::Export] {
            let mut renderer = FrameRenderer::new(&gpu, Arc::new(TextEngine::new()), mode);
            let mut decoded_width = |size: u32| {
                let target = crate::compositor::render_target(&gpu, size, size);
                // The preview decodes in the background.
                for _ in 0..400 {
                    if renderer.render(&project, 1.0, &target, size as f64 / 1024.0).complete {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                renderer.pictures["image:a"].width
            };
            assert!(decoded_width(128) < 256, "{mode:?}: decoded for the small view");
            assert_eq!(decoded_width(1024), 1024, "{mode:?}: decoded again for the large view");
        }
    }
}

#[cfg(test)]
mod detail_tests {
    use super::text_detail;
    use vf_core::model::{Animation, AnimationInKind, Keyframe, TimelineItem, Transform};

    #[test]
    fn follows_the_largest_scale_in_steps() {
        let at = |scale: f64| TimelineItem { transform: Some(Transform { scale, ..Default::default() }), ..Default::default() };
        assert_eq!(text_detail(&at(1.0)), 1.0);
        assert_eq!(text_detail(&at(0.5)), 1.0);
        assert_eq!(text_detail(&at(1.3)), 1.5);
        assert_eq!(text_detail(&at(1.9)), 2.0);
        assert_eq!(text_detail(&at(4.1)), 6.0);
        assert_eq!(text_detail(&at(9.0)), 6.0);
        let keyed = TimelineItem { keyframes: vec![Keyframe { transform: Transform { scale: 2.5, ..Default::default() }, ..Default::default() }], ..at(1.0) };
        assert_eq!(text_detail(&keyed), 3.0);
        let popping = TimelineItem { anim_in: Some(Animation { kind: AnimationInKind::Pop, duration: 0.3 }), ..at(1.0) };
        assert_eq!(text_detail(&popping), 1.5);
    }
}
