// A box blur pass of `radius` pixels, along rows or down columns (the CPU blur's passes), in
// whole 0–255 units so the floor matches the CPU's integer division:
//
// `box` is the pass: a workgroup takes 256 pixels of a line with the halo their windows read,
// scans the halo in shared memory, and writes their averages — one dispatch, no texture between
// passes. A radius past the halo's size uses `scan` (every pixel's running sum along its line)
// and `fs_box` (the window's sum as the difference of two of them) instead.
//
// So a pass costs the same whatever the radius (a radius of thousands costs what one costs).

struct Params {
    radius: u32,
    vertical: u32,
    /// The crop's top-left in the layer the first pass reads (0, 0 after it).
    origin: vec2<u32>,
    /// The crop's own size: its passes read and write that many pixels.
    size: vec2<u32>,
    _pad: vec2<u32>,
};

const TILE: u32 = 256u;
/// The largest radius `box` scans a halo for.
const MAX_RADIUS: u32 = 256u;
/// The halo for that radius: 256 pixels plus one each side.
const HALO: u32 = 768u;

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var prefix_out: texture_storage_2d<rgba32float, write>;
/// `box`'s own pass: straight into the next texture (no sums in between).
@group(0) @binding(3) var boxed: texture_storage_2d<rgba8unorm, write>;

var<workgroup> tile: array<vec4<f32>, 256>;
var<workgroup> halo: array<vec4<f32>, 768>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

// Pixel `i` of `line` in the crop, along the pass's line.
fn at(i: i32, line: i32, vertical: bool) -> vec2<i32> {
    return select(vec2<i32>(i, line), vec2<i32>(line, i), vertical);
}

// The same pixel of the source (the first pass reads a crop of the layer, at `origin`).
fn in_source(i: i32, line: i32, vertical: bool) -> vec2<i32> {
    return at(i, line, vertical) + vec2<i32>(params.origin);
}

// The CPU divides whole numbers and keeps the floor: do the same, so the two agree.
fn average(sum: vec4<f32>, radius: u32) -> vec4<f32> {
    return clamp(floor(sum / f32(2u * radius + 1u) + 0.0001) / 255.0, vec4<f32>(0.0), vec4<f32>(1.0));
}

// A box pass in one dispatch: a workgroup writes 256 pixels of a line (a tile), scanning the halo
// their windows read. The workgroups are (tile along the line, line) so any line count fits.
@compute @workgroup_size(256, 1, 1)
fn box(@builtin(local_invocation_id) lid: vec3<u32>, @builtin(workgroup_id) wid: vec3<u32>) {
    let vertical = params.vertical == 1u;
    let len = i32(select(params.size.x, params.size.y, vertical));
    let line = i32(wid.y);
    let r = params.radius;
    let n = TILE + 2u * r;
    // The halo: [start - r, start + 256 + r) of the line, zero where it lies outside.
    let first = i32(wid.x * TILE) - i32(r);
    var k = 0u;
    loop {
        if k >= HALO / TILE {
            break;
        }
        let i = lid.x + k * TILE;
        if i < n {
            let at_line = first + i32(i);
            var v = vec4<f32>(0.0);
            if at_line >= 0 && at_line < len {
                v = round(textureLoad(src, in_source(at_line, line, vertical), 0) * 255.0);
            }
            halo[i] = v;
        }
        k = k + 1u;
    }
    workgroupBarrier();
    // An inclusive scan of the halo, three entries a thread a step (Hillis–Steele).
    var step = 1u;
    loop {
        if step >= n {
            break;
        }
        let i0 = lid.x;
        let i1 = lid.x + TILE;
        let i2 = lid.x + 2u * TILE;
        var add0 = vec4<f32>(0.0);
        var add1 = vec4<f32>(0.0);
        var add2 = vec4<f32>(0.0);
        if i0 >= step {
            add0 = halo[i0 - step];
        }
        if i1 >= step && i1 < n {
            add1 = halo[i1 - step];
        }
        if i2 >= step && i2 < n {
            add2 = halo[i2 - step];
        }
        workgroupBarrier();
        halo[i0] = halo[i0] + add0;
        if i1 < n {
            halo[i1] = halo[i1] + add1;
        }
        if i2 < n {
            halo[i2] = halo[i2] + add2;
        }
        workgroupBarrier();
        step = step * 2u;
    }
    // The window's sum: two running sums' difference.
    let t = wid.x * TILE + lid.x;
    if i32(t) < len {
        var sum = halo[lid.x + 2u * r];
        if lid.x > 0u {
            sum = sum - halo[lid.x - 1u];
        }
        textureStore(boxed, at(i32(t), line, vertical), average(sum, r));
    }
}

// The running sums along each line, from each line's start, in whole 0–255 units.
@compute @workgroup_size(256, 1, 1)
fn scan(@builtin(local_invocation_id) lid: vec3<u32>, @builtin(workgroup_id) wid: vec3<u32>) {
    let vertical = params.vertical == 1u;
    let len = select(params.size.x, params.size.y, vertical);
    // The line this workgroup reads, and writes the sums of.
    let line = i32(wid.x);
    // The sums of the tiles behind this one (the same in every thread: they read one value).
    var base = vec4<f32>(0.0);
    var t = 0u;
    loop {
        if t >= len {
            break;
        }
        let i = t + lid.x;
        var v = vec4<f32>(0.0);
        if i < len {
            v = round(textureLoad(src, in_source(i32(i), line, vertical), 0) * 255.0);
        }
        tile[lid.x] = v;
        workgroupBarrier();
        // Hillis–Steele inclusive scan over the tile.
        var step = 1u;
        loop {
            if step >= TILE {
                break;
            }
            var add = vec4<f32>(0.0);
            if lid.x >= step {
                add = tile[lid.x - step];
            }
            workgroupBarrier();
            tile[lid.x] = tile[lid.x] + add;
            workgroupBarrier();
            step = step * 2u;
        }
        if i < len {
            textureStore(prefix_out, at(i32(i), line, vertical), tile[lid.x] + base);
        }
        // The last tile's total moves the next tile's sums (read after the write above).
        base = base + tile[TILE - 1u];
        workgroupBarrier();
        t = t + TILE;
    }
}

// The window's average, from the running sums in `src`.
@fragment
fn fs_box(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let vertical = params.vertical == 1u;
    let p = vec2<i32>(i32(pos.x), i32(pos.y));
    let len = i32(select(params.size.x, params.size.y, vertical));
    let i = select(p.x, p.y, vertical);
    // The pixel's own line.
    let line = select(p.y, p.x, vertical);
    let r = i32(params.radius);
    // Pixels outside the image count as transparent, as they do on the CPU.
    let hi = min(i + r, len - 1);
    let lo = i - r - 1;
    var sum = vec4<f32>(0.0);
    if hi >= 0 {
        sum = textureLoad(src, at(hi, line, vertical), 0);
    }
    if lo >= 0 {
        sum = sum - textureLoad(src, at(lo, line, vertical), 0);
    }
    // The CPU divides whole numbers and keeps the floor: do the same, so the two agree.
    let avg = floor(sum / f32(2 * r + 1) + 0.0001);
    return clamp(avg / 255.0, vec4<f32>(0.0), vec4<f32>(1.0));
}

// A box pass that reads its window directly: for a device that can't run `scan`.
@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(params.size);
    let p = vec2<i32>(i32(pos.x), i32(pos.y));
    let r = i32(params.radius);
    let vertical = params.vertical == 1u;
    var sum = vec4<f32>(0.0);
    // Outside the image counts as transparent, as it does on the CPU.
    for (var i = -r; i <= r; i = i + 1) {
        var q = p;
        if (vertical) {
            q.y = q.y + i;
        } else {
            q.x = q.x + i;
        }
        if (q.x >= 0 && q.y >= 0 && q.x < dims.x && q.y < dims.y) {
            sum = sum + textureLoad(src, q + vec2<i32>(params.origin), 0);
        }
    }
    // The CPU blur divides whole numbers and keeps the floor: do the same, so the two agree.
    return floor(sum / f32(2 * r + 1) * 255.0 + 0.0001) / 255.0;
}
