// A glass pane's pixel: the backdrop seen through a lens that curves down at the rim, bent towards
// the inside, each channel by a slightly different amount, catching the light at the edge — the
// same arithmetic the CPU paints with.

struct Params {
    cw: f32,
    ch: f32,
    m: f32,
    pad: f32,
    depth: f32,
    bend_max: f32,
    fringe: f32,
    power: f32,
    band: f32,
    reach: f32,
    floor: f32,
    fall: f32,
    intensity: f32,
    light_x: f32,
    light_y: f32,
    unused: f32,
};

@group(0) @binding(0) var frosted: texture_2d<f32>;
@group(0) @binding(1) var frosted_sampler: sampler;
@group(0) @binding(2) var coverage: texture_2d<f32>;
@group(0) @binding(3) var distance: texture_2d<f32>;
@group(0) @binding(4) var normals: texture_2d<f32>;
@group(0) @binding(5) var<uniform> p: Params;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let q = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(q * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let out = vec2<i32>(i32(pos.x), i32(pos.y));
    let at = out + vec2<i32>(i32(p.m), i32(p.m));
    let c = textureLoad(coverage, at, 0).r;
    if (c <= 0.0) {
        return vec4<f32>(0.0);
    }
    let d = textureLoad(distance, at, 0).r;
    let inward = textureLoad(normals, at, 0).rg;
    let t = min(d / p.depth, 1.0);
    let bend = p.bend_max * (1.0 - t) * (1.0 - t);
    let dims = vec2<f32>(textureDimensions(frosted));
    let b = vec2<f32>(f32(out.x) + p.pad + 0.5, f32(out.y) + p.pad + 0.5);
    var base: vec4<f32>;
    if (bend == 0.0) {
        let s = textureSampleLevel(frosted, frosted_sampler, b / dims, 0.0);
        base = vec4<f32>(min(s.r, s.a), min(s.g, s.a), min(s.b, s.a), s.a);
    } else {
        let rs = textureSampleLevel(frosted, frosted_sampler, (b + inward * bend * (1.0 - p.fringe)) / dims, 0.0);
        let gs = textureSampleLevel(frosted, frosted_sampler, (b + inward * bend) / dims, 0.0);
        let bs = textureSampleLevel(frosted, frosted_sampler, (b + inward * bend * (1.0 + p.fringe)) / dims, 0.0);
        let a = max(rs.a, max(gs.a, bs.a));
        base = vec4<f32>(min(rs.r, a), min(gs.g, a), min(bs.b, a), a);
    }
    var hl = 0.0;
    if (d < p.reach && p.fall > 0.0) {
        let facing = -(inward.x * p.light_x + inward.y * p.light_y);
        let spec = pow(max(facing, 0.0), p.power) + 0.5 * pow(max(-facing, 0.0), p.power);
        hl = clamp(p.intensity * spec * (exp(-d / p.band) - p.floor) / p.fall, 0.0, 1.0);
    }
    let rgb = (base.rgb * (1.0 - hl) + vec3<f32>(hl)) * c;
    return vec4<f32>(rgb, base.a * c);
}
