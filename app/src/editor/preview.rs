//! The preview canvas: fits the project into the viewport and renders frames
//! into textures Slint shows directly.

use std::sync::Arc;

use vf_core::engine::scene::{build_layer, Layer, LayerOptions};
use vf_core::model::Project;
use vf_render::compositor::{read, render_target};
use vf_render::{FrameRenderer, Gpu, Mode, TextEngine};

pub struct Preview {
    gpu: Gpu,
    pub renderer: FrameRenderer,
    targets: Vec<wgpu::Texture>,
    current: usize,
    pixels: (u32, u32),
    /// Canvas size in logical pixels.
    pub canvas: (f32, f32),
    viewport: (f32, f32),
    scale_factor: f32,
    pub fullscreen: bool,
}

impl Preview {
    pub fn new(gpu: &Gpu, text: Arc<TextEngine>) -> Self {
        Preview {
            gpu: gpu.clone(),
            renderer: FrameRenderer::new(gpu, text, Mode::Preview),
            targets: vec![],
            current: 0,
            pixels: (0, 0),
            canvas: (0.0, 0.0),
            viewport: (0.0, 0.0),
            scale_factor: 1.0,
            fullscreen: false,
        }
    }

    pub fn set_viewport(&mut self, width: f32, height: f32, scale_factor: f32) {
        self.viewport = (width, height);
        self.scale_factor = scale_factor.max(0.5);
    }

    /// Fits the canvas; returns true when its size changed.
    pub fn layout(&mut self, project: &Project) -> bool {
        let padding = if self.fullscreen { 0.0 } else { 24.0 };
        let available_w = (self.viewport.0 - padding * 2.0).max(40.0);
        let available_h = (self.viewport.1 - padding * 2.0).max(40.0);
        let ratio = project.width as f32 / project.height.max(1) as f32;
        let mut width = available_w;
        let mut height = width / ratio;
        if height > available_h {
            height = available_h;
            width = height * ratio;
        }
        let canvas = (width.floor(), height.floor());
        // As many pixels as the screen shows (text and shapes stay sharp when the
        // canvas is shown larger than the project), up to twice the project size.
        let pixel_w = ((canvas.0 * self.scale_factor).round() as u32).clamp(1, (project.width.max(1) * 2).min(4096));
        let pixel_h = ((pixel_w as f32 * project.height as f32 / project.width.max(1) as f32).round() as u32).max(1);
        let changed = canvas != self.canvas || (pixel_w, pixel_h) != self.pixels;
        if (pixel_w, pixel_h) != self.pixels || self.targets.is_empty() {
            self.pixels = (pixel_w, pixel_h);
            self.targets = (0..2).map(|_| render_target(&self.gpu, pixel_w, pixel_h)).collect();
        }
        self.canvas = canvas;
        changed
    }

    /// Logical canvas pixels per project pixel.
    pub fn view_scale(&self, project: &Project) -> f64 {
        self.canvas.0 as f64 / project.width.max(1) as f64
    }

    /// Renders a frame; returns the image to show and whether it was complete.
    pub fn render(&mut self, project: &Project, time: f64) -> Option<(slint::Image, bool)> {
        if self.targets.is_empty() || self.canvas.0 < 1.0 {
            return None;
        }
        self.current = (self.current + 1) % self.targets.len();
        let target = &self.targets[self.current];
        let k = self.pixels.0 as f64 / project.width.max(1) as f64;
        let stats = self.renderer.render(project, time, target, k);
        let image = match slint::Image::try_from(target.clone()) {
            Ok(image) => image,
            Err(error) => {
                log::error!("preview texture: {error:?}");
                return None;
            }
        };
        Some((image, stats.complete))
    }

    /// Layers of the given clips at `time`, without animation (selection handles).
    pub fn static_layers(&self, project: &Project, time: f64, ids: &[String]) -> Vec<(String, Layer)> {
        let measure = self.renderer.measure();
        let mut result = Vec::new();
        for (t, track) in project.tracks.iter().enumerate() {
            if track.hidden || track.kind == vf_core::model::TrackKind::Audio {
                continue;
            }
            for (i, item) in track.items.iter().enumerate() {
                if item.hidden || !ids.contains(&item.id) || time < item.start || time >= item.end() {
                    continue;
                }
                if let Some(layer) = build_layer(project, t, i, time, &measure, LayerOptions { connection: None, static_only: true }) {
                    result.push((item.id.clone(), layer));
                }
            }
        }
        result
    }

    /// Animated layers at `time` with their item ids (hit testing).
    pub fn layers(&self, project: &Project, time: f64) -> Vec<(String, Layer)> {
        self.renderer.layers(project, time).into_iter().map(|layer| (project.tracks[layer.track].items[layer.index].id.clone(), layer)).collect()
    }

    /// A 360 px wide JPEG of the frame at `time` (project thumbnails).
    pub fn thumbnail(&mut self, project: &Project, time: f64) -> Option<Vec<u8>> {
        let width = 360u32;
        let height = ((width as f64 * project.height as f64 / project.width.max(1) as f64).round() as u32).max(1);
        let target = render_target(&self.gpu, width, height);
        let k = width as f64 / project.width.max(1) as f64;
        // Pictures decode in the background; render until complete (bounded).
        for _ in 0..40 {
            if self.renderer.render(project, time, &target, k).complete {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let rgba = read(&self.gpu, &target);
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        for pixel in rgba.chunks_exact(4) {
            rgb.extend_from_slice(&pixel[..3]);
        }
        let mut jpeg = Vec::new();
        jpeg_encoder::Encoder::new(&mut jpeg, 82).encode(&rgb, width as u16, height as u16, jpeg_encoder::ColorType::Rgb).ok()?;
        Some(jpeg)
    }
}
