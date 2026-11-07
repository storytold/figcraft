//! The glass effect: the backdrop seen through a pane whose edges curve down like a lens.
//!
//! Within `depth` of the outline the surface curves like a thick lens: the backdrop there is bent
//! towards the inside, most a [`BEVEL`] in from the edge (`refraction` × `depth` far), falling off
//! as the 3.5th power of the way out of the curve and gone `depth` further in, with a fifth as
//! much on the bevel itself ([`bend_at`]). `splay` turns which way it bends, from towards the
//! shape's core (0; along the edge's normal for a shape too big to keep, see [`MAX_SHAPE_PX`]) to
//! away from the centre of the layer's bounds (1), as the reference tool's: on a ring or an L,
//! whose centre lies outside the shape, an inner edge bends towards the hole. Each colour channel
//! bends by a different amount (`dispersion`, a rainbow fringe: red furthest, blue least, each
//! smeared along the bend). The bevel catches the light as a thin line added to the backdrop
//! ([`rim_light_at`]): fully where the edge faces `light_angle`, none below 0.35 of facing it, 0.6
//! as much on the opposite side; and a faint gradient runs across the curve, 5% darker where it
//! faces the light and lighter opposite. `frost` blurs the backdrop first. Distances to the outline come from an exact Euclidean
//! distance transform of the shape's coverage.
//!
//! The shape's coverage, distances and normals depend only on the shape and the zoom. Once a
//! later render asks for the same shape at the same zoom they are worked out for the whole shape,
//! in its own pixel frame, and kept in a cache of at most [`MAX_CACHED_BYTES`]: moving or editing
//! what lies under the glass, dragging its sliders, panning and patching the canvas reuse them.
//! Zooming doesn't (each zoom is new). The bend directions towards the core depend on the depth
//! too: they are kept beside them for the last two depths asked for, so dragging the Depth slider
//! works out only those again.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use figcraft_doc::{Glass, Node, NodeKind};
use figcraft_geom::kurbo::Shape as _;
use figcraft_geom::{Affine, Point};
use tiny_skia::Pixmap;

use crate::backdrop::EffectFrame;
use crate::fx;
use crate::shadow::{Outline, spread_alpha};

/// Squared distance transform of one row or column (Felzenszwalb & Huttenlocher): `f` holds 0 at
/// sources and a large value elsewhere; `d` gets each sample's squared distance to the nearest
/// source, and `src`, when given, that source's index. `v` and `z` are scratch space (`f.len()` and
/// `f.len() + 1` long).
fn edt_1d(f: &[f64], d: &mut [f64], v: &mut [usize], z: &mut [f64], mut src: Option<&mut [usize]>) {
    let n = f.len();
    // Every index below stays under these lengths (k <= q < n), checked once here.
    if n == 0 || d.len() < n || v.len() < n || z.len() < n + 1 {
        return;
    }
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in 1..n {
        let qf = q as f64;
        let mut s;
        loop {
            let vk = v[k] as f64;
            s = ((f[q] + qf * qf) - (f[v[k]] + vk * vk)) / (2.0 * qf - 2.0 * vk);
            if s <= z[k] && k > 0 {
                k -= 1;
            } else {
                break;
            }
        }
        if s <= z[k] {
            // Only the first parabola is left and this one hides it.
            v[0] = q;
            z[1] = f64::INFINITY;
        } else {
            k += 1;
            v[k] = q;
            z[k] = s;
            z[k + 1] = f64::INFINITY;
        }
    }
    let mut k = 0usize;
    for q in 0..n {
        let qf = q as f64;
        while z[k + 1] < qf {
            k += 1;
        }
        let dv = qf - v[k] as f64;
        d[q] = dv * dv + f[v[k]];
        if let Some(o) = src.as_deref_mut().and_then(|s| s.get_mut(q)) {
            *o = v[k];
        }
    }
}

/// For a `w` × `h` coverage pixmap: each pixel's distance (pixels) from its centre to the edge of
/// the shape (pixels at least half covered), 0 outside.
pub fn inside_distance(cov: &Pixmap) -> Vec<f32> {
    let (w, h) = (cov.width() as usize, cov.height() as usize);
    const FAR: f64 = 1e20;
    let mut g: Vec<f64> = cov.data().chunks_exact(4).map(|p| if p.get(3).copied().unwrap_or(0) >= 128 { FAR } else { 0.0 }).collect();
    let n = w.max(h);
    let (mut f, mut d, mut v, mut z) = (vec![0f64; n], vec![0f64; n], vec![0usize; n], vec![0f64; n + 1]);
    for x in 0..w {
        for y in 0..h {
            if let (Some(fy), Some(gv)) = (f.get_mut(y), g.get(y * w + x)) {
                *fy = *gv;
            }
        }
        edt_1d(f.get(..h).unwrap_or(&[]), d.get_mut(..h).unwrap_or(&mut []), &mut v, &mut z, None);
        for y in 0..h {
            if let (Some(gv), Some(dy)) = (g.get_mut(y * w + x), d.get(y)) {
                *gv = *dy;
            }
        }
    }
    for y in 0..h {
        let row = y * w;
        if let (Some(src), Some(dst)) = (g.get(row..row + w), f.get_mut(..w)) {
            dst.copy_from_slice(src);
        }
        edt_1d(f.get(..w).unwrap_or(&[]), d.get_mut(..w).unwrap_or(&mut []), &mut v, &mut z, None);
        if let (Some(dst), Some(src)) = (g.get_mut(row..row + w), d.get(..w)) {
            dst.copy_from_slice(src);
        }
    }
    // Centre-to-centre distance to the nearest outside pixel → distance to the edge between.
    g.iter().map(|s| if *s > 0.0 { (s.sqrt() - 0.5).max(0.0) as f32 } else { 0.0 }).collect()
}

/// [`nearest_sites`]' answer for a pixel with no site.
const NO_SITE: u32 = u32::MAX;

/// For each pixel of a `w` × `h` grid, the nearest pixel (centre to centre) where `site` is true, as
/// its index, or [`NO_SITE`] when there is none: the exact Euclidean feature transform (the distance
/// transform above, keeping where each distance comes from). 16 bytes a pixel at most.
fn nearest_sites(site: &[bool], w: usize, h: usize) -> Vec<u32> {
    const FAR: f64 = 1e20;
    let n = w.max(h);
    let (mut f, mut d, mut v, mut z, mut src) = (vec![0f64; n], vec![0f64; n], vec![0usize; n], vec![0f64; n + 1], vec![0usize; n]);
    // Down each column: the squared distance to its nearest site, and that site's row.
    let (mut col_d, mut col_row) = (vec![FAR; w * h], vec![NO_SITE; w * h]);
    for x in 0..w {
        for y in 0..h {
            if let Some(fy) = f.get_mut(y) {
                *fy = if site.get(y * w + x).copied().unwrap_or(false) { 0.0 } else { FAR };
            }
        }
        edt_1d(f.get(..h).unwrap_or(&[]), d.get_mut(..h).unwrap_or(&mut []), &mut v, &mut z, src.get_mut(..h));
        for y in 0..h {
            if let (Some(cd), Some(cr), Some(dy), Some(sy)) = (col_d.get_mut(y * w + x), col_row.get_mut(y * w + x), d.get(y), src.get(y)) {
                (*cd, *cr) = (*dy, u32::try_from(*sy).unwrap_or(NO_SITE));
            }
        }
    }
    // Along each row: the nearest of those, by the column it is in.
    let mut out = vec![NO_SITE; w * h];
    for y in 0..h {
        let row = y * w;
        if let (Some(dst), Some(from)) = (f.get_mut(..w), col_d.get(row..row + w)) {
            dst.copy_from_slice(from);
        }
        edt_1d(f.get(..w).unwrap_or(&[]), d.get_mut(..w).unwrap_or(&mut []), &mut v, &mut z, src.get_mut(..w));
        for x in 0..w {
            if let (Some(dx), Some(sx)) = (d.get(x), src.get(x))
                && *dx < FAR / 2.0
                && let (Some(o), Some(&sy)) = (out.get_mut(row + x), col_row.get(row + *sx))
                && sy != NO_SITE
            {
                *o = u32::try_from(sy as usize * w + sx).unwrap_or(NO_SITE);
            }
        }
    }
    out
}

/// The largest of `src` (`w` × `h`) within `r` pixels across and down of each pixel (a square
/// window): running maxima along the rows, then the columns.
fn max_within(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let (mut rows, mut out) = (vec![0f32; w * h], vec![0f32; w * h]);
    max_pass(src, &mut rows, w, h, r, true);
    max_pass(&rows, &mut out, w, h, r, false);
    out
}

/// One pass of [`max_within`] along rows (`rows`) or columns: a queue holds the window's candidates
/// for its largest, falling from the front, so each value enters and leaves it once.
fn max_pass(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize, rows: bool) {
    let (lines, len, step, across) = if rows { (h, w, 1, w) } else { (w, h, w, 1) };
    let mut queue = std::collections::VecDeque::new();
    for l in 0..lines {
        let base = l * across;
        let at = |i: usize| src.get(base + i * step).copied().unwrap_or(0.0);
        queue.clear();
        let mut next = 0;
        for i in 0..len {
            while next < len && next <= i + r {
                let v = at(next);
                while queue.back().is_some_and(|&b| at(b) <= v) {
                    queue.pop_back();
                }
                queue.push_back(next);
                next += 1;
            }
            while queue.front().is_some_and(|&f| f + r < i) {
                queue.pop_front();
            }
            if let (Some(o), Some(&f)) = (dst.get_mut(base + i * step), queue.front()) {
                *o = at(f);
            }
        }
    }
}

/// How far around (pixels) a pixel counts as the middle of a thin part: no deeper than the pixels
/// within this many across and down, give or take [`RIDGE_SLACK`].
const RIDGE_PX: usize = 2;

/// How much shallower than the deepest pixel within [`RIDGE_PX`] a pixel can be and still count as
/// a thin part's middle: half a pixel per pixel, so a gently tapering stroke keeps its middle while
/// the diagonal running into a corner (depth rising by 0.7 px per pixel, more in the window) and
/// the band along an edge (1 px per pixel) do not.
const RIDGE_SLACK: f32 = 0.5 * RIDGE_PX as f32;

/// Unit inward directions from each pixel of a shape (`dist`, its inside distance field, `w` × `h`)
/// to the nearest point of its core, which the glass bends the backdrop towards: where the shape
/// is `reach` pixels deep or more, and the middle of each part thinner than that (a capsule, a
/// letter's stroke: pixels about as deep as any within [`RIDGE_PX`]). Bending towards the core
/// rather than along the edge's normal rounds a sharp corner's bend (no seam along its diagonal)
/// and makes a thin part bend across itself, each part towards its own middle. Box-blurred once
/// over `r` pixels (the normals twice: these are smooth already, but turn round at a thin part's
/// middle); zero on the core and outside.
fn bend_directions(dist: &[f32], w: usize, h: usize, reach: f32, r: usize) -> Vec<(f32, f32)> {
    // The maxima and the core are dropped before the directions are made, keeping the peak down.
    let near = {
        let around = max_within(dist, w, h, RIDGE_PX);
        let core: Vec<bool> = dist.iter().zip(&around).map(|(d, m)| *d > 0.0 && (*d >= reach - 1e-3 || *d >= m - RIDGE_SLACK - 1e-3)).collect();
        nearest_sites(&core, w, h)
    };
    let (mut gx, mut gy) = (vec![0f32; w * h], vec![0f32; w * h]);
    for (i, &s) in near.iter().enumerate() {
        if s != NO_SITE
            && let Some(d) = dist.get(i)
            && *d > 0.0
            && let (Some(a), Some(b)) = (gx.get_mut(i), gy.get_mut(i))
        {
            let s = s as usize;
            let (dx, dy) = ((s % w) as f32 - (i % w) as f32, (s / w) as f32 - (i / w) as f32);
            let len = (dx * dx + dy * dy).sqrt();
            if len > 1e-6 {
                (*a, *b) = (dx / len, dy / len);
            }
        }
    }
    smoothed_units(gx, gy, w, h, r, 1)
}

/// Running-sum box blur of a `w` × `h` field along rows (`stride` 1) or columns (`stride` `w`),
/// radius `r`, zero outside.
fn box_pass(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize, rows: bool) {
    let (lines, len, step, across) = if rows { (h, w, 1, w) } else { (w, h, w, 1) };
    let norm = 1.0 / (2 * r + 1) as f32;
    for l in 0..lines {
        let base = l * across;
        let at = |i: usize| src.get(base + i * step).copied().unwrap_or(0.0);
        let mut acc: f32 = (0..=r.min(len.saturating_sub(1))).map(at).sum();
        for i in 0..len {
            if let Some(o) = dst.get_mut(base + i * step) {
                *o = acc * norm;
            }
            if i + r + 1 < len {
                acc += at(i + r + 1);
            }
            if i >= r {
                acc -= at(i - r);
            }
        }
    }
}

/// Unit inward normals of the shape from its inside `dist`ance field (`w` × `h`): the distance's
/// gradient, smoothed over `r` pixels so curved edges bend the backdrop smoothly instead of in the
/// steps of the pixel grid. Zero where there is no direction (outside, or a flat middle).
fn smooth_normals(dist: &[f32], w: usize, h: usize, r: usize) -> Vec<(f32, f32)> {
    let at = |x: usize, y: usize| dist.get(y * w + x).copied().unwrap_or(0.0);
    let mut gx = vec![0f32; w * h];
    let mut gy = vec![0f32; w * h];
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            if let (Some(a), Some(b)) = (gx.get_mut(y * w + x), gy.get_mut(y * w + x)) {
                *a = (at(x + 1, y) - at(x - 1, y)) * 0.5;
                *b = (at(x, y + 1) - at(x, y - 1)) * 0.5;
            }
        }
    }
    smoothed_units(gx, gy, w, h, r, 2)
}

/// The vectors (`gx`, `gy`) of a `w` × `h` field box-blurred `times` over `r` pixels and made unit
/// length; zero where they cancel out (no direction).
fn smoothed_units(mut gx: Vec<f32>, mut gy: Vec<f32>, w: usize, h: usize, r: usize, times: usize) -> Vec<(f32, f32)> {
    let mut tmp = vec![0f32; w * h];
    for g in [&mut gx, &mut gy] {
        for _ in 0..times {
            box_pass(g, &mut tmp, w, h, r, true);
            box_pass(&tmp, g, w, h, r, false);
        }
    }
    gx.iter()
        .zip(&gy)
        .map(|(x, y)| {
            let len = (x * x + y * y).sqrt();
            if len > 1e-4 { (x / len, y / len) } else { (0.0, 0.0) }
        })
        .collect()
}

/// The bevel along a glass's edge (document units): where the bend eases off before the edge.
pub(crate) const BEVEL: f64 = 1.0;

/// How sharply the bend falls off going in: the 3.5th power of the way out of the curve.
const PROFILE_EXP: f32 = 3.5;

/// The furthest the backdrop moves, as a share of `depth`.
const MAX_BEND: f32 = 0.999;

/// The bend on the bevel itself, as a share of [`MAX_BEND`] × `depth` × `refraction`.
const RIM_BEND: f32 = 0.2;

/// Steps of the dispersion window: each channel is sampled this many times along the bend, weighted
/// by a triangle (the two end steps weigh nothing and are skipped).
const DISPERSION_STEPS: usize = 8;

/// The least an edge faces the light (the cosine between its outward normal and the light) and
/// still catches some.
const HL_LO: f32 = 0.35;

/// How much of the light the side opposite it catches.
const HL_SECONDARY: f32 = 0.6;

/// The gradient across the curve at its strongest, a share of the light's intensity.
const BROAD: f32 = 0.05;

/// `x` from `a` to `b` as 0 to 1, clamped (a span under 1e-5 counts as 1e-5).
fn lin(a: f32, b: f32, x: f32) -> f32 {
    ((x - a) / (b - a).max(1e-5)).clamp(0.0, 1.0)
}

/// `x` from `a` to `b` as a smooth 0-to-1 step (Hermite), clamped.
fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a).max(1e-5)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much of the rim light falls `x` pixels in from the edge, for a `bevel` pixels deep: all
/// of it within a fifth of the bevel, none a bevel in.
pub(crate) fn rim_light_at(x: f32, bevel: f32) -> f32 {
    smoothstep(-bevel - 0.001, -0.2 * bevel, -x)
}

/// How much light an edge catches, `facing` being the cosine between its outward normal and the
/// light: from [`HL_LO`] to fully facing it, and [`HL_SECONDARY`] as much facing away.
fn catches(facing: f32) -> f32 {
    smoothstep(HL_LO, 1.0, facing) + HL_SECONDARY * smoothstep(HL_LO, 1.0, -facing)
}

/// How strongly the gradient across the curve shows `x` pixels in from the edge (`depth` and
/// `bevel` in pixels): fully on the bevel, falling as the square of the way in to none at `depth`.
pub(crate) fn sheen_at(x: f32, depth: f32, bevel: f32) -> f32 {
    lin(-depth, -bevel + 1e-4, -x).powi(2)
}

/// How far the backdrop moves at `x` pixels in from the edge, for a glass `depth` and [`BEVEL`]
/// `bevel` deep (pixels) and `refraction` (0–1): a thick lens's profile, the most a bevel in from
/// the edge, falling off as the 3.5th power of the way out of the curve to nothing `depth` further
/// in, and a fifth as much on the bevel itself, eased between the two over its outer 70%.
pub(crate) fn bend_at(x: f32, depth: f32, bevel: f32, refraction: f32) -> f32 {
    // Past the curve (and the bevel's easing) nothing bends: the flat inside of the pane.
    if x >= depth.max(1e-4) + bevel {
        return 0.0;
    }
    let sd = -x;
    let t = lin(-depth - bevel, -bevel, sd);
    let most = MAX_BEND * depth;
    let core = (t.powf(PROFILE_EXP) * depth * refraction).clamp(-most, most);
    let rim = refraction * RIM_BEND * most;
    let u = lin(-bevel - 1e-4, -0.3 * bevel, sd);
    core + (rim - core) * u
}

/// Below this split (pixels: the move's length × `k`) the dispersion window takes one sample: its
/// steps lie within 1/256 px of the moved point, so one sample is within one level (of 255) of the
/// window even across the hardest edge, from black to white or opaque to clear in a pixel.
const ONE_SAMPLE_SPLIT_PX: f32 = 1.0 / 256.0;

/// The backdrop seen at pixel coordinates (`x`, `y`) moved by (`dx`, `dy`), its channels split by
/// `k` (dispersion, 0–1): red moves up to `k` further, blue up to `k` less, each smeared along the
/// move by a triangular window of [`DISPERSION_STEPS`]. One sample when the split is too small to
/// show: `k` under 0.001, or the channels under [`ONE_SAMPLE_SPLIT_PX`] apart (where the curve
/// flattens out and the move is short).
fn dispersed(frosted: &Pixmap, x: f32, y: f32, dx: f32, dy: f32, k: f32) -> [f32; 4] {
    if k < 0.001 || (dx * dx + dy * dy).sqrt() * k < ONE_SAMPLE_SPLIT_PX {
        let s = sample(frosted, x + dx, y + dy);
        return [s[0].min(s[3]), s[1].min(s[3]), s[2].min(s[3]), s[3]];
    }
    dispersion_window(frosted, x, y, dx, dy, k)
}

/// [`dispersed`]'s triangular window, every step sampled.
fn dispersion_window(frosted: &Pixmap, x: f32, y: f32, dx: f32, dy: f32, k: f32) -> [f32; 4] {
    let at = |s: f32| sample(frosted, x + dx * s, y + dy * s);
    let last = (DISPERSION_STEPS - 1) as f32;
    let (mut acc, mut sum) = ([0.0f32; 4], 0.0f32);
    for i in 1..DISPERSION_STEPS - 1 {
        let t = i as f32 / last - 0.5;
        let w = 1.0 - (2.0 * t).abs();
        let (r, g, b) = (at(1.0 + (t + 0.5) * k), at(1.0 + t * k), at(1.0 + (t - 0.5) * k));
        acc[0] += r[0] * w;
        acc[1] += g[1] * w;
        acc[2] += b[2] * w;
        acc[3] += r[3].max(g[3]).max(b[3]) * w;
        sum += w;
    }
    let a = acc[3] / sum;
    [(acc[0] / sum).min(a), (acc[1] / sum).min(a), (acc[2] / sum).min(a), a]
}

/// `base`, a premultiplied backdrop pixel (channels 0–255), with `add` (levels, below 0 darkens)
/// added to its colour, at coverage `c`. Where that is more than the backdrop's alpha holds (all of
/// it over a transparent backdrop, an export with nothing under the glass), the alpha rises with
/// it, so the light shows there too.
fn lit(base: [f32; 4], add: f32, c: f32) -> [u8; 4] {
    let [r, g, b, a] = base;
    let (r, g, b) = (r + add, g + add, b + add);
    let a = a.max(0.0).max(r).max(g).max(b).min(255.0);
    [r, g, b, a].map(|v| (v.clamp(0.0, a) * c).round().clamp(0.0, 255.0) as u8)
}

/// Bilinear sample of premultiplied `p` at pixel coordinates `(x, y)` (centres at +0.5), each
/// channel 0..=255; transparent outside.
fn sample(p: &Pixmap, x: f32, y: f32) -> [f32; 4] {
    let (w, h) = (p.width() as i64, p.height() as i64);
    let (x, y) = (x - 0.5, y - 0.5);
    let (x0, y0) = (x.floor(), y.floor());
    let (tx, ty) = (x - x0, y - y0);
    let (x0, y0) = (x0 as i64, y0 as i64);
    let data = p.data();
    let px = |xi: i64, yi: i64| -> [f32; 4] {
        if xi < 0 || yi < 0 || xi >= w || yi >= h {
            return [0.0; 4];
        }
        let i = ((yi * w + xi) * 4) as usize;
        match data.get(i..i + 4) {
            Some(s) => [f32::from(s[0]), f32::from(s[1]), f32::from(s[2]), f32::from(s[3])],
            None => [0.0; 4],
        }
    };
    let (a, b, c, d) = (px(x0, y0), px(x0 + 1, y0), px(x0, y0 + 1), px(x0 + 1, y0 + 1));
    std::array::from_fn(|k| {
        let top = a[k] + (b[k] - a[k]) * tx;
        let bot = c[k] + (d[k] - c[k]) * tx;
        top + (bot - top) * ty
    })
}

/// A shape's coverage (alpha), distance to its edge and smoothed inward normals (which the light
/// reads) over a window of pixels whose top-left pixel is (`x0`, `y0`): what the shape and the zoom
/// alone decide, so it is kept whatever the depth ([`whole_shape`]).
struct EdgeFields {
    x0: i64,
    y0: i64,
    w: usize,
    h: usize,
    cov: Vec<u8>,
    dist: Vec<f32>,
    normals: Vec<(f32, f32)>,
    /// How far the normals and the bend directions are smoothed (pixels).
    r: usize,
    /// The bend directions for the last [`KEPT_BENDS`] core depths asked for, by their bits, the
    /// most recent last ([`EdgeFields::bends()`]).
    bends: Mutex<Vec<(u32, Bends)>>,
}

/// How many depths' bend directions a kept shape holds: two glass layers of the same shape with
/// different depths don't take turns working theirs out.
const KEPT_BENDS: usize = 2;

/// Inward bend directions ([`bend_directions`]) over an [`EdgeFields`] window.
type Bends = Arc<Vec<(f32, f32)>>;

/// The fields `glass` reads: a shape's [`EdgeFields`] and its bend directions for one depth.
struct Fields {
    edge: Arc<EdgeFields>,
    /// `None` for a shape too big to work out a core ([`fits_whole`]): it bends along its normals.
    bends: Option<Bends>,
}

/// Bytes a field pixel holds: coverage, distance, a normal and [`KEPT_BENDS`] bend directions.
const FIELD_PX_BYTES: u64 = 1 + 4 + 8 + 8 * KEPT_BENDS as u64;

/// A pixel of [`Fields`]: coverage (0–1), distance to the edge, inward normal, inward bend direction.
type FieldPx = (f32, f32, (f32, f32), (f32, f32));

impl EdgeFields {
    /// The fields of `cov`, the shape's coverage on the window at (`x0`, `y0`), normals smoothed
    /// over `r` pixels.
    fn new(cov: &Pixmap, x0: i64, y0: i64, r: usize) -> EdgeFields {
        FIELDS_COMPUTED.with(|n| n.set(n.get() + 1));
        let dist = inside_distance(cov);
        let (w, h) = (cov.width() as usize, cov.height() as usize);
        let normals = smooth_normals(&dist, w, h, r);
        let cov = cov.data().chunks_exact(4).map(|p| p.get(3).copied().unwrap_or(0)).collect();
        EdgeFields { x0, y0, w, h, cov, dist, normals, r, bends: Mutex::new(Vec::new()) }
    }

    /// The bend directions towards the core `reach` pixels in: kept from a recent render at the
    /// same depth, else worked out and kept in place of the least recent (bands of one render
    /// asking at once wait for the first).
    fn bends(&self, reach: f32) -> Bends {
        let bits = reach.to_bits();
        let mut kept = self.bends.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(i) = kept.iter().position(|(b, _)| *b == bits) {
            let hit = kept.remove(i);
            let bends = hit.1.clone();
            kept.push(hit);
            return bends;
        }
        BENDS_COMPUTED.with(|n| n.set(n.get() + 1));
        let bends = Arc::new(bend_directions(&self.dist, self.w, self.h, reach, self.r));
        if kept.len() >= KEPT_BENDS {
            kept.remove(0);
        }
        kept.push((bits, bends.clone()));
        bends
    }
}

impl Fields {
    /// The fields of `cov`, the shape's coverage on the window at (`x0`, `y0`), normals and bend
    /// directions smoothed over `r` pixels, bending towards the core `reach` pixels in (along the
    /// normals without a `reach`).
    fn new(cov: &Pixmap, x0: i64, y0: i64, r: usize, reach: Option<f32>) -> Fields {
        Fields::of(Arc::new(EdgeFields::new(cov, x0, y0, r)), reach)
    }

    /// `edge`'s fields bending towards the core `reach` pixels in (along the normals without one).
    fn of(edge: Arc<EdgeFields>, reach: Option<f32>) -> Fields {
        let bends = reach.map(|reach| edge.bends(reach));
        Fields { edge, bends }
    }

    /// The fields at pixel (`x`, `y`); `None` outside the window.
    fn at(&self, x: i64, y: i64) -> Option<FieldPx> {
        let e = &*self.edge;
        let (fx, fy) = (x.checked_sub(e.x0)?, y.checked_sub(e.y0)?);
        if fx < 0 || fy < 0 || fx >= e.w as i64 || fy >= e.h as i64 {
            return None;
        }
        let i = fy as usize * e.w + fx as usize;
        let normal = *e.normals.get(i)?;
        let bend = match &self.bends {
            Some(bends) => *bends.get(i)?,
            None => normal,
        };
        Some((f32::from(*e.cov.get(i)?) / 255.0, *e.dist.get(i)?, normal, bend))
    }
}

thread_local! {
    static FIELDS_COMPUTED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static BENDS_COMPUTED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many times this thread has worked out a glass shape's fields: what a cache miss costs. For
/// tests and benchmarks.
pub fn glass_fields_computed() -> u64 {
    FIELDS_COMPUTED.with(std::cell::Cell::get)
}

/// How many times this thread has worked out a glass shape's bend directions towards its core: a
/// new shape, or a kept one at a new depth. For tests and benchmarks.
pub fn glass_bends_computed() -> u64 {
    BENDS_COMPUTED.with(std::cell::Cell::get)
}

/// What a glass shape's [`EdgeFields`] depend on, with the transform's translation reduced to its
/// fraction of a pixel: the same shape at the same zoom has the same key wherever it is and however
/// a render cuts it. Not the depth (beyond the margin and smoothing): the bend directions that
/// depend on it are worked out from the kept fields ([`EdgeFields::bends()`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ShapeKey {
    path: u64,
    linear: [u64; 4],
    frac: (i64, i64),
    margin: u32,
    r: usize,
}

/// The cached fields of a shape whose window holds at most this many pixels: 4M on the desktop, 1M
/// on the web. Bigger shapes (a pane far larger than the view) are worked out for the part a render
/// shows each time, with no core: they bend along their edge normals. So a large glass text or
/// vector can bend differently once zoomed into, in a 2× export than on the canvas, and on the web
/// than on the desktop. A rounded rectangle works out its core in closed form at any size.
#[cfg(not(target_arch = "wasm32"))]
const MAX_SHAPE_PX: u64 = 4 << 20;
#[cfg(target_arch = "wasm32")]
const MAX_SHAPE_PX: u64 = 1 << 20;
/// Bytes the glass shape cache holds at most (less on the web, where memory never shrinks).
#[cfg(not(target_arch = "wasm32"))]
const MAX_CACHED_BYTES: u64 = 128 << 20;
#[cfg(target_arch = "wasm32")]
const MAX_CACHED_BYTES: u64 = 32 << 20;

/// A shape's fields once worked out; parallel bands wanting the same shape wait for one of them.
type Slot = Arc<OnceLock<Option<Arc<EdgeFields>>>>;

/// The shapes' fields by key, at most `budget` bytes; the least recently used go first. A shape is
/// kept from the second render that asks for it: one asked for once (each step of a zoom, a pane
/// dragged by fractions of a pixel) is cheaper worked out for just the part a render shows.
struct ShapeCache {
    map: HashMap<ShapeKey, (Slot, u64, u64)>,
    /// Shapes asked for but not kept, by the render that first asked.
    seen: HashMap<ShapeKey, u64>,
    held: u64,
    tick: u64,
    budget: u64,
}

/// Shapes remembered as asked for once; past this many they are forgotten.
const MAX_SEEN: usize = 256;

impl ShapeCache {
    fn new(budget: u64) -> Self {
        Self { map: HashMap::new(), seen: HashMap::new(), held: 0, tick: 0, budget }
    }

    /// The slot for `key` (filled or about to be) asked for in render number `render`, making room
    /// for `bytes`; `None` when it isn't kept (first asked for in this render, or too big).
    fn slot(&mut self, key: &ShapeKey, bytes: u64, render: u64) -> Option<Slot> {
        self.tick += 1;
        if let Some((slot, _, used)) = self.map.get_mut(key) {
            *used = self.tick;
            return Some(slot.clone());
        }
        if bytes > self.budget {
            return None;
        }
        match self.seen.get(key) {
            Some(first) if *first < render => {
                self.seen.remove(key);
            }
            Some(_) => return None,
            None => {
                if self.seen.len() >= MAX_SEEN {
                    self.seen.clear();
                }
                self.seen.insert(key.clone(), render);
                return None;
            }
        }
        while self.held + bytes > self.budget {
            let oldest = self.map.iter().min_by_key(|(_, (_, _, used))| *used).map(|(k, _)| k.clone())?;
            if let Some((_, b, _)) = self.map.remove(&oldest) {
                self.held -= b;
            }
        }
        let slot = Slot::default();
        self.map.insert(key.clone(), (slot.clone(), bytes, self.tick));
        self.held += bytes;
        Some(slot)
    }
}

fn shape_cache() -> &'static Mutex<ShapeCache> {
    static C: OnceLock<Mutex<ShapeCache>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(ShapeCache::new(MAX_CACHED_BYTES)))
}

/// Whether `shape` (through `xf`) fits [`MAX_SHAPE_PX`] whole with a margin `m` round it: the
/// window a kept shape's fields cover ([`whole_shape`]). Depends on the shape and the zoom alone, so
/// every band and patch of a render answers the same.
fn fits_whole(shape: &Outline, xf: Affine, m: u32) -> bool {
    let Some(own) = own_frame(xf) else { return false };
    let bb = (own.xf * shape.0.clone()).bounding_box();
    let margin = f64::from(m.div_ceil(32).saturating_mul(32));
    let (w, h) = (bb.x1.ceil() - bb.x0.floor() + 2.0 + 2.0 * margin, bb.y1.ceil() - bb.y0.floor() + 2.0 + 2.0 * margin);
    let px = w * h;
    px.is_finite() && px <= MAX_SHAPE_PX as f64
}

/// The fields `glass` reads for a `w` × `h` layer whose `shape` maps through `xf` (local → layer
/// pixels), exact for distances up to the margin `m`, normals and bend directions smoothed over
/// `r` pixels, bending towards the core `reach` pixels in, and the shift from layer pixels to the
/// fields' pixels.
///
/// A shape asked for again in a later render, whose window fits [`MAX_SHAPE_PX`], is worked out
/// whole, in its own frame (the translation's fraction of a pixel), and kept: a band, a canvas
/// patch or the next frame of a drag reads the same fields. Otherwise it is worked out for the part
/// shown plus the margin.
fn shape_fields(shape: &Outline, xf: Affine, w: u32, h: u32, m: u32, r: usize, reach: Option<f32>) -> Option<(Fields, (i64, i64))> {
    if let Some((edge, shift)) = whole_shape(shape, xf, m, r) {
        return Some((Fields::of(edge, reach), shift));
    }
    let cov = spread_alpha(std::slice::from_ref(shape), 0.0, Affine::translate((f64::from(m), f64::from(m))) * xf, w + 2 * m, h + 2 * m)?;
    Some((Fields::new(&cov, -i64::from(m), -i64::from(m), r, reach), (0, 0)))
}

/// A layer transform split so the same shape at the same zoom lands on the same pixels of its own
/// frame wherever a render puts it or cuts it (see [`own_frame`]).
struct OwnFrame {
    /// The whole pixels of the translation: the shift from layer pixels to the shape's own frame.
    shift: (i64, i64),
    /// The translation's fraction of a pixel, in steps of 1/65536.
    frac: (i64, i64),
    /// Local coordinates → the shape's own frame: the linear part and that fraction.
    xf: Affine,
}

/// `xf` (local → layer pixels) split into its [`OwnFrame`]; `None` for a translation too far out.
fn own_frame(xf: Affine) -> Option<OwnFrame> {
    let [a, b, c, d, e, f] = xf.as_coeffs();
    // The translation in steps of 1/65536 px (far finer than the rasteriser's 1/64, coarse enough
    // that rounding errors in where a render starts land on the same step), its whole pixels (the
    // shift) and the fraction left; rounding first carries 99.9999999 over to 100.
    const STEPS: f64 = 65536.0;
    let q = |v: f64| {
        let q = (v * STEPS).round();
        (q.is_finite() && q.abs() < 1e15).then_some(q as i64)
    };
    let (qe, qf) = (q(e)?, q(f)?);
    let shift = (qe.div_euclid(STEPS as i64), qf.div_euclid(STEPS as i64));
    let frac = (qe.rem_euclid(STEPS as i64), qf.rem_euclid(STEPS as i64));
    Some(OwnFrame { shift, frac, xf: Affine::new([a, b, c, d, frac.0 as f64 / STEPS, frac.1 as f64 / STEPS]) })
}

/// [`shape_fields`]' [`EdgeFields`] for a whole shape, from the cache; `None` when it isn't kept
/// (first asked for in this render, too big, or its transform too far out).
fn whole_shape(shape: &Outline, xf: Affine, m: u32, r: usize) -> Option<(Arc<EdgeFields>, (i64, i64))> {
    let [a, b, c, d, _, _] = xf.as_coeffs();
    let OwnFrame { shift, frac, xf: own } = own_frame(xf)?;
    let bb = (own * shape.0.clone()).bounding_box();
    let (x0, y0, x1, y1) = (bb.x0.floor(), bb.y0.floor(), bb.x1.ceil(), bb.y1.ceil());
    if ![x0, y0, x1, y1].iter().all(|v| v.is_finite() && v.abs() < 1e9) {
        return None;
    }
    // The margin in steps of 32 px, so dragging a slider that widens it a little keeps the same
    // fields (a wider margin only adds exact distances further out: the same pixels).
    let margin = m.div_ceil(32).checked_mul(32)?;
    let (rw, rh) = ((x1 - x0) as u64 + 2, (y1 - y0) as u64 + 2);
    let (ww, wh) = (rw + 2 * u64::from(margin), rh + 2 * u64::from(margin));
    if ww.checked_mul(wh)? > MAX_SHAPE_PX {
        return None;
    }
    let mut hasher = DefaultHasher::new();
    crate::draw::hash_path(&shape.0, &mut hasher);
    matches!(shape.1, tiny_skia::FillRule::EvenOdd).hash(&mut hasher);
    let key = ShapeKey { path: hasher.finish(), linear: [a, b, c, d].map(f64::to_bits), frac, margin, r };
    let compute = || {
        // Rastered against the shape's own bounds (not the window), so the margin can't move a
        // coverage value.
        let raster = spread_alpha(std::slice::from_ref(shape), 0.0, Affine::translate((1.0 - x0, 1.0 - y0)) * own, rw as u32, rh as u32)?;
        let mut win = Pixmap::new(ww as u32, wh as u32)?;
        let (row, at) = (raster.width() as usize * 4, (margin as usize, margin as usize));
        let stride = win.width() as usize * 4;
        for (y, src) in raster.data().chunks_exact(row).enumerate() {
            let start = (y + at.1) * stride + at.0 * 4;
            win.data_mut().get_mut(start..start + row)?.copy_from_slice(src);
        }
        let origin = (x0 as i64 - 1 - i64::from(margin), y0 as i64 - 1 - i64::from(margin));
        Some(EdgeFields::new(&win, origin.0, origin.1, r))
    };
    let slot =
        shape_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner).slot(&key, ww * wh * FIELD_PX_BYTES, crate::render_number())?;
    Some((slot.get_or_init(|| compute().map(Arc::new)).clone()?, shift))
}

/// A layer whose glass shape is a rectangle with circular corners: its size and corner radii
/// (top-left, top-right, bottom-right, bottom-left), as [`figcraft_geom::rounded_rect`] draws them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoundedRect {
    pub width: f64,
    pub height: f64,
    pub radii: [f64; 4],
}

impl RoundedRect {
    /// The rounded rectangle `n`'s glass is the shape of, when its outline is one: rectangles,
    /// frames, components, instances and sections whose corners aren't smoothed.
    pub fn of(n: &Node) -> Option<RoundedRect> {
        let rect_like = matches!(
            n.kind,
            NodeKind::Rectangle
                | NodeKind::Frame
                | NodeKind::Component { .. }
                | NodeKind::ComponentSet { .. }
                | NodeKind::Instance { .. }
                | NodeKind::Section { .. }
        );
        let sized = n.width.is_finite() && n.height.is_finite() && n.width > 0.0 && n.height > 0.0;
        (rect_like && sized && n.corners.smoothing == 0.0).then(|| {
            // Clamped as `rounded_rect` clamps them: each radius to half the shorter side.
            let most = n.width.min(n.height) / 2.0;
            let radii = n.corners.radii.map(|r| if r.is_finite() { r.clamp(0.0, most) } else { 0.0 });
            RoundedRect { width: n.width, height: n.height, radii }
        })
    }
}

/// Signed distance from `q` (relative to the centre) to a rectangle reaching `half` each way with
/// corner radius `r` (negative inside), and its outward gradient: the unit normal along the sides,
/// pointing away from the corner's centre round the corners.
fn rounded_rect_sd(q: (f64, f64), half: (f64, f64), r: f64) -> (f64, (f64, f64)) {
    let (sx, sy) = (if q.0 < 0.0 { -1.0 } else { 1.0 }, if q.1 < 0.0 { -1.0 } else { 1.0 });
    let k = (q.0.abs() - half.0 + r, q.1.abs() - half.1 + r);
    if k.0 > 0.0 && k.1 > 0.0 {
        let len = (k.0 * k.0 + k.1 * k.1).sqrt();
        (len - r, (sx * k.0 / len, sy * k.1 / len))
    } else if k.0 > k.1 {
        (k.0 - r, (sx, 0.0))
    } else {
        (k.1 - r, (0.0, sy))
    }
}

/// The radius of the corner of `radii` (top-left, top-right, bottom-right, bottom-left) whose
/// quarter `q` (relative to the centre, y down) lies in.
fn corner_of(radii: [f64; 4], q: (f64, f64)) -> f64 {
    let i = match (q.0 < 0.0, q.1 < 0.0) {
        (true, true) => 0,
        (false, true) => 1,
        (false, false) => 2,
        (true, false) => 3,
    };
    radii.get(i).copied().unwrap_or(0.0)
}

/// A rounded rectangle's glass fields worked out pixel by pixel, in closed form: no coverage to
/// rasterise, no distance transform, nothing to cache. The same fields as [`Fields`] (the bend
/// directions round the corners towards the rectangle shrunk by the core's depth, the middle line
/// of one thinner than twice that), exact rather than from pixels, and the same wherever a render
/// cuts it: each pixel is placed in the layer's own frame.
struct AnalyticRect {
    /// Layer pixels → the shape's own frame.
    shift: (i64, i64),
    /// The shape's own frame → local coordinates.
    inv: Affine,
    /// Local → device directions (the transform's linear part), and its scale.
    linear: [f64; 4],
    scale: f64,
    half: (f64, f64),
    radii: [f64; 4],
    /// The corners the light's normals round to: each radius at least the bevel (a sharp corner's
    /// bevel catches the light along its diagonal), at most the half side.
    bevels: [f64; 4],
    /// The corners the bend rounds to: each radius at least the core's depth, at most the half
    /// side ([`bend_directions`]).
    rounds: [f64; 4],
}

impl AnalyticRect {
    /// `rect`'s fields for a layer whose `xf` maps local coordinates to its pixels, with a `bevel`
    /// and bending towards the core `reach` pixels in; `None` when `xf` isn't a rotation, flip and
    /// even scale (skewed or stretched rectangles go through the distance transform) or its
    /// translation is too far out.
    fn new(rect: RoundedRect, xf: Affine, bevel: f64, reach: f64) -> Option<AnalyticRect> {
        let own = own_frame(xf)?;
        let [a, b, c, d, _, _] = own.xf.as_coeffs();
        let (sq_x, sq_y) = (a * a + b * b, c * c + d * d);
        let even = (sq_x - sq_y).abs() <= 1e-9 * sq_x && (a * c + b * d).abs() <= 1e-9 * sq_x;
        let scale = sq_x.sqrt();
        if !(even && scale.is_finite() && scale > 1e-9) {
            return None;
        }
        let half = (rect.width / 2.0, rect.height / 2.0);
        let at_least = |least: f64| rect.radii.map(|r| r.max(least / scale).min(half.0.min(half.1)));
        let (bevels, rounds) = (at_least(bevel), at_least(reach));
        Some(AnalyticRect { shift: own.shift, inv: own.xf.inverse(), linear: [a, b, c, d], scale, half, radii: rect.radii, bevels, rounds })
    }

    /// The fields at layer pixel (`x`, `y`): coverage (box-filtered from the signed distance),
    /// distance in, inward normal and inward bend direction.
    fn at(&self, x: i64, y: i64) -> FieldPx {
        let p = self.inv * Point::new((x - self.shift.0) as f64 + 0.5, (y - self.shift.1) as f64 + 0.5);
        let q = (p.x - self.half.0, p.y - self.half.1);
        let (sd, _) = rounded_rect_sd(q, self.half, corner_of(self.radii, q));
        let (_, normal) = rounded_rect_sd(q, self.half, corner_of(self.bevels, q));
        let (_, round) = rounded_rect_sd(q, self.half, corner_of(self.rounds, q));
        let [a, b, c, d] = self.linear;
        let inward = |n: (f64, f64)| (-((a * n.0 + c * n.1) / self.scale) as f32, -((b * n.0 + d * n.1) / self.scale) as f32);
        let sd = sd * self.scale;
        ((0.5 - sd).clamp(0.0, 1.0) as f32, (-sd).max(0.0) as f32, inward(normal), inward(round))
    }
}

/// Where a glass reads its fields: worked out in closed form for a rounded rectangle, from its
/// coverage otherwise.
enum Geometry {
    Rect(AnalyticRect),
    Fields(Fields, (i64, i64)),
}

impl Geometry {
    /// The fields at layer pixel (`x`, `y`); `None` where there are none.
    fn at(&self, x: i64, y: i64) -> Option<FieldPx> {
        match self {
            Geometry::Rect(r) => Some(r.at(x, y)),
            Geometry::Fields(f, shift) => f.at(x - shift.0, y - shift.1),
        }
    }
}

/// The glass of a `w` × `h` layer whose `shape` maps through `xf` (local → layer pixels), from
/// `back`, its backdrop padded by `pad` pixels each way, at `scale` device pixels per unit. `rect`:
/// the rounded rectangle the shape is, when it is one, so its fields are worked out in closed form.
pub fn glass(g: &Glass, frame: EffectFrame<'_>, rect: Option<RoundedRect>) -> Option<Pixmap> {
    let EffectFrame { shape, back, pad, size: (w, h), xf, scale } = frame;
    let g = g.sanitized();
    let depth = (g.depth * scale).clamp(1.0, 1024.0);
    let bevel = (BEVEL * scale).min(1024.0);
    let r = (depth * 0.1).clamp(1.0, 8.0) as usize;
    // The core the backdrop bends towards lies `reach` in from the edge ([`bend_directions`]).
    let reach = (depth + bevel) as f32;
    // Coverage with a margin, so wherever the layer is cut every field the pixels read is the
    // whole render's: the distances the bend and the gradient read (within `depth` + `bevel`; the
    // light lies on the bevel), the core they bend towards (its nearest pixel up to √2 × `reach`
    // further, round a sharp corner, so 1.5 × `reach`; its depths exact within `reach` and
    // [`RIDGE_PX`] of it), and those the normals and bends are smoothed from (2r + 1 further).
    let plain = (depth + bevel).ceil() as u32 + 4 * r as u32 + 4;
    let cored = plain + (1.5 * reach).ceil() as u32 + RIDGE_PX as u32;
    // A core takes a wider margin and more bytes a pixel, so only a shape whose whole window fits
    // [`MAX_SHAPE_PX`] works one out: a bigger one bends along its edge normals, so no render
    // allocates a core's fields over more pixels than a kept shape holds.
    let core = fits_whole(shape, xf, cored).then_some(reach);
    let m = if core.is_some() { cored } else { plain };
    let geometry = match rect.and_then(|rect| AnalyticRect::new(rect, xf, bevel, f64::from(reach))) {
        Some(analytic) => Geometry::Rect(analytic),
        None => {
            let (fields, shift) = shape_fields(shape, xf, w, h, m, r, core)?;
            Geometry::Fields(fields, shift)
        }
    };
    let mut frosted = back.clone();
    fx::blur(&mut frosted, g.frost / 2.0 * scale);
    let ang = g.light_angle.to_radians();
    let light = (ang.sin() as f32, -ang.cos() as f32);
    // The centre of the layer's bounds in its own frame (see `own_frame`), which `splay` bends away
    // from: the direction to it is the same wherever a render puts or cuts the layer.
    let local_centre = shape.0.bounding_box().center();
    let ((sx, sy), centre) = match own_frame(xf) {
        Some(own) => (own.shift, own.xf * local_centre),
        None => ((0, 0), xf * local_centre),
    };
    let (refraction, dispersion, bevel) = (g.refraction as f32, (g.dispersion * Glass::DISPERSION_SPREAD) as f32, bevel as f32);
    let (splay, intensity, depth) = (g.splay as f32, g.light_intensity as f32, depth as f32);
    let mut out = Pixmap::new(w, h)?;
    let dst = out.data_mut();
    for y in 0..h as usize {
        for x in 0..w as usize {
            let Some((c, d, inward, toward)) = geometry.at(x as i64, y as i64) else { continue };
            if c <= 0.0 {
                continue;
            }
            let (bx, by) = (x as f32 + pad as f32 + 0.5, y as f32 + pad as f32 + 0.5);
            let (base, add) = if d >= depth + bevel {
                // Past the curve nothing bends and no light falls (the rim light and the gradient
                // are 0 there): the channels' samples are at the same point, so one sample.
                (dispersed(&frosted, bx, by, 0.0, 0.0, 0.0), 0.0)
            } else {
                // The curve's outward direction: away from its core, turned by `splay` towards the
                // way away from the centre (not renormalised: where the two part, the bend is less).
                let away = if splay > 0.0 {
                    let px = ((x as i64 - sx) as f64 + 0.5 - centre.x) as f32;
                    let py = ((y as i64 - sy) as f64 + 0.5 - centre.y) as f32;
                    let len = (px * px + py * py).sqrt();
                    if len > 1e-6 { (px / len, py / len) } else { (0.0, 0.0) }
                } else {
                    (0.0, 0.0)
                };
                let out_dir = (-toward.0 * (1.0 - splay) + away.0 * splay, -toward.1 * (1.0 - splay) + away.1 * splay);
                // No refraction: nothing bends, so one sample (`dispersed` with no move).
                let bend = bend_at(d, depth, bevel, refraction);
                let k = if bend == 0.0 { 0.0 } else { dispersion };
                let base = dispersed(&frosted, bx, by, -out_dir.0 * bend, -out_dir.1 * bend, k);
                // The bevel catches the light as a line added to the backdrop, by how much the edge
                // faces it; across the curve a faint gradient, darker on the side facing it.
                let facing = -(inward.0 * light.0 + inward.1 * light.1);
                let rim = rim_light_at(d, bevel) * catches(facing);
                let sheen = -(out_dir.0 * light.0 + out_dir.1 * light.1) * sheen_at(d, depth, bevel) * BROAD;
                (base, 255.0 * intensity * (rim + sheen))
            };
            let i = (y * w as usize + x) * 4;
            if let Some(p) = dst.get_mut(i..i + 4) {
                p.copy_from_slice(&lit(base, add, c));
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bend at depth 20, refraction 0.8 and a 1-unit bevel, x units in from the edge: the
    /// reference profile's values.
    #[test]
    fn the_bend_follows_the_thick_lens_profile() {
        let table = [
            (0.10, 3.1968),
            (0.25, 3.1968),
            (0.50, 6.8543),
            (0.75, 11.4263),
            (1.00, 15.9982),
            (2.0, 13.3707),
            (4.0, 9.0591),
            (6.0, 5.8457),
            (8.0, 3.5426),
            (10.0, 1.9742),
            (16.0, 0.1250),
            (20.0, 0.0004),
            (21.0, 0.0),
            (40.0, 0.0),
        ];
        for (x, want) in table {
            let got = bend_at(x, 20.0, 1.0, 0.8);
            assert!((got - want).abs() < 1e-3, "x {x}: {got} vs {want}");
        }
        // No refraction, no bend; never more than 0.999 × depth, never outwards.
        assert_eq!(bend_at(0.5, 20.0, 1.0, 0.0), 0.0);
        for i in 0..400 {
            let x = i as f32 * 0.1;
            let b = bend_at(x, 20.0, 1.0, 1.0);
            assert!((0.0..=0.999 * 20.0).contains(&b), "x {x}: {b}");
        }
        // A depth thinner than the bevel (zoomed far in) still gives numbers.
        assert!(bend_at(0.5, 1.0, 8.0, 1.0).is_finite());
        // `Glass::max_bend`, which sizes the backdrop read around a pane, bounds the whole bend.
        for refraction in [0.3, 0.8, 1.0] {
            let most = (0..800).map(|i| bend_at(i as f32 * 0.05, 20.0, 1.0, refraction as f32)).fold(0.0f32, f32::max);
            let bound = Glass { refraction, depth: 20.0, dispersion: 0.0, ..Glass::default() }.max_bend();
            assert!(f64::from(most) <= bound + 1e-4, "refraction {refraction}: {most} past {bound}");
        }
    }

    /// Dispersion off is one sample at the moved point; on, the channels split along the move, red
    /// further and blue less, and a flat backdrop stays flat.
    #[test]
    fn dispersion_splits_the_channels_along_the_bend() {
        // A 40 × 1 ramp: red, green and blue all rise with x.
        let mut p = Pixmap::new(40, 1).unwrap();
        for (i, px) in p.data_mut().chunks_exact_mut(4).enumerate() {
            let v = (i * 6) as u8;
            px.copy_from_slice(&[v, v, v, 255]);
        }
        let one = sample(&p, 10.5 + 8.0, 0.5);
        let off = dispersed(&p, 10.5, 0.5, 8.0, 0.0, 0.0);
        assert_eq!(off, [one[0], one[1], one[2], one[3]]);
        let on = dispersed(&p, 10.5, 0.5, 8.0, 0.0, 1.0);
        assert!(on[0] > on[1] && on[1] > on[2], "{on:?}");
        assert!((on[1] - one[1]).abs() < 1e-3, "green stays centred on the move: {on:?} vs {one:?}");
        let mut flat = Pixmap::new(40, 1).unwrap();
        flat.fill(tiny_skia::Color::from_rgba8(90, 120, 200, 255));
        let f = dispersed(&flat, 10.5, 0.5, 8.0, 0.0, 1.0);
        assert!(f.iter().zip([90.0, 120.0, 200.0, 255.0]).all(|(a, b)| (a - b).abs() < 1e-3), "{f:?}");
        // One white pixel on black: only the weighted steps that reach it pick it up, red and green
        // from opposite ends of their windows (the same amount), blue none.
        let mut impulse = Pixmap::new(40, 1).unwrap();
        impulse.fill(tiny_skia::Color::BLACK);
        impulse.data_mut()[20 * 4..21 * 4].copy_from_slice(&[255, 255, 255, 255]);
        let spread = dispersed(&impulse, 10.5, 0.5, 8.0, 0.0, 1.0);
        assert!((spread[0] - 33.392857).abs() < 1e-3 && (spread[1] - 33.392857).abs() < 1e-3, "{spread:?}");
        assert_eq!(spread[2], 0.0);
    }

    /// Where the channels split under [`ONE_SAMPLE_SPLIT_PX`], one sample stands in for the window
    /// within one level, even over the hardest backdrops: 1 px columns of black and white, and of
    /// white and nothing.
    #[test]
    fn a_split_too_narrow_to_show_takes_one_sample() {
        for clear in [false, true] {
            let mut p = Pixmap::new(64, 4).unwrap();
            for (i, px) in p.data_mut().chunks_exact_mut(4).enumerate() {
                let on = i % 2 == 1;
                let v = if on { 255 } else { 0 };
                px.copy_from_slice(&[v, v, v, if clear { v } else { 255 }]);
            }
            let mut worst = 0.0f32;
            for step in 0..400 {
                let x = 20.0 + step as f32 / 100.0;
                // Along the columns' rows and across them, and out past the backdrop's last row.
                for (dx, dy) in [(2.0f32, 0.0f32), (-2.0, 0.0), (1.2, 1.6), (0.0, 2.0), (40.0, 0.0)] {
                    let k = ONE_SAMPLE_SPLIT_PX * 0.999 / (dx * dx + dy * dy).sqrt();
                    let one = dispersed(&p, x, 2.0, dx, dy, k);
                    assert_eq!(one, dispersed(&p, x, 2.0, dx, dy, 0.0), "one sample at {x}");
                    let window = dispersion_window(&p, x, 2.0, dx, dy, k);
                    worst = one.iter().zip(window).fold(worst, |m, (a, b)| m.max((a - b).abs()));
                }
            }
            assert!(worst <= 1.0, "clear {clear}: {worst}");
        }
        // A wider split still takes the window.
        let mut p = Pixmap::new(64, 4).unwrap();
        p.data_mut().chunks_exact_mut(4).step_by(2).for_each(|px| px.copy_from_slice(&[255; 4]));
        let k = ONE_SAMPLE_SPLIT_PX * 2.0 / 2.0;
        assert_eq!(dispersed(&p, 20.3, 2.0, 2.0, 0.0, k), dispersion_window(&p, 20.3, 2.0, 2.0, 0.0, k));
        assert_ne!(dispersed(&p, 20.3, 2.0, 2.0, 0.0, k), dispersed(&p, 20.3, 2.0, 2.0, 0.0, 0.0));
    }

    /// A `w` × `h` coverage with the rectangles `rects` (x, y, width, height) filled.
    fn coverage(w: u32, h: u32, rects: &[(f32, f32, f32, f32)]) -> Pixmap {
        let mut p = Pixmap::new(w, h).unwrap();
        let paint = tiny_skia::Paint { shader: tiny_skia::Shader::SolidColor(tiny_skia::Color::WHITE), ..Default::default() };
        for (x, y, rw, rh) in rects {
            p.fill_rect(tiny_skia::Rect::from_xywh(*x, *y, *rw, *rh).unwrap(), &paint, tiny_skia::Transform::identity(), None);
        }
        p
    }

    fn bend_dir(f: &Fields, x: i64, y: i64) -> (f32, f32) {
        f.at(x, y).unwrap().3
    }

    fn close(a: (f32, f32), b: (f32, f32), tol: f32) -> bool {
        (a.0 - b.0).abs() <= tol && (a.1 - b.1).abs() <= tol
    }

    #[test]
    fn a_sharp_corner_bends_round_towards_the_core() {
        // A 120 × 80 rectangle at (20, 20), bending towards its core 20 px in.
        let f = Fields::new(&coverage(160, 120, &[(20.0, 20.0, 120.0, 80.0)]), 0, 0, 1, Some(20.0));
        let s = std::f32::consts::FRAC_1_SQRT_2;
        // Halfway down the left side: straight in. In the corner: diagonally, towards the core's own
        // corner 20 px in.
        assert!(close(bend_dir(&f, 21, 60), (1.0, 0.0), 0.02), "{:?}", bend_dir(&f, 21, 60));
        assert!(close(bend_dir(&f, 22, 22), (s, s), 0.05), "{:?}", bend_dir(&f, 22, 22));
        // Mirror images across the corner's diagonal, with no seam along it: neighbours agree.
        let (a, b) = (bend_dir(&f, 24, 29), bend_dir(&f, 29, 24));
        assert!(close(a, (b.1, b.0), 1e-4), "{a:?} vs {b:?}");
        let (on, off) = (bend_dir(&f, 26, 26), bend_dir(&f, 27, 26));
        assert!(close(on, off, 0.1), "{on:?} vs {off:?}");
    }

    #[test]
    fn a_shape_too_big_to_keep_bends_along_its_normals() {
        // A small shape fits whole with a core's margin; one wider than the kept-shape limit
        // doesn't, at this zoom or with this margin, wherever it sits.
        let shape = |w: f64, h: f64| -> Outline { (figcraft_geom::kurbo::Rect::new(0.0, 0.0, w, h).to_path(0.1), tiny_skia::FillRule::Winding) };
        assert!(fits_whole(&shape(200.0, 120.0), Affine::translate((10.5, 3.0)), 64));
        assert!(!fits_whole(&shape(4000.0, 3000.0), Affine::translate((10.5, 3.0)), 64));
        assert!(!fits_whole(&shape(200.0, 120.0), Affine::scale(40.0), 64));
        // Without a core the fields hold no bend directions, and bend along the normals.
        let f = Fields::new(&coverage(160, 120, &[(20.0, 20.0, 120.0, 80.0)]), 0, 0, 1, None);
        assert!(f.bends.is_none());
        for (x, y) in [(21, 60), (24, 24), (80, 21)] {
            let (_, _, normal, bend) = f.at(x, y).unwrap();
            assert_eq!(normal, bend, "({x}, {y})");
        }
    }

    #[test]
    fn a_thin_part_bends_across_itself() {
        // A 200 × 12 bar, thinner than twice the reach: its core is its middle line, and each side
        // bends towards it.
        let f = Fields::new(&coverage(240, 40, &[(20.0, 14.0, 200.0, 12.0)]), 0, 0, 1, Some(20.0));
        assert!(close(bend_dir(&f, 120, 14), (0.0, 1.0), 0.02), "top: {:?}", bend_dir(&f, 120, 14));
        assert!(close(bend_dir(&f, 120, 25), (0.0, -1.0), 0.02), "bottom: {:?}", bend_dir(&f, 120, 25));
        assert!(f.bends.as_ref().unwrap().iter().all(|(x, y)| x.is_finite() && y.is_finite()));
    }

    #[test]
    fn neighbouring_strokes_bend_towards_their_own_middles() {
        // Two 8 px strokes 2 px apart, like letters: the facing edges bend away from each other,
        // each into its own stroke.
        let f = Fields::new(&coverage(60, 60, &[(10.0, 10.0, 8.0, 40.0), (20.0, 10.0, 8.0, 40.0)]), 0, 0, 1, Some(20.0));
        assert!(bend_dir(&f, 17, 30).0 < -0.9, "{:?}", bend_dir(&f, 17, 30));
        assert!(bend_dir(&f, 20, 30).0 > 0.9, "{:?}", bend_dir(&f, 20, 30));
    }

    #[test]
    fn rounded_rectangle_distances_and_normals() {
        let half = (60.0, 40.0);
        // Halfway along the left side, 3 in: 3 deep, the normal straight out.
        let (sd, n) = rounded_rect_sd((-57.0, 0.0), half, 10.0);
        assert!((sd + 3.0).abs() < 1e-9 && n == (-1.0, 0.0), "{sd} {n:?}");
        // Round a 10-unit corner: the distance to its arc, the normal away from its centre.
        let (sd, n) = rounded_rect_sd((-57.0, -37.0), half, 10.0);
        let (cx, cy) = (-50.0, -30.0);
        let len = ((-57.0f64 - cx).powi(2) + (-37.0f64 - cy).powi(2)).sqrt();
        assert!((sd - (len - 10.0)).abs() < 1e-9, "{sd}");
        assert!((n.0 + 7.0 / len).abs() < 1e-9 && (n.1 + 7.0 / len).abs() < 1e-9, "{n:?}");
        // Outside, past the right side.
        assert!((rounded_rect_sd((65.0, 0.0), half, 10.0).0 - 5.0).abs() < 1e-9);
        assert_eq!([(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|q| corner_of([1.0, 2.0, 3.0, 4.0], q)), [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn a_rounded_rectangle_in_closed_form_matches_its_distance_transform() {
        // A 120 × 80 rectangle at (20, 20), bending towards its core 21 px in, both ways.
        let fields = Fields::new(&coverage(160, 120, &[(20.0, 20.0, 120.0, 80.0)]), 0, 0, 2, Some(21.0));
        let rect = RoundedRect { width: 120.0, height: 80.0, radii: [0.0; 4] };
        let closed = AnalyticRect::new(rect, Affine::translate((20.0, 20.0)), 1.0, 21.0).unwrap();
        for (x, y) in [(21, 60), (60, 21), (138, 60), (80, 98), (24, 24), (30, 26), (135, 95), (80, 60)] {
            let (a, b) = (fields.at(x, y).unwrap(), closed.at(x, y));
            assert!((a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3, "({x}, {y}) coverage and depth: {a:?} vs {b:?}");
            assert!(a.1 >= 21.0 || close(a.3, b.3, 0.08), "({x}, {y}) bend: {a:?} vs {b:?}");
        }
        // Rotated a quarter turn, the fields turn with it; skewed or stretched, no closed form.
        let turned = AnalyticRect::new(rect, Affine::translate((100.0, 20.0)) * Affine::rotate(std::f64::consts::FRAC_PI_2), 1.0, 21.0).unwrap();
        assert!(close(turned.at(98, 60).2, (-1.0, 0.0), 1e-6), "{:?}", turned.at(98, 60));
        assert!(AnalyticRect::new(rect, Affine::new([1.0, 0.0, 0.3, 1.0, 0.0, 0.0]), 1.0, 21.0).is_none());
        assert!(AnalyticRect::new(rect, Affine::scale_non_uniform(2.0, 1.0), 1.0, 21.0).is_none());
    }

    #[test]
    fn nearest_sites_and_running_maxima() {
        // Sites at (1, 1) and (8, 3) on a 10 × 5 grid.
        let mut site = vec![false; 50];
        site[11] = true;
        site[38] = true;
        let near = nearest_sites(&site, 10, 5);
        assert_eq!((near[0], near[4], near[49], near[37]), (11, 11, 38, 38));
        assert!(nearest_sites(&[false; 6], 3, 2).iter().all(|s| *s == NO_SITE));
        let m = max_within(&[1.0, 5.0, 2.0, 0.0, 0.0, 3.0], 6, 1, 1);
        assert_eq!(m, [5.0, 5.0, 5.0, 2.0, 3.0, 3.0]);
    }

    /// At full dispersion red's furthest step stays within `Glass::max_bend` of the pixel, all
    /// along the bend, so the backdrop read around a pane covers it: one white pixel further out
    /// than that is never seen.
    #[test]
    fn full_dispersion_reads_no_further_than_max_bend() {
        let bound = Glass { refraction: 1.0, depth: 20.0, dispersion: 1.0, ..Glass::default() }.max_bend() as f32;
        let k = Glass::DISPERSION_SPREAD as f32;
        let mut furthest = 0usize;
        for i in 0..100 {
            let bend = bend_at(i as f32 * 0.25, 20.0, 1.0, 1.0);
            for at in 0..40 {
                let mut impulse = Pixmap::new(40, 1).unwrap();
                impulse.fill(tiny_skia::Color::BLACK);
                impulse.data_mut()[at * 4..at * 4 + 4].copy_from_slice(&[255, 255, 255, 255]);
                let s = dispersed(&impulse, 0.5, 0.5, bend, 0.0, k);
                if s[0].max(s[1]).max(s[2]) > 0.0 {
                    furthest = furthest.max(at);
                }
            }
        }
        // A sample `o` pixels along picks up the pixel `at` while |o − at| < 1.
        assert!(furthest > 0 && (furthest as f32) < bound + 1.0, "pixel {furthest} seen, past {bound}");
    }

    #[test]
    fn distance_of_a_square() {
        let mut p = Pixmap::new(21, 21).unwrap();
        let r = tiny_skia::Rect::from_xywh(5.0, 5.0, 11.0, 11.0).unwrap();
        let paint = tiny_skia::Paint { shader: tiny_skia::Shader::SolidColor(tiny_skia::Color::WHITE), ..Default::default() };
        p.fill_rect(r, &paint, tiny_skia::Transform::identity(), None);
        let d = inside_distance(&p);
        // Outside: 0. The centre pixel (10, 10) is 5.5 px from each edge.
        assert_eq!(d[0], 0.0);
        assert!((d[10 * 21 + 10] - 5.5).abs() < 1e-4, "{}", d[10 * 21 + 10]);
        // A pixel on the edge is half a pixel from it.
        assert!((d[10 * 21 + 5] - 0.5).abs() < 1e-4, "{}", d[10 * 21 + 5]);
    }

    #[test]
    fn the_shape_cache_stays_within_its_bytes() {
        let key = |i: u64| ShapeKey { path: i, linear: [0; 4], frac: (0, 0), margin: 0, r: 1 };
        let mut c = ShapeCache::new(100);
        // Kept from the second render that asks, not the first (nor again within the first).
        assert!(c.slot(&key(1), 40, 1).is_none());
        assert!(c.slot(&key(1), 40, 1).is_none());
        assert!(c.slot(&key(1), 40, 2).is_some());
        for k in [2, 3] {
            assert!(c.slot(&key(k), 40, 2).is_none());
        }
        assert!(c.slot(&key(2), 40, 3).is_some());
        // Using 1 again makes 2 the least recently used: 2 goes to make room for 3.
        assert!(c.slot(&key(1), 40, 3).is_some());
        assert!(c.slot(&key(3), 40, 3).is_some());
        assert!(c.held <= 100, "{}", c.held);
        assert!(c.map.contains_key(&key(1)) && c.map.contains_key(&key(3)) && !c.map.contains_key(&key(2)));
        // More than it holds in all: never kept.
        assert!(c.slot(&key(4), 101, 3).is_none() && c.slot(&key(4), 101, 4).is_none());
        assert!(c.held <= 100 && !c.map.contains_key(&key(4)));
    }

    /// The rim light and the gradient across the curve at depth 20 and a 1-unit bevel, x units in
    /// from the edge: the reference profile's values.
    #[test]
    fn the_light_lies_on_the_bevel_and_the_gradient_across_the_curve() {
        for (x, rim, sheen) in [
            (0.10, 1.0, 1.0),
            (0.25, 0.9888, 1.0),
            (0.50, 0.6843, 1.0),
            (0.75, 0.2330, 1.0),
            (1.00, 0.0, 1.0),
            (2.0, 0.0, 0.8975),
            (4.0, 0.0, 0.7091),
            (6.0, 0.0, 0.5429),
            (8.0, 0.0, 0.3989),
            (16.0, 0.0, 0.0443),
            (20.0, 0.0, 0.0),
            (30.0, 0.0, 0.0),
        ] {
            assert!((rim_light_at(x, 1.0) - rim).abs() < 1e-3, "rim at {x}: {}", rim_light_at(x, 1.0));
            assert!((sheen_at(x, 20.0, 1.0) - sheen).abs() < 1e-3, "gradient at {x}: {}", sheen_at(x, 20.0, 1.0));
        }
        // Facing the light fully, at 45° (cos 0.707) and side on; the opposite side 0.6 as much.
        assert!((catches(1.0) - 1.0).abs() < 1e-6);
        assert!((catches(std::f32::consts::FRAC_1_SQRT_2) - 0.574).abs() < 1e-3, "{}", catches(std::f32::consts::FRAC_1_SQRT_2));
        assert_eq!(catches(0.0), 0.0);
        assert!((catches(-1.0) - 0.6).abs() < 1e-6);
        assert!((catches(-std::f32::consts::FRAC_1_SQRT_2) - 0.6 * 0.574).abs() < 1e-3);
    }
}
