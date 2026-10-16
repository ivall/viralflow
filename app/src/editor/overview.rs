//! Pictures inside timeline clips: video filmstrips, image tiles and audio
//! waveforms. Source data is computed once per file on the shared media pool
//! (timeline pictures before library thumbnails); the picture of a clip is
//! composed for its visible part at the current zoom.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use vf_core::model::{ItemKind, MediaAsset, MediaKind, TimelineItem};
use vf_media::RgbaImage;

/// Filmstrip frames are this tall; one frame per second, at most this many.
const FRAME_HEIGHT: u32 = 64;
const MAX_FRAMES: usize = 160;
const PEAKS_PER_SECOND: u32 = 100;
/// Library thumbnails fit this box (2× the tile size for sharp HiDPI).
const THUMB_WIDTH: u32 = 320;
const THUMB_HEIGHT: u32 = 180;

enum Source {
    Loading,
    Frames { frames: Vec<RgbaImage>, step: f64 },
    Picture(RgbaImage),
    Peaks(Vec<f32>),
    Failed,
}

struct Result {
    path: String,
    source: Source,
}

pub struct Overviews {
    sources: HashMap<String, Source>,
    sender: Sender<Result>,
    receiver: Receiver<Result>,
    composed: HashMap<String, (u64, Image)>,
    thumbs: HashMap<String, Image>,
    /// Bumped whenever background work finished (pictures may change).
    generation: u64,
}

impl Default for Overviews {
    fn default() -> Self {
        let (sender, receiver) = channel();
        Overviews { sources: HashMap::new(), sender, receiver, composed: HashMap::new(), thumbs: HashMap::new(), generation: 0 }
    }
}

impl Overviews {
    /// Takes finished background work; true when something new arrived.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(result) = self.receiver.try_recv() {
            self.sources.insert(result.path, result.source);
            changed = true;
        }
        if changed {
            self.composed.clear();
            self.generation += 1;
        }
        changed
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Library thumbnail of a picture or video (a frame a third of the way in).
    pub fn thumbnail(&mut self, asset: &MediaAsset) -> Option<Image> {
        if let Some(image) = self.thumbs.get(&asset.id) {
            return Some(image.clone());
        }
        let key = format!("Thumb:{}", asset.path);
        match self.sources.get(&key) {
            Some(Source::Picture(picture)) => {
                let image = to_image(picture);
                self.thumbs.insert(asset.id.clone(), image.clone());
                Some(image)
            }
            Some(_) => None,
            None => {
                self.sources.insert(key.clone(), Source::Loading);
                let sender = self.sender.clone();
                let path = PathBuf::from(&asset.path);
                let (kind, duration) = (asset.kind, asset.duration.unwrap_or(0.0));
                vf_media::jobs::spawn(vf_media::jobs::Priority::Library, move || {
                    let picture = match kind {
                        MediaKind::Image => vf_media::video::load_image(&path, THUMB_WIDTH, THUMB_HEIGHT),
                        _ => vf_media::overview::filmstrip(&path, duration, 3, THUMB_HEIGHT).and_then(|frames| {
                            let middle = 1.min(frames.len().saturating_sub(1));
                            frames.into_iter().nth(middle).ok_or_else(|| vf_core::Error::msg("The video has no frames."))
                        }),
                    };
                    let source = picture.map(Source::Picture).unwrap_or(Source::Failed);
                    let _ = sender.send(Result { path: key, source });
                });
                None
            }
        }
    }

    fn request(&mut self, asset: &MediaAsset, kind: ItemKind) {
        let key = source_key(asset, kind);
        if self.sources.contains_key(&key) {
            return;
        }
        self.sources.insert(key.clone(), Source::Loading);
        let sender = self.sender.clone();
        let path = PathBuf::from(&asset.path);
        let duration = asset.duration.unwrap_or(0.0);
        vf_media::jobs::spawn(vf_media::jobs::Priority::Timeline, move || {
            let source = match kind {
                ItemKind::Audio => vf_media::overview::waveform(&path, PEAKS_PER_SECOND, &|| false).map(Source::Peaks),
                ItemKind::Image => vf_media::video::load_image(&path, FRAME_HEIGHT * 4, FRAME_HEIGHT).map(Source::Picture),
                _ => {
                    let count = (duration.ceil() as usize).clamp(1, MAX_FRAMES);
                    vf_media::overview::filmstrip(&path, duration, count, FRAME_HEIGHT).map(|frames| Source::Frames { step: duration / count as f64, frames })
                }
            };
            let _ = sender.send(Result { path: key, source: source.unwrap_or(Source::Failed) });
        });
    }

    /// Picture for the visible part `[from, to)` (pixels from the clip's left
    /// edge) of a clip `height` pixels tall, or None while it is loading.
    pub fn picture(&mut self, item: &TimelineItem, asset: Option<&MediaAsset>, zoom: f64, from: f64, to: f64, height: f64) -> Option<Image> {
        let asset = asset?;
        let kind = match (item.kind, asset.kind) {
            (ItemKind::Audio, _) => ItemKind::Audio,
            (ItemKind::Image, _) | (_, MediaKind::Image) => ItemKind::Image,
            (ItemKind::Video, _) => ItemKind::Video,
            _ => return None,
        };
        self.request(asset, kind);
        let width = (to - from).round().clamp(1.0, 4096.0) as u32;
        let height = height.round().clamp(1.0, 256.0) as u32;
        let stamp = stamp(&[
            item.source_in().to_bits(),
            item.speed().to_bits(),
            zoom.to_bits(),
            from.round().to_bits(),
            width as u64,
            height as u64,
            item.volume().to_bits(),
            item.muted as u64,
        ]);
        if let Some((key, image)) = self.composed.get(&item.id) {
            if *key == stamp {
                return Some(image.clone());
            }
        }
        let source = self.sources.get(&source_key(asset, kind))?;
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
        let pixels = buffer.make_mut_bytes();
        let speed = item.speed();
        let time_at = |x: f64| item.source_in() + (from + x) / zoom.max(1e-6) * speed;
        match source {
            Source::Loading => return None,
            Source::Failed => return None,
            Source::Frames { frames, step } => {
                let first = frames.first()?;
                let tile = (first.width as f64 * height as f64 / first.height.max(1) as f64).max(8.0);
                // Tiles stay put on the timeline while scrolling.
                let offset = from.rem_euclid(tile);
                let mut x0 = -offset;
                while x0 < width as f64 {
                    let index = ((time_at(x0 + tile / 2.0) / step.max(1e-6)).floor() as usize).min(frames.len() - 1);
                    blit_scaled(pixels, width, height, &frames[index], x0.round() as i32, tile.round() as u32);
                    x0 += tile;
                }
            }
            Source::Picture(image) => {
                let tile = (image.width as f64 * height as f64 / image.height.max(1) as f64).max(8.0);
                let mut x0 = -from.rem_euclid(tile);
                while x0 < width as f64 {
                    blit_scaled(pixels, width, height, image, x0.round() as i32, tile.round() as u32);
                    x0 += tile;
                }
                for pixel in pixels.chunks_exact_mut(4) {
                    pixel[3] = (pixel[3] as u32 * 230 / 255) as u8;
                }
            }
            Source::Peaks(peaks) => {
                let gain = if item.muted { 0.0 } else { item.volume() as f32 };
                let mid = height as f32 / 2.0;
                for x in 0..width {
                    let t0 = time_at(x as f64);
                    let t1 = time_at(x as f64 + 1.0);
                    let i0 = (t0 * PEAKS_PER_SECOND as f64).floor().max(0.0) as usize;
                    let i1 = ((t1 * PEAKS_PER_SECOND as f64).ceil() as usize).max(i0 + 1).min(peaks.len());
                    let peak = peaks.get(i0..i1).map(|s| s.iter().fold(0.0f32, |a, b| a.max(*b))).unwrap_or(0.0);
                    let half = ((peak * gain).min(1.0) * (mid - 2.0)).max(0.5);
                    let top = (mid - half).floor().max(0.0) as u32;
                    let bottom = ((mid + half).ceil() as u32).min(height);
                    for y in top..bottom {
                        let i = ((y * width + x) * 4) as usize;
                        pixels[i..i + 4].copy_from_slice(&[120, 230, 190, 230]);
                    }
                }
            }
        }
        let image = Image::from_rgba8(buffer);
        self.composed.insert(item.id.clone(), (stamp, image.clone()));
        Some(image)
    }
}

fn to_image(picture: &RgbaImage) -> Image {
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(picture.width.max(1), picture.height.max(1));
    let bytes = buffer.make_mut_bytes();
    if bytes.len() == picture.data.len() {
        bytes.copy_from_slice(&picture.data);
    }
    Image::from_rgba8(buffer)
}

fn source_key(asset: &MediaAsset, kind: ItemKind) -> String {
    format!("{:?}:{}", kind, asset.path)
}

fn stamp(values: &[u64]) -> u64 {
    values.iter().fold(0xcbf29ce484222325u64, |hash, value| (hash ^ value).wrapping_mul(0x100000001b3))
}

/// Draws `image` scaled to `tile`×`height` at `x` (nearest neighbour).
fn blit_scaled(out: &mut [u8], width: u32, height: u32, image: &RgbaImage, x: i32, tile: u32) {
    if image.width == 0 || image.height == 0 || tile == 0 {
        return;
    }
    for ty in 0..height {
        let sy = (ty as u64 * image.height as u64 / height as u64) as u32;
        for tx in 0..tile {
            let px = x + tx as i32;
            if px < 0 || px >= width as i32 {
                continue;
            }
            let sx = (tx as u64 * image.width as u64 / tile as u64) as u32;
            let src = ((sy * image.width + sx) * 4) as usize;
            let dst = ((ty * width + px as u32) * 4) as usize;
            out[dst..dst + 4].copy_from_slice(&image.data[src..src + 4]);
        }
    }
}
