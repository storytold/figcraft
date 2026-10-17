//! The glass effect: the backdrop seen through a pane whose edges curve down like a lens.
//!
//! Within `depth` of the outline the surface curves: the backdrop there is bent towards the inside
//! (strongest at the very edge, `refraction` × `depth` far), each colour channel by a slightly
//! different amount (`dispersion`, a rainbow fringe), and the rim catches the light: brightest
//! where it faces `light_angle`, half as bright on the opposite side, spread wider by `splay`.
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

/// The furthest in (pixels) the rim light is worth half a level or more: where
/// `255 × intensity × e^(−d / band)` falls to 0.5, at most [`MAX_LIGHT_PX`] (0 for no light).
fn light_reach(band: f64, intensity: f64) -> f64 {
    let k = 255.0 * intensity / 0.5;
    if k > 1.0 && band.is_finite() { (band * k.ln()).min(MAX_LIGHT_PX) } else { 0.0 }
}

/// The light reaches no further in than this many pixels (a wide light on a deep pane at a high
/// zoom would otherwise need a huge margin of coverage around every cut).
const MAX_LIGHT_PX: f64 = 512.0;

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

/// [`shape_fields`] for a whole shape, from the cache; `None` when it isn't kept (first asked for
/// in this render, too big, or its transform too far out).
fn whole_shape(shape: &Outline, xf: Affine, m: u32, r: usize) -> Option<(Arc<Fields>, (i64, i64))> {
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
    let own = Affine::new([a, b, c, d, frac.0 as f64 / STEPS, frac.1 as f64 / STEPS]);
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
#[allow(clippy::too_many_arguments)]
pub fn glass(g: &Glass, shape: &Outline, back: &Pixmap, pad: u32, w: u32, h: u32, xf: Affine, scale: f64) -> Option<Pixmap> {
    let g = g.sanitized();
    let depth = (g.depth * scale).clamp(1.0, 1024.0);
    let band = (depth * (0.15 + 0.35 * g.splay)).max(1.0);
    let reach = light_reach(band, g.light_intensity);
    let r = (depth * 0.1).clamp(1.0, 8.0) as usize;
    // Coverage with a margin, so wherever the layer is cut every distance the pixels read is
    // exact: the bend's (within `depth`), the light's (within `reach`) and those the normals are
    // smoothed from (2r + 1 further).
    let m = depth.max(reach).ceil() as u32 + 4 * r as u32 + 4;
    let (fields, shift) = shape_fields(shape, xf, w, h, m, r)?;
    let mut frosted = back.clone();
    fx::blur(&mut frosted, g.frost / 2.0 * scale);
    let ang = g.light_angle.to_radians();
    let light = (ang.sin() as f32, -ang.cos() as f32);
    let bend_max = (g.refraction * depth) as f32;
    let fringe = (g.dispersion * 0.25) as f32;
    let power = (6.0 - 5.0 * g.splay) as f32;
    let (band, reach) = (band as f32, reach as f32);
    // The falloff, lowered by its value at `reach` and scaled back to 1 at the edge, so the light
    // fades out to 0 at `reach` instead of stopping there.
    let floor = (-reach / band).exp();
    let fall = 1.0 - floor;
    let intensity = g.light_intensity as f32;
    let depth = depth as f32;
    let mut out = Pixmap::new(w, h)?;
    let dst = out.data_mut();
    for y in 0..h as usize {
        for x in 0..w as usize {
            let Some((c, d, inward)) = fields.at(x as i64 - shift.0, y as i64 - shift.1) else { continue };
            if c <= 0.0 {
                continue;
            }
            let t = (d / depth).min(1.0);
            let bend = bend_max * (1.0 - t) * (1.0 - t);
            let (bx, by) = (x as f32 + pad as f32 + 0.5, y as f32 + pad as f32 + 0.5);
            let base = if bend == 0.0 {
                // Nothing bends (past `depth`, or no refraction): the three channels' samples are
                // at the same point, so one sample.
                let s = sample(&frosted, bx, by);
                [s[0].min(s[3]), s[1].min(s[3]), s[2].min(s[3]), s[3]]
            } else {
                let at = |k: f32| sample(&frosted, bx + inward.0 * bend * k, by + inward.1 * bend * k);
                let (rs, gs, bs) = (at(1.0 - fringe), at(1.0), at(1.0 + fringe));
                let a = rs[3].max(gs[3]).max(bs[3]);
                [rs[0].min(a), gs[1].min(a), bs[2].min(a), a]
            };
            // The rim catches the light where it faces it, and half as much opposite. It fades out
            // by `reach` and is none past it, wherever the layer is cut.
            let hl = if d < reach && fall > 0.0 {
                let facing = -(inward.0 * light.0 + inward.1 * light.1);
                let spec = facing.max(0.0).powf(power) + 0.5 * (-facing).max(0.0).powf(power);
                (intensity * spec * ((-d / band).exp() - floor) / fall).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let i = (y * w as usize + x) * 4;
            if let Some(p) = dst.get_mut(i..i + 4) {
                for (o, v) in p.iter_mut().zip(base) {
                    *o = ((v * (1.0 - hl) + 255.0 * hl) * c).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn light_reach_is_where_the_light_falls_under_half_a_level() {
        let r = light_reach(10.0, 1.0);
        assert!((255.0 * (-r / 10.0).exp() - 0.5).abs() < 1e-9, "{r}");
        // No light, too faint to show, or a band that isn't a number: it reaches nowhere.
        for (band, intensity) in [(10.0, 0.0), (10.0, 0.5 / 255.0), (f64::NAN, 1.0), (f64::INFINITY, 1.0)] {
            assert_eq!(light_reach(band, intensity), 0.0, "{band} {intensity}");
        }
        assert_eq!(light_reach(1e6, 1.0), MAX_LIGHT_PX);
    }
}
