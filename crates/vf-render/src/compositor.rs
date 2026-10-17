//! Draws the layers of a scene into a texture on the GPU. The preview and the
//! export both go through here, so they cannot differ.

use std::num::NonZeroU64;

use vf_core::engine::scene::Layer;
use vf_core::model::BlendMode;

use crate::Gpu;

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Pass uniforms sit 256 bytes apart (the dynamic offset alignment); a layer's
/// uniform is larger and takes two slots.
const UNIFORM_STRIDE: u64 = 256;
const LAYER_STRIDE: u64 = 512;
const LAYER_UNIFORM_SIZE: u64 = 288;
const PASS_UNIFORM_SIZE: u64 = 32;
/// Most blur taps per side in one pass; larger blurs run on a smaller image.
const MAX_TAPS: f32 = 24.0;

/// What a layer shows.
pub enum Content<'a> {
    /// A picture (video frame, image or text raster), premultiplied RGBA.
    /// `source_size` is the size of the full source the layer's crop refers to.
    Texture {
        view: &'a wgpu::TextureView,
        source_size: (f64, f64),
    },
    Shape {
        kind: ShapeKind,
        color: [f32; 3],
        radius: f64,
    },
    /// Blurs what is already drawn below, inside the layer box.
    BlurRegion {
        strength: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeKind {
    Rect,
    Circle,
    Line,
}

pub struct DrawLayer<'a> {
    pub layer: &'a Layer,
    pub content: Content<'a>,
}

struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Target {
    fn new(device: &wgpu::Device, width: u32, height: u32, label: &str) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Target { texture, view }
    }
}

struct Scratch {
    width: u32,
    height: u32,
    acc: [Target; 2],
    layer: Target,
    blur: Vec<(u32, Target, Target)>,
}

pub struct Compositor {
    gpu: Gpu,
    layer_pipeline: wgpu::RenderPipeline,
    blur_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    texture_layout: wgpu::BindGroupLayout,
    layer_uniforms: wgpu::Buffer,
    pass_uniforms: wgpu::Buffer,
    layer_group: wgpu::BindGroup,
    pass_group: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    empty: wgpu::TextureView,
    capacity: u64,
    scratch: Option<Scratch>,
}

fn uniform_layout(device: &wgpu::Device, binding: u32, size: u64) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: NonZeroU64::new(size) },
            count: None,
        }],
    })
}

impl Compositor {
    pub fn new(gpu: &Gpu) -> Self {
        let device = &gpu.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("compositor"),
            source: wgpu::ShaderSource::Wgsl(include_str!("compositor.wgsl").into()),
        });
        let layer_layout = uniform_layout(device, 0, LAYER_UNIFORM_SIZE);
        let pass_layout = uniform_layout(device, 1, PASS_UNIFORM_SIZE);
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                texture_entry(0),
                texture_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline = |layout: &wgpu::BindGroupLayout, vertex: &str, fragment: &str, blend: Option<wgpu::BlendState>| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(layout), Some(&texture_layout)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fragment),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some(vertex), buffers: &[], compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    targets: &[Some(wgpu::ColorTargetState { format: FORMAT, blend, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let layer_pipeline = pipeline(&layer_layout, "vs_layer", "fs_layer", Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING));
        let blur_pipeline = pipeline(&pass_layout, "vs_full", "fs_blur", None);
        let composite_pipeline = pipeline(&pass_layout, "vs_full", "fs_composite", None);
        let capacity = 64;
        let (layer_uniforms, layer_group) = Self::uniform_buffer(device, &layer_layout, 0, LAYER_UNIFORM_SIZE, capacity * LAYER_STRIDE / UNIFORM_STRIDE);
        let (pass_uniforms, pass_group) = Self::uniform_buffer(device, &pass_layout, 1, PASS_UNIFORM_SIZE, capacity * 4);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let empty = Target::new(device, 1, 1, "empty").view;
        Compositor {
            gpu: gpu.clone(),
            layer_pipeline,
            blur_pipeline,
            composite_pipeline,
            texture_layout,
            layer_uniforms,
            pass_uniforms,
            layer_group,
            pass_group,
            sampler,
            empty,
            capacity,
            scratch: None,
        }
    }

    fn uniform_buffer(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, binding: u32, size: u64, capacity: u64) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: UNIFORM_STRIDE * capacity,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &buffer, offset: 0, size: NonZeroU64::new(size) }),
            }],
        });
        (buffer, group)
    }

    fn ensure_capacity(&mut self, layers: usize) {
        let needed = (layers as u64 + 1).max(1);
        if needed <= self.capacity {
            return;
        }
        let capacity = needed.next_power_of_two();
        let device = &self.gpu.device;
        let layer_layout = uniform_layout(device, 0, LAYER_UNIFORM_SIZE);
        let pass_layout = uniform_layout(device, 1, PASS_UNIFORM_SIZE);
        (self.layer_uniforms, self.layer_group) = Self::uniform_buffer(device, &layer_layout, 0, LAYER_UNIFORM_SIZE, capacity * LAYER_STRIDE / UNIFORM_STRIDE);
        (self.pass_uniforms, self.pass_group) = Self::uniform_buffer(device, &pass_layout, 1, PASS_UNIFORM_SIZE, capacity * 4);
        self.capacity = capacity;
    }

    fn scratch(&mut self, width: u32, height: u32) -> &mut Scratch {
        if self.scratch.as_ref().is_none_or(|s| s.width != width || s.height != height) {
            let device = &self.gpu.device;
            self.scratch = Some(Scratch {
                width,
                height,
                acc: [Target::new(device, width, height, "acc-a"), Target::new(device, width, height, "acc-b")],
                layer: Target::new(device, width, height, "layer"),
                blur: vec![],
            });
        }
        self.scratch.as_mut().expect("scratch")
    }

    fn textures(&self, source: &wgpu::TextureView, aux: &wgpu::TextureView) -> wgpu::BindGroup {
        self.gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(source) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(aux) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        })
    }

    /// Renders `layers` (bottom first) over `background` into `target`
    /// (FORMAT, RENDER_ATTACHMENT | COPY_DST). `k` converts project pixels to
    /// target pixels; `frame` seeds film grain.
    pub fn render(&mut self, target: &wgpu::Texture, k: f64, background: [f32; 3], layers: &[DrawLayer], frame: i64) {
        let (width, height) = (target.width(), target.height());
        self.ensure_capacity(layers.len());
        self.scratch(width, height);
        let device = self.gpu.device.clone();
        let queue = self.gpu.queue.clone();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        let mut current = 0usize;
        {
            let scratch = self.scratch.as_ref().expect("scratch");
            let clear = wgpu::Color { r: background[0] as f64, g: background[1] as f64, b: background[2] as f64, a: 1.0 };
            begin(&mut encoder, &scratch.acc[0].view, wgpu::LoadOp::Clear(clear));
        }
        let mut pass_slot: u64 = 0;
        for (index, draw) in layers.iter().enumerate() {
            let offset = index as u64 * LAYER_STRIDE;
            queue.write_buffer(&self.layer_uniforms, offset, &layer_uniform(draw, k, width, height, frame));
            let layer = draw.layer;
            match &draw.content {
                Content::BlurRegion { strength } => {
                    let sigma = (strength * k) as f32;
                    let acc_view = self.scratch.as_ref().unwrap().acc[current].view.clone();
                    let blurred = self.blur(&mut encoder, &acc_view, width, height, sigma, &mut pass_slot);
                    let group = self.textures(&self.empty, &blurred);
                    let scratch = self.scratch.as_ref().unwrap();
                    let mut pass = begin(&mut encoder, &scratch.acc[current].view, wgpu::LoadOp::Load);
                    pass.set_pipeline(&self.layer_pipeline);
                    pass.set_bind_group(0, &self.layer_group, &[offset as u32]);
                    pass.set_bind_group(1, &group, &[]);
                    pass.draw(0..6, 0..1);
                }
                content => {
                    let source = match content {
                        Content::Texture { view, .. } => (*view).clone(),
                        _ => self.empty.clone(),
                    };
                    let group = self.textures(&source, &self.empty);
                    let simple = layer.blend == BlendMode::Normal && layer.blur <= 0.3;
                    if simple {
                        let scratch = self.scratch.as_ref().unwrap();
                        let mut pass = begin(&mut encoder, &scratch.acc[current].view, wgpu::LoadOp::Load);
                        pass.set_pipeline(&self.layer_pipeline);
                        pass.set_bind_group(0, &self.layer_group, &[offset as u32]);
                        pass.set_bind_group(1, &group, &[]);
                        pass.draw(0..6, 0..1);
                        continue;
                    }
                    {
                        let scratch = self.scratch.as_ref().unwrap();
                        let mut pass = begin(&mut encoder, &scratch.layer.view, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT));
                        pass.set_pipeline(&self.layer_pipeline);
                        pass.set_bind_group(0, &self.layer_group, &[offset as u32]);
                        pass.set_bind_group(1, &group, &[]);
                        pass.draw(0..6, 0..1);
                    }
                    let layer_view = self.scratch.as_ref().unwrap().layer.view.clone();
                    let drawn = if layer.blur > 0.3 {
                        self.blur(&mut encoder, &layer_view, width, height, (layer.blur * k) as f32, &mut pass_slot)
                    } else {
                        layer_view
                    };
                    let mode = blend_index(layer.blend);
                    let slot = self.next_pass(&mut pass_slot, [0.0, 0.0, 0.0, 0.0], [mode as f32, 0.0, 0.0, 0.0]);
                    let scratch = self.scratch.as_ref().unwrap();
                    let group = self.textures(&drawn, &scratch.acc[current].view);
                    let mut pass = begin(&mut encoder, &scratch.acc[1 - current].view, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT));
                    pass.set_pipeline(&self.composite_pipeline);
                    pass.set_bind_group(0, &self.pass_group, &[slot]);
                    pass.set_bind_group(1, &group, &[]);
                    pass.draw(0..3, 0..1);
                    drop(pass);
                    current = 1 - current;
                }
            }
        }
        let scratch = self.scratch.as_ref().unwrap();
        encoder.copy_texture_to_texture(
            scratch.acc[current].texture.as_image_copy(),
            target.as_image_copy(),
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        queue.submit([encoder.finish()]);
    }

    fn next_pass(&self, slot: &mut u64, a: [f32; 4], b: [f32; 4]) -> u32 {
        let offset = (*slot % (self.capacity * 4)) * UNIFORM_STRIDE;
        *slot += 1;
        let mut bytes = Vec::with_capacity(32);
        for value in a.iter().chain(b.iter()) {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
        self.gpu.queue.write_buffer(&self.pass_uniforms, offset, &bytes);
        offset as u32
    }

    /// Gaussian blur of `source` (full-target sized); returns a view of the result.
    fn blur(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        width: u32,
        height: u32,
        sigma: f32,
        slot: &mut u64,
    ) -> wgpu::TextureView {
        let sigma = sigma.max(0.5);
        let factor = ((sigma * 3.0) / MAX_TAPS).ceil().max(1.0) as u32;
        let (bw, bh) = ((width / factor).max(1), (height / factor).max(1));
        let device = self.gpu.device.clone();
        let scratch = self.scratch.as_mut().expect("scratch");
        if !scratch.blur.iter().any(|(f, ..)| *f == factor) {
            scratch.blur.push((factor, Target::new(&device, bw, bh, "blur-h"), Target::new(&device, bw, bh, "blur-v")));
        }
        let (h_view, v_view) = {
            let (_, h, v) = scratch.blur.iter().find(|(f, ..)| *f == factor).expect("blur targets");
            (h.view.clone(), v.view.clone())
        };
        let steps = sigma / factor as f32;
        let taps = (steps * 3.0).ceil().min(MAX_TAPS);
        let horizontal = self.next_pass(slot, [factor as f32 / width as f32, 0.0, steps, taps], [0.0; 4]);
        let vertical = self.next_pass(slot, [0.0, 1.0 / bh as f32, steps, taps], [0.0; 4]);
        for (input, output, uniform) in [(source.clone(), &h_view, horizontal), (h_view.clone(), &v_view, vertical)] {
            let group = self.textures(&input, &self.empty);
            let mut pass = begin(encoder, output, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT));
            pass.set_pipeline(&self.blur_pipeline);
            pass.set_bind_group(0, &self.pass_group, &[uniform]);
            pass.set_bind_group(1, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        v_view
    }
}

/// Starts a render pass that writes one colour target.
fn begin<'a>(encoder: &'a mut wgpu::CommandEncoder, view: &wgpu::TextureView, load: wgpu::LoadOp<wgpu::Color>) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

fn blend_index(mode: BlendMode) -> u32 {
    match mode {
        BlendMode::Normal => 0,
        BlendMode::Multiply => 1,
        BlendMode::Screen => 2,
        BlendMode::Overlay => 3,
        BlendMode::Darken => 4,
        BlendMode::Lighten => 5,
        BlendMode::ColorDodge => 6,
        BlendMode::ColorBurn => 7,
        BlendMode::HardLight => 8,
        BlendMode::SoftLight => 9,
        BlendMode::Difference => 10,
        BlendMode::Exclusion => 11,
    }
}

/// The `Layer` uniform (see compositor.wgsl) for one draw.
fn layer_uniform(draw: &DrawLayer, k: f64, width: u32, height: u32, frame: i64) -> Vec<u8> {
    let layer = draw.layer;
    let (bw, bh) = (layer.box_w * k, layer.box_h * k);
    let (sin, cos) = layer.rotation.to_radians().sin_cos();
    let sx = layer.scale * if layer.flip_x { -1.0 } else { 1.0 };
    let sy = layer.scale * if layer.flip_y { -1.0 } else { 1.0 };
    // m = R · S, t = centre − m · (box / 2).
    let (m00, m01, m10, m11) = (cos * sx, -sin * sy, sin * sx, cos * sy);
    let (hx, hy) = (bw / 2.0, bh / 2.0);
    let mut tx = layer.cx * k - (m00 * hx + m01 * hy);
    let mut ty = layer.cy * k - (m10 * hx + m11 * hy);
    // Upright pictures start on a whole pixel: sampling a texture half a pixel
    // off blurs small text (motion moves in 1 px steps, which is invisible).
    if matches!(draw.content, Content::Texture { .. }) && layer.rotation.rem_euclid(360.0).abs() < 1e-6 {
        tx = tx.round();
        ty = ty.round();
    }
    let (src_rect, kind, color, radius) = match &draw.content {
        Content::Texture { source_size, .. } => {
            let rect = match &layer.source {
                Some(s) if source_size.0 > 0.0 && source_size.1 > 0.0 => {
                    [s.sx / source_size.0, s.sy / source_size.1, s.sw / source_size.0, s.sh / source_size.1]
                }
                _ => [0.0, 0.0, 1.0, 1.0],
            };
            (rect, 0.0, [1.0, 1.0, 1.0, 1.0], layer.radius * k)
        }
        Content::Shape { kind, color, radius } => {
            let code = match kind {
                ShapeKind::Rect => 1.0,
                ShapeKind::Circle => 2.0,
                ShapeKind::Line => 3.0,
            };
            ([0.0, 0.0, 1.0, 1.0], code, [color[0] as f64, color[1] as f64, color[2] as f64, 1.0], radius * k)
        }
        Content::BlurRegion { .. } => ([0.0, 0.0, 1.0, 1.0], 4.0, [1.0; 4], layer.radius * k),
    };
    let mut glitch = [0.0f64; 6];
    for (slot, value) in glitch.iter_mut().zip(&layer.glitch) {
        *slot = value / 100.0;
    }
    let matrix = layer.matrix.unwrap_or(vf_core::engine::color::IDENTITY);
    let fx = &layer.fx;
    let values: [f64; 72] = [
        m00,
        m01,
        m10,
        m11,
        tx,
        ty,
        width as f64,
        height as f64,
        src_rect[0],
        src_rect[1],
        src_rect[2],
        src_rect[3],
        bw,
        bh,
        radius,
        kind,
        color[0],
        color[1],
        color[2],
        color[3],
        layer.opacity,
        layer.vignette,
        layer.grain,
        if layer.scanlines { 1.0 } else { 0.0 },
        layer.clip_left,
        layer.clip_right,
        (frame.rem_euclid(9973)) as f64,
        if layer.glitch.is_empty() { 0.0 } else { 1.0 },
        glitch[0],
        glitch[1],
        glitch[2],
        glitch[3],
        glitch[4],
        glitch[5],
        if layer.matrix.is_some() { 1.0 } else { 0.0 },
        0.0,
        // Matrix rows (R, G, B, A coefficients) then the offsets column.
        matrix[0],
        matrix[1],
        matrix[2],
        matrix[3],
        matrix[5],
        matrix[6],
        matrix[7],
        matrix[8],
        matrix[10],
        matrix[11],
        matrix[12],
        matrix[13],
        matrix[15],
        matrix[16],
        matrix[17],
        matrix[18],
        matrix[4],
        matrix[9],
        matrix[14],
        matrix[19],
        // Treatments (see `LayerFx`); pixel blocks in target pixels of the box.
        fx.chroma,
        fx.pixelate * k,
        fx.posterize,
        fx.sharpen,
        fx.bulge,
        if fx.mirror { 1.0 } else { 0.0 },
        fx.zoom_blur,
        fx.leak,
        fx.leak_phase,
        fx.clip_top,
        fx.clip_bottom,
        fx.iris,
        fx.clock,
        0.0,
        0.0,
        0.0,
    ];
    values.iter().flat_map(|v| (*v as f32).to_ne_bytes()).collect()
}

/// Creates a sampled texture from premultiplied RGBA pixels.
pub fn upload(gpu: &Gpu, width: u32, height: u32, rgba: &[u8], label: &str) -> wgpu::Texture {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    write(gpu, &texture, width, height, rgba);
    texture
}

/// Replaces the pixels of a texture created by `upload` (same size).
pub fn write(gpu: &Gpu, texture: &wgpu::Texture, width: u32, height: u32, rgba: &[u8]) {
    gpu.queue.write_texture(
        texture.as_image_copy(),
        rgba,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: Some(height) },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
}

/// A texture the compositor can render into and Slint can display.
pub fn render_target(gpu: &Gpu, width: u32, height: u32) -> wgpu::Texture {
    Target::new(&gpu.device, width, height, "output").texture
}

/// Reads a FORMAT texture back to tightly packed RGBA (blocking).
pub fn read(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<u8> {
    let (width, height) = (texture.width(), texture.height());
    let row = width * 4;
    let padded = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: padded as u64 * height as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(height) },
        },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    gpu.queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = gpu.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    let Ok(data) = slice.get_mapped_range() else { return vec![0; (row * height) as usize] };
    let mut out = Vec::with_capacity((row * height) as usize);
    for y in 0..height as usize {
        out.extend_from_slice(&data[y * padded as usize..y * padded as usize + row as usize]);
    }
    drop(data);
    buffer.unmap();
    out
}
