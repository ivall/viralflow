//! Renders small scenes on the real GPU and checks pixels (skipped without a GPU).

use vf_core::engine::fx::LayerFx;
use vf_core::engine::scene::Layer;
use vf_core::model::{BlendMode, ItemKind};
use vf_render::compositor::{read, render_target, upload, Compositor, Content, DrawLayer, ShapeKind};
use vf_render::Gpu;

const W: u32 = 64;
const H: u32 = 64;

fn layer(cx: f64, cy: f64, w: f64, h: f64) -> Layer {
    Layer {
        track: 0,
        index: 0,
        kind: ItemKind::Shape,
        box_w: w,
        box_h: h,
        source: None,
        media_time: None,
        cx,
        cy,
        scale: 1.0,
        rotation: 0.0,
        flip_x: false,
        flip_y: false,
        opacity: 1.0,
        blur: 0.0,
        blend: BlendMode::Normal,
        radius: 0.0,
        matrix: None,
        vignette: 0.0,
        clip_left: 0.0,
        clip_right: 1.0,
        grain: 0.0,
        scanlines: false,
        glitch: vec![],
        fx: LayerFx::default(),
    }
}

fn pixel(data: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * W + x) * 4) as usize;
    [data[i], data[i + 1], data[i + 2], data[i + 3]]
}

fn close(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (*x as i32 - y as i32).abs() <= 3)
}

fn setup() -> Option<(Gpu, Compositor, wgpu::Texture)> {
    let gpu = Gpu::new().ok()?;
    let compositor = Compositor::new(&gpu);
    let target = render_target(&gpu, W, H);
    Some((gpu, compositor, target))
}

#[test]
fn draws_background_and_shapes() {
    let Some((gpu, mut compositor, target)) = setup() else { return };
    let rect = layer(32.0, 32.0, 32.0, 32.0);
    compositor.render(
        &target,
        1.0,
        [0.0, 0.0, 1.0],
        &[DrawLayer { layer: &rect, content: Content::Shape { kind: ShapeKind::Rect, color: [1.0, 0.0, 0.0], radius: 0.0 } }],
        0,
    );
    let data = read(&gpu, &target);
    assert!(close(pixel(&data, 2, 2), [0, 0, 255, 255]), "background {:?}", pixel(&data, 2, 2));
    assert!(close(pixel(&data, 32, 32), [255, 0, 0, 255]), "shape {:?}", pixel(&data, 32, 32));
    assert!(close(pixel(&data, 20, 20), [255, 0, 0, 255]), "inside edge {:?}", pixel(&data, 20, 20));
    assert!(close(pixel(&data, 14, 32), [0, 0, 255, 255]), "outside {:?}", pixel(&data, 14, 32));
}

#[test]
fn rotation_and_scale_move_the_box() {
    let Some((gpu, mut compositor, target)) = setup() else { return };
    // A wide bar rotated 90° becomes tall.
    let mut bar = layer(32.0, 32.0, 40.0, 6.0);
    bar.rotation = 90.0;
    compositor.render(
        &target,
        1.0,
        [0.0, 0.0, 0.0],
        &[DrawLayer { layer: &bar, content: Content::Shape { kind: ShapeKind::Rect, color: [1.0, 1.0, 1.0], radius: 0.0 } }],
        0,
    );
    let data = read(&gpu, &target);
    assert!(close(pixel(&data, 32, 16), [255, 255, 255, 255]), "rotated bar covers the top");
    assert!(close(pixel(&data, 16, 32), [0, 0, 0, 255]), "rotated bar leaves the left");
}

#[test]
fn textures_blend_premultiplied() {
    let Some((gpu, mut compositor, target)) = setup() else { return };
    // 2×2 texture of 50% white (premultiplied 128,128,128,128).
    let texture = upload(&gpu, 2, 2, &[128; 16], "half");
    let view = texture.create_view(&Default::default());
    let mut picture = layer(32.0, 32.0, 64.0, 64.0);
    picture.kind = ItemKind::Image;
    compositor.render(&target, 1.0, [0.0, 0.0, 0.0], &[DrawLayer { layer: &picture, content: Content::Texture { view: &view, source_size: (2.0, 2.0) } }], 0);
    let data = read(&gpu, &target);
    assert!(close(pixel(&data, 32, 32), [128, 128, 128, 255]), "half white over black {:?}", pixel(&data, 32, 32));
}

#[test]
fn multiply_and_opacity() {
    let Some((gpu, mut compositor, target)) = setup() else { return };
    let mut gray = layer(32.0, 32.0, 64.0, 64.0);
    gray.blend = BlendMode::Multiply;
    let mut faint = layer(32.0, 32.0, 16.0, 16.0);
    faint.opacity = 0.5;
    compositor.render(
        &target,
        1.0,
        [1.0, 1.0, 1.0],
        &[
            DrawLayer { layer: &gray, content: Content::Shape { kind: ShapeKind::Rect, color: [0.5, 0.5, 0.5], radius: 0.0 } },
            DrawLayer { layer: &faint, content: Content::Shape { kind: ShapeKind::Rect, color: [0.0, 0.0, 0.0], radius: 0.0 } },
        ],
        0,
    );
    let data = read(&gpu, &target);
    assert!(close(pixel(&data, 4, 4), [128, 128, 128, 255]), "white × gray {:?}", pixel(&data, 4, 4));
    assert!(close(pixel(&data, 32, 32), [64, 64, 64, 255]), "half black over gray {:?}", pixel(&data, 32, 32));
}

#[test]
fn blur_region_softens_what_is_below() {
    let Some((gpu, mut compositor, target)) = setup() else { return };
    // Left half white, right half black; blur a box across the edge.
    let white = layer(16.0, 32.0, 32.0, 64.0);
    let region = layer(32.0, 32.0, 40.0, 40.0);
    compositor.render(
        &target,
        1.0,
        [0.0, 0.0, 0.0],
        &[
            DrawLayer { layer: &white, content: Content::Shape { kind: ShapeKind::Rect, color: [1.0, 1.0, 1.0], radius: 0.0 } },
            DrawLayer { layer: &region, content: Content::BlurRegion { strength: 6.0 } },
        ],
        0,
    );
    let data = read(&gpu, &target);
    let edge = pixel(&data, 33, 32)[0];
    assert!(edge > 30 && edge < 225, "edge is softened, got {edge}");
    assert!(close(pixel(&data, 2, 32), [255, 255, 255, 255]), "outside the region stays sharp");
}

/// A 4×1 picture: red, green, blue, white (premultiplied, opaque).
fn stripes(gpu: &Gpu) -> wgpu::Texture {
    upload(gpu, 4, 1, &[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255], "stripes")
}

fn draw_picture(fx: LayerFx) -> Option<Vec<u8>> {
    let (gpu, mut compositor, target) = setup()?;
    let texture = stripes(&gpu);
    let view = texture.create_view(&Default::default());
    let mut picture = layer(32.0, 32.0, 64.0, 64.0);
    picture.kind = ItemKind::Image;
    picture.fx = fx;
    compositor.render(&target, 1.0, [0.0, 0.0, 0.0], &[DrawLayer { layer: &picture, content: Content::Texture { view: &view, source_size: (4.0, 1.0) } }], 0);
    Some(read(&gpu, &target))
}

#[test]
fn mirror_repeats_the_left_half() {
    let Some(data) = draw_picture(LayerFx { mirror: true, ..Default::default() }) else { return };
    // Pixels inside the outer texels, where the linear sampling is the texel itself.
    assert!(close(pixel(&data, 5, 32), [255, 0, 0, 255]), "left stays {:?}", pixel(&data, 5, 32));
    assert!(close(pixel(&data, 58, 32), [255, 0, 0, 255]), "right shows the left {:?}", pixel(&data, 58, 32));
}

#[test]
fn iris_and_clock_hide_part_of_the_box() {
    let Some(data) = draw_picture(LayerFx { iris: 0.3, ..Default::default() }) else { return };
    assert!(close(pixel(&data, 2, 2), [0, 0, 0, 255]), "corner outside the circle {:?}", pixel(&data, 2, 2));
    assert!(pixel(&data, 31, 32)[3] == 255 && pixel(&data, 31, 32) != [0, 0, 0, 255], "centre inside");
    // A quarter turn: the top right quarter shows, the bottom left does not.
    let Some(data) = draw_picture(LayerFx { clock: 0.25, ..Default::default() }) else { return };
    assert!(close(pixel(&data, 56, 8), [255, 255, 255, 255]), "top right {:?}", pixel(&data, 56, 8));
    assert!(close(pixel(&data, 8, 56), [0, 0, 0, 255]), "bottom left {:?}", pixel(&data, 8, 56));
    let Some(data) = draw_picture(LayerFx { clip_top: 0.5, ..Default::default() }) else { return };
    assert!(close(pixel(&data, 8, 8), [0, 0, 0, 255]), "wiped top {:?}", pixel(&data, 8, 8));
    assert!(close(pixel(&data, 5, 56), [255, 0, 0, 255]), "bottom stays {:?}", pixel(&data, 5, 56));
}

#[test]
fn pixel_blocks_take_one_colour() {
    // Blocks as wide as half the box: each half is one sample of the stripes.
    let Some(data) = draw_picture(LayerFx { pixelate: 32.0, ..Default::default() }) else { return };
    assert_eq!(pixel(&data, 1, 32), pixel(&data, 30, 32), "one block, one colour");
}

#[test]
fn rgb_split_pulls_red_from_the_right() {
    // Shifted by a quarter of the box, the red channel at the green stripe comes from the blue one (no red).
    let Some(data) = draw_picture(LayerFx { chroma: 0.25, ..Default::default() }) else { return };
    let green = pixel(&data, 24, 32);
    assert!(green[1] > 200 && green[0] < 40, "green stripe {:?}", green);
    // The red stripe gets its blue from further left (clamped to red itself: no blue) and red from green (none).
    let red = pixel(&data, 8, 32);
    assert!(red[0] < 40, "red channel moved away {:?}", red);
}
