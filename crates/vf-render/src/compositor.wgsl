// ViralFlow compositor. All colours are sRGB-encoded and premultiplied, like the
// browser canvas and the CPU renderer this replaces.

struct Layer {
    // Box space (pixels, origin top-left) to output pixels: p = m * q + t.
    m: vec4<f32>,          // m00 m01 m10 m11
    t_out: vec4<f32>,      // tx ty out_w out_h
    src_rect: vec4<f32>,   // u0 v0 du dv (normalised source rectangle)
    box_info: vec4<f32>,   // box_w box_h radius_px kind
    color: vec4<f32>,      // shape colour (straight alpha)
    params: vec4<f32>,     // opacity vignette grain scanlines
    clip: vec4<f32>,       // clip_left clip_right seed glitch(0/1)
    glitch_a: vec4<f32>,   // band offsets 0..3 (fraction of box width)
    glitch_b: vec4<f32>,   // band offsets 4..5, has_matrix, source_h_px
    matrix: array<vec4<f32>, 5>,
    fx_a: vec4<f32>,       // chroma (share of box width), pixel block (box px), posterize levels, sharpen
    fx_b: vec4<f32>,       // bulge, mirror (0/1), zoom blur, light leak
    fx_c: vec4<f32>,       // leak position, clip top, clip bottom, iris (share of half diagonal, >= 1 off)
    fx_d: vec4<f32>,       // clock (share of a turn, >= 1 off), unused x3
};

@group(0) @binding(0) var<uniform> layer: Layer;
@group(1) @binding(0) var source_tex: texture_2d<f32>;
@group(1) @binding(1) var aux_tex: texture_2d<f32>;
@group(1) @binding(2) var linear_sampler: sampler;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) screen: vec2<f32>,
};

fn corner(index: u32) -> vec2<f32> {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    return corners[index];
}

@vertex
fn vs_layer(@builtin(vertex_index) index: u32) -> VsOut {
    // Grow the quad by one pixel so anti-aliased edges are not cut.
    let pad = vec2<f32>(1.0, 1.0) / max(layer.box_info.xy, vec2<f32>(1.0, 1.0));
    let uv = corner(index) * (1.0 + 2.0 * pad) - pad;
    let q = uv * layer.box_info.xy;
    let p = vec2<f32>(layer.m.x * q.x + layer.m.y * q.y, layer.m.z * q.x + layer.m.w * q.y) + layer.t_out.xy;
    let size = layer.t_out.zw;
    var out: VsOut;
    out.position = vec4<f32>(p.x / size.x * 2.0 - 1.0, 1.0 - p.y / size.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    out.screen = p / size;
    return out;
}

fn hash(p: vec2<f32>, seed: f32) -> f32 {
    let h = dot(p, vec2<f32>(127.1, 311.7)) + seed * 74.7;
    return fract(sin(h) * 43758.5453);
}

// Coverage (0..1) of the rounded box at box pixel q, anti-aliased over one output pixel.
fn box_coverage(q: vec2<f32>, size: vec2<f32>, radius: f32, aa: f32) -> f32 {
    let r = min(radius, min(size.x, size.y) * 0.5);
    let d = abs(q - size * 0.5) - (size * 0.5 - vec2<f32>(r, r));
    let dist = length(max(d, vec2<f32>(0.0, 0.0))) + min(max(d.x, d.y), 0.0) - r;
    return clamp(0.5 - dist / aa, 0.0, 1.0);
}

fn ellipse_coverage(q: vec2<f32>, size: vec2<f32>, aa: f32) -> f32 {
    let p = (q - size * 0.5) / (size * 0.5);
    let dist = (length(p) - 1.0) * min(size.x, size.y) * 0.5;
    return clamp(0.5 - dist / aa, 0.0, 1.0);
}

fn apply_matrix(c: vec4<f32>) -> vec4<f32> {
    if (layer.glitch_b.z < 0.5 || c.a <= 0.0) {
        return c;
    }
    let s = vec4<f32>(c.rgb / c.a, c.a);
    let r = dot(layer.matrix[0], s) + layer.matrix[4].x;
    let g = dot(layer.matrix[1], s) + layer.matrix[4].y;
    let b = dot(layer.matrix[2], s) + layer.matrix[4].z;
    let a = dot(layer.matrix[3], s) + layer.matrix[4].w;
    let out = clamp(vec4<f32>(r, g, b, a), vec4<f32>(0.0), vec4<f32>(1.0));
    return vec4<f32>(out.rgb * out.a, out.a);
}

// Where in the box a pixel samples once the lens, the mirror and the pixel
// blocks have moved it.
fn distort(uv_in: vec2<f32>) -> vec2<f32> {
    var uv = uv_in;
    if (layer.fx_b.y > 0.5 && uv.x > 0.5) {
        uv.x = 1.0 - uv.x;
    }
    let bulge = layer.fx_b.x;
    if (abs(bulge) > 0.001) {
        // Square space, so the lens is round whatever the box's shape.
        let size = layer.box_info.xy;
        let unit = size / max(size.x, size.y);
        let d = (uv - vec2<f32>(0.5)) * unit;
        let r2 = clamp(dot(d, d) * 2.0, 0.0, 1.0);
        let k = max(1.0 - bulge * (1.0 - r2), 0.1);
        uv = vec2<f32>(0.5) + (uv - vec2<f32>(0.5)) * k;
    }
    let block = layer.fx_a.y;
    if (block > 1.0) {
        let cells = max(layer.box_info.xy / block, vec2<f32>(1.0));
        uv = (floor(uv * cells) + vec2<f32>(0.5)) / cells;
    }
    return uv;
}

fn source_at(uv: vec2<f32>) -> vec4<f32> {
    let src = layer.src_rect.xy + clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)) * layer.src_rect.zw;
    return textureSampleLevel(source_tex, linear_sampler, src, 0.0);
}

// The source pixel at box uv, with the colour fringes and sharpening.
fn fetch(uv: vec2<f32>) -> vec4<f32> {
    var c = source_at(uv);
    let shift = layer.fx_a.x;
    if (shift > 0.0) {
        let r = source_at(uv + vec2<f32>(shift, 0.0));
        let b = source_at(uv - vec2<f32>(shift, 0.0));
        c = vec4<f32>(r.r, c.g, b.b, max(c.a, max(r.a, b.a)));
    }
    let sharpen = layer.fx_a.w;
    if (sharpen > 0.0) {
        let px = vec2<f32>(1.0) / max(layer.box_info.xy, vec2<f32>(1.0));
        let around = source_at(uv + vec2<f32>(px.x, 0.0)) + source_at(uv - vec2<f32>(px.x, 0.0))
            + source_at(uv + vec2<f32>(0.0, px.y)) + source_at(uv - vec2<f32>(0.0, px.y));
        c = vec4<f32>(clamp(c.rgb + (c.rgb * 4.0 - around.rgb) * sharpen * 0.5, vec3<f32>(0.0), vec3<f32>(c.a)), c.a);
    }
    return c;
}

// The picture at box uv: one fetch, or twelve along the way to the centre for a zoom blur.
fn picture(uv: vec2<f32>) -> vec4<f32> {
    let zoom = layer.fx_b.z;
    if (zoom <= 0.001) {
        return fetch(distort(uv));
    }
    var sum = vec4<f32>(0.0);
    for (var i = 0; i < 12; i = i + 1) {
        let k = 1.0 - zoom * 0.3 * f32(i) / 11.0;
        sum += fetch(distort(vec2<f32>(0.5) + (uv - vec2<f32>(0.5)) * k));
    }
    return sum / 12.0;
}

fn posterize(c: vec4<f32>) -> vec4<f32> {
    let levels = layer.fx_a.z;
    if (levels < 2.0 || c.a <= 0.0) {
        return c;
    }
    let steps = levels - 1.0;
    let straight = floor(c.rgb / c.a * steps + vec3<f32>(0.5)) / steps;
    return vec4<f32>(straight * c.a, c.a);
}

// A warm glow drifting across the box: the light adds orange and burns the
// blue out of what it falls on, as light leaking into a film camera does.
fn light_leak(c: vec4<f32>, uv: vec2<f32>) -> vec4<f32> {
    let strength = layer.fx_b.w;
    if (strength <= 0.001 || c.a <= 0.0) {
        return c;
    }
    let centre = vec2<f32>(mix(-0.2, 1.2, layer.fx_c.x), 0.3);
    let d = (uv - centre) * vec2<f32>(1.0, 1.4);
    let glow = clamp(strength * exp(-dot(d, d) * 3.0), 0.0, 1.0);
    let warmed = c.rgb / c.a * (vec3<f32>(1.0) - glow * vec3<f32>(0.0, 0.15, 0.75));
    let lit = clamp(warmed + vec3<f32>(1.0, 0.55, 0.1) * glow, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(lit * c.a, c.a);
}

// Coverage of the masks transitions draw with: the vertical wipes, the iris
// and the clock hand.
fn mask_coverage(uv: vec2<f32>, size: vec2<f32>, aa: f32) -> f32 {
    var cov = clamp((uv.y - layer.fx_c.y) * size.y / aa + 0.5, 0.0, 1.0) * clamp((1.0 - layer.fx_c.z - uv.y) * size.y / aa + 0.5, 0.0, 1.0);
    let p = uv * size - size * 0.5;
    let iris = layer.fx_c.w;
    if (iris < 1.0) {
        cov *= clamp((iris * length(size) * 0.5 - length(p)) / aa + 0.5, 0.0, 1.0);
    }
    let clock = layer.fx_d.x;
    if (clock < 1.0) {
        let turn = fract(atan2(p.x, -p.y) / 6.2831853 + 1.0);
        cov *= clamp((clock - turn) * 6.2831853 * length(p) / aa + 0.5, 0.0, 1.0) * step(0.0001, clock);
    }
    return cov;
}

@fragment
fn fs_layer(input: VsOut) -> @location(0) vec4<f32> {
    let size = layer.box_info.xy;
    let kind = layer.box_info.w;
    // Box pixels per output pixel, for anti-aliasing in box space.
    let scale = max(length(vec2<f32>(layer.m.x, layer.m.z)), 1e-4);
    let aa = 1.0 / scale;
    var uv = input.uv;
    var q = uv * size;

    // Wipes and the typewriter reveal clip the box horizontally.
    let clip_cov = clamp((uv.x - layer.clip.x) * size.x / aa + 0.5, 0.0, 1.0) * clamp((layer.clip.y - uv.x) * size.x / aa + 0.5, 0.0, 1.0);
    var coverage = clip_cov * mask_coverage(uv, size, aa);

    // Glitch: six horizontal bands, each shifted sideways inside the box.
    if (layer.clip.w > 0.5) {
        let band = u32(clamp(floor(uv.y * 6.0), 0.0, 5.0));
        var offsets = array<f32, 6>(layer.glitch_a.x, layer.glitch_a.y, layer.glitch_a.z, layer.glitch_a.w, layer.glitch_b.x, layer.glitch_b.y);
        uv.x = uv.x - offsets[band];
    }

    var color: vec4<f32>;
    if (kind < 0.5) {
        // Media or text: rounded box mask, then the source pixel.
        coverage *= box_coverage(input.uv * size, size, layer.box_info.z, aa);
        let inside = step(0.0, uv.x) * step(uv.x, 1.0);
        let src = layer.src_rect.xy + clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)) * layer.src_rect.zw;
        color = picture(uv) * inside;
        color = apply_matrix(color);
        color = posterize(color);
        color = light_leak(color, input.uv);
        // Vignette and grain work in source space, as on the CPU.
        if (layer.params.y > 0.001) {
            let d = (src - vec2<f32>(0.5)) * 2.0;
            let distance = length(d) / sqrt(2.0);
            let t = clamp((distance - 0.35) / 0.65, 0.0, 1.0);
            let falloff = 1.0 - layer.params.y * 0.85 * (t * t * (3.0 - 2.0 * t));
            color = vec4<f32>(color.rgb * falloff, color.a);
        }
        if (layer.params.z > 0.0 || layer.params.w > 0.5) {
            let pixel = floor(src * vec2<f32>(textureDimensions(source_tex)));
            let noise = (hash(pixel, layer.clip.z) - 0.5) * layer.params.z * 2.0 * color.a;
            var line = 1.0;
            if (layer.params.w > 0.5 && u32(pixel.y) % 4u == 0u) {
                line = 0.85;
            }
            color = vec4<f32>(clamp((color.rgb + vec3<f32>(noise)) * line, vec3<f32>(0.0), vec3<f32>(color.a)), color.a);
        }
    } else if (kind < 3.5) {
        // Shapes: rectangle (1), circle (2), line (3).
        if (kind > 1.5 && kind < 2.5) {
            coverage *= ellipse_coverage(q, size, aa);
        } else if (kind > 2.5) {
            coverage *= box_coverage(q, size, size.y * 0.5, aa);
        } else {
            coverage *= box_coverage(q, size, layer.box_info.z, aa);
        }
        color = vec4<f32>(layer.color.rgb * layer.color.a, layer.color.a);
    } else {
        // Blur region: what is underneath, blurred, inside the rounded box.
        coverage *= box_coverage(q, size, layer.box_info.z, aa);
        color = textureSample(aux_tex, linear_sampler, input.screen);
    }
    return color * coverage * layer.params.x;
}

// ---------------------------------------------------------------- full-screen passes

struct Pass {
    // Blur: direction (x, y) in texels of the source, sigma, taps.
    a: vec4<f32>,
    // Composite: blend mode, unused...
    b: vec4<f32>,
};

@group(0) @binding(1) var<uniform> pass_info: Pass;

@vertex
fn vs_full(@builtin(vertex_index) index: u32) -> VsOut {
    var points = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    let p = points[index];
    var out: VsOut;
    out.position = vec4<f32>(p, 0.0, 1.0);
    out.uv = vec2<f32>((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5);
    out.screen = out.uv;
    return out;
}

@fragment
fn fs_blur(input: VsOut) -> @location(0) vec4<f32> {
    let texel = pass_info.a.xy;
    let sigma = max(pass_info.a.z, 0.01);
    let taps = i32(pass_info.a.w);
    var sum = vec4<f32>(0.0);
    var weight_sum = 0.0;
    for (var i = -taps; i <= taps; i = i + 1) {
        let w = exp(-f32(i * i) / (2.0 * sigma * sigma));
        sum += textureSample(source_tex, linear_sampler, input.uv + texel * f32(i)) * w;
        weight_sum += w;
    }
    return sum / weight_sum;
}

fn blend_channel(mode: i32, cb: f32, cs: f32) -> f32 {
    switch mode {
        case 1: { return cb * cs; }                                   // multiply
        case 2: { return cb + cs - cb * cs; }                         // screen
        case 3: {                                                     // overlay
            if (cb <= 0.5) { return 2.0 * cb * cs; }
            return 1.0 - 2.0 * (1.0 - cb) * (1.0 - cs);
        }
        case 4: { return min(cb, cs); }                               // darken
        case 5: { return max(cb, cs); }                               // lighten
        case 6: {                                                     // color-dodge
            if (cb <= 0.0) { return 0.0; }
            if (cs >= 1.0) { return 1.0; }
            return min(1.0, cb / (1.0 - cs));
        }
        case 7: {                                                     // color-burn
            if (cb >= 1.0) { return 1.0; }
            if (cs <= 0.0) { return 0.0; }
            return 1.0 - min(1.0, (1.0 - cb) / cs);
        }
        case 8: {                                                     // hard-light
            if (cs <= 0.5) { return 2.0 * cb * cs; }
            return 1.0 - 2.0 * (1.0 - cb) * (1.0 - cs);
        }
        case 9: {                                                     // soft-light
            if (cs <= 0.5) { return cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb); }
            var d: f32;
            if (cb <= 0.25) { d = ((16.0 * cb - 12.0) * cb + 4.0) * cb; } else { d = sqrt(cb); }
            return cb + (2.0 * cs - 1.0) * (d - cb);
        }
        case 10: { return abs(cb - cs); }                             // difference
        case 11: { return cb + cs - 2.0 * cb * cs; }                  // exclusion
        default: { return cs; }                                       // normal
    }
}

@fragment
fn fs_composite(input: VsOut) -> @location(0) vec4<f32> {
    // source_tex: the layer (premultiplied), aux_tex: the backdrop (premultiplied).
    let s = textureSample(source_tex, linear_sampler, input.uv);
    let b = textureSample(aux_tex, linear_sampler, input.uv);
    let mode = i32(pass_info.b.x);
    let cs = select(vec3<f32>(0.0), s.rgb / s.a, s.a > 0.0);
    let cb = select(vec3<f32>(0.0), b.rgb / b.a, b.a > 0.0);
    let mixed = vec3<f32>(blend_channel(mode, cb.r, cs.r), blend_channel(mode, cb.g, cs.g), blend_channel(mode, cb.b, cs.b));
    // W3C compositing: source-over with the blended colour where both overlap.
    let rgb = s.rgb * (1.0 - b.a) + b.rgb * (1.0 - s.a) + s.a * b.a * mixed;
    let a = s.a + b.a * (1.0 - s.a);
    return vec4<f32>(rgb, a);
}

@fragment
fn fs_copy(input: VsOut) -> @location(0) vec4<f32> {
    return textureSample(source_tex, linear_sampler, input.uv);
}
