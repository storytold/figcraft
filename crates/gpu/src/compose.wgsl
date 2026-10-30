// Lays one canvas-sized layer over another: the page's segments are drawn into their own
// textures and laid over the accumulation as premultiplied pixels.
//
// vello's own targets hold straight (separated) alpha — its fine pass stores `rgb / alpha` (see
// `vello_shaders`' `fine.wgsl`) — so every layer it drew is premultiplied here, at the one place
// it meets the premultiplied accumulation, the blurs and the glass shader (which all work on
// premultiplied pixels, like tiny-skia's pixmaps).

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

// The luminance mask pass reads its coverage from a second texture.
@group(0) @binding(2) var mask: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let q = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(q * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(src));
    let s = textureSample(src, samp, pos.xy / dims);
    return vec4<f32>(s.rgb * s.a, s.a);
}

// A downscale (a blur's levels of a large layer): the same triangle, with its uv handed through
// the vertices so the target's own size needs no uniform. The source is already premultiplied.
struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_down(@builtin(vertex_index) i: u32) -> VOut {
    let q = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    // The position is `q * 2 - 1`, so `q` runs 0..1 across the screen (the triangle's third
    // vertex is off it). Clip space has y up, a texture's v down: the bottom takes v = 1.
    return VOut(vec4<f32>(q * 2.0 - 1.0, 0.0, 1.0), vec2<f32>(q.x, 1.0 - q.y));
}

@fragment
fn fs_down(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    return textureSample(src, samp, uv);
}

// A luminance mask: the content times the mask's coverage, `ceil(255 × luma × alpha) / 255` of the
// mask's straight colour — tiny-skia's `MaskType::Luminance`, which the CPU renderer uses. The
// content is a rendered layer (premultiplied), the mask another.
@fragment
fn fs_luma(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(src));
    let s = textureSample(src, samp, pos.xy / dims);
    let m = textureSample(mask, samp, pos.xy / dims);
    let a = m.a;
    let rgb = select(vec3<f32>(0.0), m.rgb / max(a, 1e-6), a > 0.0);
    let luma = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    let f = ceil(clamp(luma * a, 0.0, 1.0) * 255.0) / 255.0;
    return vec4<f32>(s.rgb * f, s.a * f);
}
