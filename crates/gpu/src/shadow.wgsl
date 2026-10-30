// Shadow passes: the renderer's `fx::tint` (a blurred silhouette in the shadow's colour) and
// `fx::morph` (a layer's alpha grown or shrunk by whole pixels, white), on premultiplied pixels.

struct Params {
    color: vec4<f32>,
    // Morph: whole pixels grown (positive) or shrunk (negative) along the pass's axis.
    radius: f32,
    // Morph: give 1 - alpha (an inner shadow's outside) rather than alpha.
    invert: f32,
    // Morph: 0 down the columns, 1 along the rows.
    axis: f32,
    _pad: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: Params;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let q = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(q * 2.0 - 1.0, 0.0, 1.0);
}

// Every pixel becomes `color` at its own alpha × the colour's alpha (alpha only: the source's
// colour is a coverage, not a picture).
@fragment
fn tint(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let a = textureLoad(src, vec2<i32>(pos.xy), 0).a * params.color.a;
    return vec4<f32>(params.color.rgb * a, a);
}

// A square (separable) maximum/minimum over the alpha, as white; outside the layer counts as empty.
@fragment
fn morph(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(textureDimensions(src));
    let c = vec2<i32>(pos.xy);
    let grow = params.radius >= 0.0;
    let r = i32(abs(params.radius));
    let dir = select(vec2<i32>(0, 1), vec2<i32>(1, 0), params.axis >= 0.5);
    var m = select(0.0, 1.0, !grow);
    for (var k = -r; k <= r; k = k + 1) {
        let q = c + dir * k;
        var v = 0.0;
        if (q.x >= 0 && q.y >= 0 && q.x < dims.x && q.y < dims.y) {
            v = textureLoad(src, q, 0).a;
        }
        if (grow) {
            m = max(m, v);
        } else {
            m = min(m, v);
        }
    }
    if (params.invert >= 0.5) {
        m = 1.0 - m;
    }
    return vec4<f32>(m, m, m, m);
}
