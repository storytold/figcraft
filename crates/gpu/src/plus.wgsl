// Plus-darker onto the page: the CPU renderer's `fx::plus_darker` — a linear burn through the W3C
// general compositing formula — which vello has no blend mode for. `under` is a copy of the page
// (premultiplied; a pass cannot read what it writes), `src` the node's own pixels straight from
// vello (straight alpha), premultiplied here.

@group(0) @binding(0) var under: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var src: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let q = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(q * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(under));
    let d = textureSample(under, samp, pos.xy / dims);
    let straight = textureSample(src, samp, pos.xy / dims);
    let s = vec4<f32>(straight.rgb * straight.a, straight.a);
    let v = s.rgb * (1.0 - d.a) + d.rgb * (1.0 - s.a) + max(s.a * d.rgb + d.a * s.rgb - s.a * d.a, vec3<f32>(0.0));
    let a = s.a + d.a - s.a * d.a;
    return vec4<f32>(v, a);
}
