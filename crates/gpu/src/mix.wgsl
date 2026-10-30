// Progressive blur: the renderer blurs a layer at a few radii between its smallest and largest
// and blends every pixel between the two levels around its own radius (see
// `figcraft_render::blur::{levels, bracket}`). This pass is that blend: it picks each pixel's two
// levels from the ramp and mixes them.

struct Params {
    // The levels' standard deviations, nearest first (at most eight).
    levels: array<vec4<f32>, 2>,
    count: f32,
    s0: f32,
    s1: f32,
    // t at pixel (0, 0)'s centre and its change per pixel across and down.
    t0: f32,
    tx: f32,
    ty: f32,
    _pad0: f32,
    _pad1: f32,
    // Each level's top-left in the layer: a level is blurred only where it is used, plus its
    // reach, so its texture is that crop (the blend reads it at the pixel less the crop). Two
    // levels to a `vec4`.
    offsets: array<vec4<f32>, 4>,
    // The resolution each level was blurred at (a large layer's levels at half, the layer itself
    // at one): the blend reads them all at the layer's own scale.
    scales: array<vec4<f32>, 2>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var level0: texture_2d<f32>;
@group(0) @binding(2) var level1: texture_2d<f32>;
@group(0) @binding(3) var level2: texture_2d<f32>;
@group(0) @binding(4) var level3: texture_2d<f32>;
@group(0) @binding(5) var level4: texture_2d<f32>;
@group(0) @binding(6) var level5: texture_2d<f32>;
@group(0) @binding(7) var level6: texture_2d<f32>;
@group(0) @binding(8) var level7: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let q = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(q * 2.0 - 1.0, 0.0, 1.0);
}

// `c` in `t`'s own pixels (past its edges reads the edge, where no weight is left).
fn load(t: texture_2d<f32>, c: vec2<i32>) -> vec4<f32> {
    let last = vec2<i32>(textureDimensions(t)) - 1;
    return textureLoad(t, clamp(c, vec2<i32>(0), max(last, vec2<i32>(0))), 0);
}

fn offset_at(k: i32) -> vec2<f32> {
    let v = params.offsets[k / 2];
    return select(v.xy, v.zw, (k % 2) == 1);
}

fn scale_at(k: i32) -> f32 {
    return params.scales[k / 4][k % 4];
}

// `p` in a level's texel coordinates (a whole number reads that texel), blended by bilinear
// weights: a pixel of a level blurred at half resolution covers two of the layer's.
fn sample(t: texture_2d<f32>, p: vec2<f32>) -> vec4<f32> {
    let i = floor(p);
    let f = p - i;
    let at = vec2<i32>(i);
    let c00 = load(t, at);
    let c10 = load(t, at + vec2<i32>(1, 0));
    let c01 = load(t, at + vec2<i32>(0, 1));
    let c11 = load(t, at + vec2<i32>(1, 1));
    return mix(mix(c00, c10, f.x), mix(c01, c11, f.x), f.y);
}

fn at_level(k: i32, c: vec2<i32>) -> vec4<f32> {
    let q = (vec2<f32>(c) - offset_at(k)) * scale_at(k);
    switch k {
        case 0: { return sample(level0, q); }
        case 1: { return sample(level1, q); }
        case 2: { return sample(level2, q); }
        case 3: { return sample(level3, q); }
        case 4: { return sample(level4, q); }
        case 5: { return sample(level5, q); }
        case 6: { return sample(level6, q); }
        default: { return sample(level7, q); }
    }
}

fn sigma_at(k: i32) -> f32 {
    return params.levels[k / 4][k % 4];
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let c = vec2<i32>(pos.xy);
    let n = i32(params.count);
    if (n < 2) {
        return at_level(max(n - 1, 0), c);
    }
    let t = clamp(params.t0 + pos.x * params.tx + pos.y * params.ty, 0.0, 1.0);
    let s = params.s0 + (params.s1 - params.s0) * t;
    var k = 0;
    for (var i = 0; i < n; i = i + 1) {
        if (sigma_at(i) <= s) {
            k = i;
        }
    }
    k = min(k, n - 2);
    let lo = sigma_at(k);
    let hi = sigma_at(k + 1);
    let f = select(0.0, (s - lo) / (hi - lo), hi > lo);
    // The renderer's weights are whole 1/255ths.
    let w = round(clamp(f, 0.0, 1.0) * 255.0) / 255.0;
    return mix(at_level(k, c), at_level(k + 1, c), w);
}
