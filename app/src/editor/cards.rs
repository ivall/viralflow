//! Effect and transition cards: a small landscape drawn through the real
//! scene engine and compositor, so a card shows exactly what the effect or
//! transition does to a clip. Stills are drawn a few at a time after the
//! editor opens; the moving version of a card is drawn when the pointer
//! first rests on it.

use std::collections::HashMap;

use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use vf_core::engine::scene::{resolve_scene, scene_connections, TextMeasure};
use vf_core::model::{Animation, ClipEffect, EffectKind, ItemKind, MediaAsset, MediaKind, Project, TimelineItem, Track, TrackKind, TransitionKind};
use vf_render::compositor::{read, render_target, upload, Compositor, Content, DrawLayer};
use vf_render::Gpu;

/// Card size in pixels (16:9, as the panels show them).
const WIDTH: u32 = 160;
const HEIGHT: u32 = 90;
/// The project the card scene lives in; effects measured in project pixels
/// (pixel blocks) look on a card as they do on a 640 px wide video.
const PROJECT_W: u32 = 640;
const PROJECT_H: u32 = 360;
/// Frames of a moving card, and how long they loop.
pub const FRAMES: usize = 12;
const LOOP: f64 = 1.2;
const TRANSITION: f64 = 0.8;
/// Moving cards kept; older ones are dropped.
const KEEP_MOVING: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Card {
    Effect(EffectKind),
    Transition(TransitionKind),
}

struct NoText;

impl TextMeasure for NoText {
    fn text_box(&self, _item: &TimelineItem, _canvas_w: f64) -> (f64, f64) {
        (1.0, 1.0)
    }
}

pub struct Cards {
    gpu: Gpu,
    compositor: Compositor,
    target: wgpu::Texture,
    /// The landscape, and the sunset a transition cuts to.
    day: (wgpu::Texture, wgpu::TextureView),
    dusk: (wgpu::Texture, wgpu::TextureView),
    stills: HashMap<Card, Image>,
    moving: Vec<(Card, Vec<Image>)>,
}

impl Cards {
    pub fn new(gpu: &Gpu) -> Self {
        let picture = |palette: &Palette, label: &str| {
            let texture = upload(gpu, PROJECT_W, PROJECT_H, &landscape(PROJECT_W, PROJECT_H, palette), label);
            let view = texture.create_view(&Default::default());
            (texture, view)
        };
        Cards {
            gpu: gpu.clone(),
            compositor: Compositor::new(gpu),
            target: render_target(gpu, WIDTH, HEIGHT),
            day: picture(&DAY, "card-day"),
            dusk: picture(&DUSK, "card-dusk"),
            stills: HashMap::new(),
            moving: Vec::new(),
        }
    }

    /// The card at rest: the frame of its loop that changes the picture most.
    pub fn still(&mut self, card: Card) -> Image {
        if let Some(image) = self.stills.get(&card) {
            return image.clone();
        }
        let plain = self.render_rgba(&card_project(None), 0.0);
        let (project, times) = card_scene(card);
        let best = times
            .iter()
            .map(|&time| {
                let rgba = self.render_rgba(&project, time);
                let change: u64 = rgba.iter().zip(&plain).map(|(a, b)| (*a as i32 - *b as i32).unsigned_abs() as u64).sum();
                (change, rgba)
            })
            .max_by_key(|(change, _)| *change)
            .map(|(_, rgba)| rgba)
            .unwrap_or(plain);
        let image = to_image(&best);
        self.stills.insert(card, image.clone());
        image
    }

    /// The card's loop.
    pub fn frames(&mut self, card: Card) -> Vec<Image> {
        if let Some((_, frames)) = self.moving.iter().find(|(c, _)| *c == card) {
            return frames.clone();
        }
        let (project, times) = card_scene(card);
        let frames: Vec<Image> = times.iter().map(|&time| to_image(&self.render_rgba(&project, time))).collect();
        self.moving.push((card, frames.clone()));
        if self.moving.len() > KEEP_MOVING {
            self.moving.remove(0);
        }
        frames
    }

    fn render_rgba(&mut self, project: &Project, time: f64) -> Vec<u8> {
        let connections = scene_connections(project);
        let layers = resolve_scene(project, &connections, time, &NoText);
        let draws: Vec<DrawLayer> = layers
            .iter()
            .map(|layer| {
                let item = &project.tracks[layer.track].items[layer.index];
                let view = if item.asset_id.as_deref() == Some("dusk") { &self.dusk.1 } else { &self.day.1 };
                DrawLayer { layer, content: Content::Texture { view, source_size: (PROJECT_W as f64, PROJECT_H as f64) } }
            })
            .collect();
        let k = WIDTH as f64 / PROJECT_W as f64;
        // A fixed frame number: the grain of a card does not crawl.
        self.compositor.render(&self.target, k, [0.0, 0.0, 0.0], &draws, (time * 30.0) as i64);
        read(&self.gpu, &self.target)
    }
}

fn to_image(rgba: &[u8]) -> Image {
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(WIDTH, HEIGHT);
    buffer.make_mut_bytes().copy_from_slice(&rgba[..(WIDTH * HEIGHT * 4) as usize]);
    Image::from_rgba8(buffer)
}

fn asset(id: &str) -> MediaAsset {
    MediaAsset {
        id: id.into(),
        kind: MediaKind::Image,
        name: id.into(),
        path: String::new(),
        width: Some(PROJECT_W as f64),
        height: Some(PROJECT_H as f64),
        ..Default::default()
    }
}

fn clip(id: &str, asset: &str, start: f64, duration: f64) -> TimelineItem {
    TimelineItem { id: id.into(), track_id: "t".into(), kind: ItemKind::Image, start, duration, asset_id: Some(asset.into()), ..Default::default() }
}

/// One clip of the landscape, with an effect when given.
fn card_project(effect: Option<EffectKind>) -> Project {
    let mut item = clip("a", "day", 0.0, LOOP);
    if let Some(kind) = effect {
        item.effects.push(ClipEffect { id: "fx".into(), kind, ..Default::default() });
    }
    Project {
        width: PROJECT_W,
        height: PROJECT_H,
        fps: 30,
        assets: vec![asset("day"), asset("dusk")],
        tracks: vec![Track { id: "t".into(), kind: TrackKind::Video, items: vec![item], ..Default::default() }],
        ..Default::default()
    }
}

/// The scene a card shows and the moments of its loop.
fn card_scene(card: Card) -> (Project, Vec<f64>) {
    match card {
        Card::Effect(kind) => (card_project(Some(kind)), (0..FRAMES).map(|i| i as f64 / FRAMES as f64 * LOOP).collect()),
        Card::Transition(kind) => {
            let mut project = card_project(None);
            let mut incoming = clip("b", "dusk", 1.0, 1.0);
            incoming.transition = Some(Animation { kind, duration: TRANSITION });
            project.tracks[0].items[0].duration = 1.0;
            project.tracks[0].items.push(incoming);
            // Across the transition, with a moment of each clip on its own either side.
            let (from, to) = (1.0 - TRANSITION / 2.0 - 0.1, 1.0 + TRANSITION / 2.0 + 0.1);
            (project, (0..FRAMES).map(|i| from + (to - from) * i as f64 / (FRAMES - 1) as f64).collect())
        }
    }
}

struct Palette {
    sky_top: [f32; 3],
    sky_bottom: [f32; 3],
    sun: [f32; 3],
    hill: [f32; 3],
    near: [f32; 3],
}

const DAY: Palette =
    Palette { sky_top: [0.31, 0.49, 1.0], sky_bottom: [0.55, 0.42, 0.96], sun: [1.0, 0.82, 0.4], hill: [0.17, 0.18, 0.42], near: [0.1, 0.11, 0.27] };

const DUSK: Palette =
    Palette { sky_top: [1.0, 0.45, 0.36], sky_bottom: [1.0, 0.72, 0.42], sun: [1.0, 0.97, 0.88], hill: [0.38, 0.16, 0.36], near: [0.2, 0.08, 0.22] };

/// A sky, a sun and two hills: enough shapes and colour for any effect to show.
fn landscape(width: u32, height: u32, p: &Palette) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    let mix = |a: [f32; 3], b: [f32; 3], t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
    for y in 0..height {
        for x in 0..width {
            let (u, v) = (x as f32 / width as f32, y as f32 / height as f32);
            let mut color = mix(p.sky_top, p.sky_bottom, v);
            let (dx, dy) = ((u - 0.68) * width as f32 / height as f32, v - 0.32);
            let sun = (dx * dx + dy * dy).sqrt();
            if sun < 0.14 {
                color = mix(p.sun, color, ((sun - 0.13) / 0.01).clamp(0.0, 1.0));
            }
            let far = 0.62 + 0.08 * (u * 7.0).sin() * (u * 2.3 + 0.4).cos();
            if v > far {
                color = p.hill;
            }
            let near = 0.78 + 0.06 * (u * 4.0 + 1.3).cos();
            if v > near {
                color = mix(p.near, p.hill, ((near + 0.2 - v) / 0.2).clamp(0.0, 0.4));
            }
            rgba.extend(color.iter().map(|c| (c.clamp(0.0, 1.0) * 255.0) as u8));
            rgba.push(255);
        }
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_cards_span_both_clips() {
        let (project, times) = card_scene(Card::Transition(TransitionKind::Fade));
        assert_eq!(times.len(), FRAMES);
        assert!(times[0] < 1.0 - TRANSITION / 2.0 && *times.last().unwrap() > 1.0 + TRANSITION / 2.0);
        let connections = scene_connections(&project);
        assert_eq!(connections[0].len(), 1, "the clips touch and carry the transition");
    }

    #[test]
    fn cards_show_what_the_effect_does() {
        let Ok(gpu) = Gpu::new() else { return };
        let mut cards = Cards::new(&gpu);
        let plain = cards.render_rgba(&card_project(None), 0.0);
        let mono = cards.render_rgba(&card_project(Some(EffectKind::Mono)), 0.0);
        assert_ne!(plain, mono);
        // Black & white: every pixel grey.
        assert!(mono.chunks_exact(4).all(|p| (p[0] as i32 - p[2] as i32).abs() <= 6), "grey");
        assert_eq!(cards.frames(Card::Effect(EffectKind::Glitch)).len(), FRAMES);
        let _ = cards.still(Card::Transition(TransitionKind::Iris));
    }
}
