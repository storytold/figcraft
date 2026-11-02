//! The glass effect: the backdrop seen through a pane whose edges curve down like a lens.
//!
//! Within `depth` of the outline the surface curves like a thick lens: the backdrop there is bent
//! towards the inside, most a [`BEVEL`] in from the edge (`refraction` × `depth` far), falling off
//! as the 3.5th power of the way out of the curve and gone `depth` further in, with a fifth as
//! much on the bevel itself ([`bend_at`]). `splay` turns which way it bends, from along the edge's
//! normal (0) to away from the layer's centre (1). Each colour channel bends by a different amount
//! (`dispersion`, a rainbow fringe: red furthest, blue least, each smeared along the bend). The
//! bevel catches the light as a thin line added to the backdrop ([`rim_light_at`]): fully where the
//! edge faces `light_angle`, none below 0.35 of facing it, 0.6 as much on the opposite side; and a
//! faint gradient runs across the curve, 5% darker where it faces the light and lighter opposite.
//! `frost` blurs the backdrop first. Distances to the outline come from an exact Euclidean
//! distance transform of the shape's coverage.
//!
//! The shape's coverage, distances and normals depend only on the shape and the zoom. Once a
//! later render asks for the same shape at the same zoom they are worked out for the whole shape,
//! in its own pixel frame, and kept in a cache of at most [`MAX_CACHED_BYTES`]: moving or editing
//! what lies under the glass, dragging its sliders, panning and patching the canvas reuse them.
//! Zooming doesn't (each zoom is new).

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use figcraft_doc::Glass;
use figcraft_geom::Affine;
use figcraft_geom::kurbo::Shape as _;
use tiny_skia::Pixmap;

use crate::backdrop::EffectFrame;
use crate::fx;
use crate::shadow::{Outline, spread_alpha};

/// Squared distance transform of one row or column (Felzenszwalb & Huttenlocher): `f` holds 0 at
/// sources and a large value elsewhere; `d` gets each sample's squared distance to the nearest
/// source. `v` and `z` are scratch space (`f.len()` and `f.len() + 1` long).
fn edt_1d(f: &[f64], d: &mut [f64], v: &mut [usize], z: &mut [f64]) {
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
        edt_1d(f.get(..h).unwrap_or(&[]), d.get_mut(..h).unwrap_or(&mut []), &mut v, &mut z);
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
        edt_1d(f.get(..w).unwrap_or(&[]), d.get_mut(..w).unwrap_or(&mut []), &mut v, &mut z);
        if let (Some(dst), Some(src)) = (g.get_mut(row..row + w), d.get(..w)) {
            dst.copy_from_slice(src);
        }
    }
    // Centre-to-centre distance to the nearest outside pixel → distance to the edge between.
    g.iter().map(|s| if *s > 0.0 { (s.sqrt() - 0.5).max(0.0) as f32 } else { 0.0 }).collect()
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
    let mut tmp = vec![0f32; w * h];
    for g in [&mut gx, &mut gy] {
        for _ in 0..2 {
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
    let sd = -x;
    let t = lin(-depth - bevel, -bevel, sd);
    let most = MAX_BEND * depth;
    let core = (t.powf(PROFILE_EXP) * depth * refraction).clamp(-most, most);
    let rim = refraction * RIM_BEND * most;
    let u = lin(-bevel - 1e-4, -0.3 * bevel, sd);
    core + (rim - core) * u
}

/// The backdrop seen at pixel coordinates (`x`, `y`) moved by (`dx`, `dy`), its channels split by
/// `k` (dispersion, 0–1): red moves up to `k` further, blue up to `k` less, each smeared along the
/// move by a triangular window of [`DISPERSION_STEPS`]. One sample when `k` is too small to show.
fn dispersed(frosted: &Pixmap, x: f32, y: f32, dx: f32, dy: f32, k: f32) -> [f32; 4] {
    let at = |s: f32| sample(frosted, x + dx * s, y + dy * s);
    if k < 0.001 {
        let s = at(1.0);
        return [s[0].min(s[3]), s[1].min(s[3]), s[2].min(s[3]), s[3]];
    }
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

/// A shape's coverage (alpha), distance to its edge and smoothed inward normals over a window of
/// pixels whose top-left pixel is (`x0`, `y0`).
struct Fields {
    x0: i64,
    y0: i64,
    w: usize,
    h: usize,
    cov: Vec<u8>,
    dist: Vec<f32>,
    normals: Vec<(f32, f32)>,
}

/// Bytes a field pixel holds: coverage, distance and a normal.
const FIELD_PX_BYTES: u64 = 1 + 4 + 8;

impl Fields {
    /// The fields of `cov`, the shape's coverage on the window at (`x0`, `y0`), normals smoothed
    /// over `r` pixels.
    fn new(cov: &Pixmap, x0: i64, y0: i64, r: usize) -> Fields {
        FIELDS_COMPUTED.with(|n| n.set(n.get() + 1));
        let dist = inside_distance(cov);
        let (w, h) = (cov.width() as usize, cov.height() as usize);
        let normals = smooth_normals(&dist, w, h, r);
        let cov = cov.data().chunks_exact(4).map(|p| p.get(3).copied().unwrap_or(0)).collect();
        Fields { x0, y0, w, h, cov, dist, normals }
    }

    /// Coverage (0–1), distance and inward normal at pixel (`x`, `y`); `None` outside the window.
    fn at(&self, x: i64, y: i64) -> Option<(f32, f32, (f32, f32))> {
        let (fx, fy) = (x.checked_sub(self.x0)?, y.checked_sub(self.y0)?);
        if fx < 0 || fy < 0 || fx >= self.w as i64 || fy >= self.h as i64 {
            return None;
        }
        let i = fy as usize * self.w + fx as usize;
        Some((f32::from(*self.cov.get(i)?) / 255.0, *self.dist.get(i)?, *self.normals.get(i)?))
    }
}

thread_local! {
    static FIELDS_COMPUTED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many times this thread has worked out a glass shape's fields: what a cache miss costs. For
/// tests and benchmarks.
pub fn glass_fields_computed() -> u64 {
    FIELDS_COMPUTED.with(std::cell::Cell::get)
}

/// What a glass shape's fields depend on, with the transform's translation reduced to its fraction
/// of a pixel: the same shape at the same zoom has the same key wherever it is and however a render
/// cuts it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ShapeKey {
    path: u64,
    linear: [u64; 4],
    frac: (i64, i64),
    margin: u32,
    r: usize,
}

/// The cached fields of a shape whose window holds at most this many pixels; bigger shapes (a
/// pane far larger than the view) are worked out for the part a render shows each time.
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
type Slot = Arc<OnceLock<Option<Arc<Fields>>>>;

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

/// The fields `glass` reads for a `w` × `h` layer whose `shape` maps through `xf` (local → layer
/// pixels), exact for distances up to the margin `m`, normals smoothed over `r` pixels, and the
/// shift from layer pixels to the fields' pixels.
///
/// A shape asked for again in a later render, whose window fits [`MAX_SHAPE_PX`], is worked out
/// whole, in its own frame (the translation's fraction of a pixel), and kept: a band, a canvas
/// patch or the next frame of a drag reads the same fields. Otherwise it is worked out for the part
/// shown plus the margin.
fn shape_fields(shape: &Outline, xf: Affine, w: u32, h: u32, m: u32, r: usize) -> Option<(Arc<Fields>, (i64, i64))> {
    if let Some(whole) = whole_shape(shape, xf, m, r) {
        return Some(whole);
    }
    let cov = spread_alpha(std::slice::from_ref(shape), 0.0, Affine::translate((f64::from(m), f64::from(m))) * xf, w + 2 * m, h + 2 * m)?;
    Some((Arc::new(Fields::new(&cov, -i64::from(m), -i64::from(m), r)), (0, 0)))
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

/// [`shape_fields`] for a whole shape, from the cache; `None` when it isn't kept (first asked for
/// in this render, too big, or its transform too far out).
fn whole_shape(shape: &Outline, xf: Affine, m: u32, r: usize) -> Option<(Arc<Fields>, (i64, i64))> {
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
        Some(Fields::new(&win, origin.0, origin.1, r))
    };
    let slot =
        shape_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner).slot(&key, ww * wh * FIELD_PX_BYTES, crate::render_number())?;
    Some((slot.get_or_init(|| compute().map(Arc::new)).clone()?, shift))
}

/// The glass of a `w` × `h` layer whose `shape` maps through `xf` (local → layer pixels), from
/// `back`, its backdrop padded by `pad` pixels each way, at `scale` device pixels per unit.
pub fn glass(g: &Glass, frame: EffectFrame<'_>) -> Option<Pixmap> {
    let EffectFrame { shape, back, pad, size: (w, h), xf, scale } = frame;
    let g = g.sanitized();
    let depth = (g.depth * scale).clamp(1.0, 1024.0);
    let bevel = (BEVEL * scale).min(1024.0);
    let r = (depth * 0.1).clamp(1.0, 8.0) as usize;
    // Coverage with a margin, so wherever the layer is cut every distance the pixels read is
    // exact: the bend's and the gradient's (within `depth` + `bevel`; the light lies on the bevel)
    // and those the normals are smoothed from (2r + 1 further).
    let m = (depth + bevel).ceil() as u32 + 4 * r as u32 + 4;
    let (fields, shift) = shape_fields(shape, xf, w, h, m, r)?;
    let mut frosted = back.clone();
    fx::blur(&mut frosted, g.frost / 2.0 * scale);
    let ang = g.light_angle.to_radians();
    let light = (ang.sin() as f32, -ang.cos() as f32);
    // The layer's centre in its own frame (see `own_frame`), which `splay` bends away from: the
    // direction to it is the same wherever a render puts or cuts the layer.
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
            let Some((c, d, inward)) = fields.at(x as i64 - shift.0, y as i64 - shift.1) else { continue };
            if c <= 0.0 {
                continue;
            }
            // The curve's outward direction: the edge's normal, turned by `splay` towards the way
            // away from the centre (not renormalised: where the two part, the bend is less).
            let away = if splay > 0.0 {
                let px = ((x as i64 - sx) as f64 + 0.5 - centre.x) as f32;
                let py = ((y as i64 - sy) as f64 + 0.5 - centre.y) as f32;
                let len = (px * px + py * py).sqrt();
                if len > 1e-6 { (px / len, py / len) } else { (0.0, 0.0) }
            } else {
                (0.0, 0.0)
            };
            let out_dir = (-inward.0 * (1.0 - splay) + away.0 * splay, -inward.1 * (1.0 - splay) + away.1 * splay);
            let bend = bend_at(d, depth, bevel, refraction);
            let (bx, by) = (x as f32 + pad as f32 + 0.5, y as f32 + pad as f32 + 0.5);
            let base = if bend == 0.0 {
                // Nothing bends (past `depth` + `bevel`, or no refraction): the channels' samples
                // are at the same point, so one sample.
                let s = sample(&frosted, bx, by);
                [s[0].min(s[3]), s[1].min(s[3]), s[2].min(s[3]), s[3]]
            } else {
                dispersed(&frosted, bx, by, -out_dir.0 * bend, -out_dir.1 * bend, dispersion)
            };
            // The bevel catches the light as a line added to the backdrop, by how much the edge
            // faces it; across the curve a faint gradient, darker on the side facing it.
            let facing = -(inward.0 * light.0 + inward.1 * light.1);
            let rim = rim_light_at(d, bevel) * catches(facing);
            let sheen = -(out_dir.0 * light.0 + out_dir.1 * light.1) * sheen_at(d, depth, bevel) * BROAD;
            let add = 255.0 * intensity * (rim + sheen);
            let i = (y * w as usize + x) * 4;
            if let Some(p) = dst.get_mut(i..i + 4) {
                let a = base[3];
                for (k, o) in p.iter_mut().enumerate() {
                    let v = base.get(k).copied().unwrap_or(0.0);
                    let v = if k < 3 { (v + add).clamp(0.0, a) } else { v };
                    *o = (v * c).round().clamp(0.0, 255.0) as u8;
                }
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
