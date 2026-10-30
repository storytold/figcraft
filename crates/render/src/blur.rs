//! Layer and background blurs: uniform (one Gaussian) or progressive (the radius ramps linearly
//! along a line, from a start radius at its start point to an end radius at its end point).
//!
//! A progressive blur blurs the pixels at a few radii between its smallest and largest radius,
//! spaced geometrically (steps between small radii show the most), and blends every pixel between
//! the two levels around its own radius. Each level is one Gaussian-like blur (three box passes,
//! linear in the pixel count whatever the radius), so the cost is a handful of uniform blurs.

use figcraft_doc::Blur;
use figcraft_geom::{Affine, Point};
use tiny_skia::Pixmap;

use crate::fx;

/// Most blur levels a progressive blur draws.
const MAX_LEVELS: usize = 8;
/// Ratio between neighbouring levels' radii (above one pixel).
const LEVEL_RATIO: f64 = 1.5;

/// Blur `p` with `b`. `size` is the layer's box (local units, for the progressive line's
/// unit-box points), `xf` maps local units to `p`'s pixels and `scale` is device pixels per unit.
/// `accelerate` lets the canvas's GPU blur (see [`fx::blur`]); exports never ask for it.
pub fn apply(p: &mut Pixmap, b: &Blur, size: (f64, f64), xf: Affine, scale: f64, accelerate: bool) {
    let b = b.sanitized();
    let sigma = |r: f64| r / 2.0 * scale;
    if !b.is_progressive() {
        fx::blur(p, sigma(b.radius), accelerate);
        return;
    }
    let at = |u: (f64, f64)| xf * Point::new(u.0 * size.0, u.1 * size.1);
    progressive(p, sigma(b.start_radius), sigma(b.radius), at(b.start), at(b.end), accelerate);
}

/// The blur's standard deviation along the line: `s0` at `a` and before it, `s1` at `b` and past
/// it, linear in between (pixels).
struct Ramp {
    s0: f64,
    s1: f64,
    /// t at pixel (0, 0)'s centre and its change per pixel across and down.
    t0: f64,
    tx: f64,
    ty: f64,
}

impl Ramp {
    fn new(s0: f64, s1: f64, a: Point, b: Point) -> Option<Ramp> {
        let d = b - a;
        let len2 = d.hypot2();
        if !(len2.is_finite() && len2 > 1e-12 && a.x.is_finite() && a.y.is_finite()) {
            return None;
        }
        let (tx, ty) = (d.x / len2, d.y / len2);
        Some(Ramp { s0, s1, t0: (0.5 - a.x) * tx + (0.5 - a.y) * ty, tx, ty })
    }
    fn sigma(&self, x: f64, y: f64) -> f64 {
        let t = (self.t0 + x * self.tx + y * self.ty).clamp(0.0, 1.0);
        self.s0 + (self.s1 - self.s0) * t
    }
}

/// The radii blurred for a ramp from `lo` to `hi` (pixels): `lo`, then geometric steps from one
/// pixel (or `lo`) up to `hi`, at most [`MAX_LEVELS`].
pub fn levels(lo: f64, hi: f64) -> Vec<f64> {
    let mut out = Vec::with_capacity(MAX_LEVELS);
    let base = lo.max(1.0);
    if lo < base {
        out.push(lo);
    }
    if hi <= base {
        if hi > lo || out.is_empty() {
            out.push(hi);
        }
        return out;
    }
    let steps = ((hi / base).ln() / LEVEL_RATIO.ln()).ceil().clamp(1.0, (MAX_LEVELS - 2) as f64) as usize;
    out.extend((0..steps).map(|k| base * (hi / base).powf(k as f64 / steps as f64)));
    out.push(hi);
    out
}

/// The level below `s` and the weight (0–255) of the level above it.
fn bracket(levels: &[f64], s: f64) -> (u8, u8) {
    let n = levels.len();
    if n < 2 {
        return (0, 0);
    }
    let k = levels.partition_point(|v| *v <= s).saturating_sub(1).min(n - 2);
    let (Some(lo), Some(hi)) = (levels.get(k), levels.get(k + 1)) else { return (0, 0) };
    let f = if hi > lo { ((s - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.0 };
    (k as u8, (f * 255.0).round() as u8)
}

/// A progressive blur of `p`: standard deviation `s0` at pixel point `a` (and before it) to `s1`
/// at `b` (and past it). A degenerate line blurs with `s1` everywhere.
pub fn progressive(p: &mut Pixmap, s0: f64, s1: f64, a: Point, b: Point, accelerate: bool) {
    let cap = |s: f64| if s.is_finite() { s.clamp(0.0, fx::MAX_SIGMA) } else { 0.0 };
    let (s0, s1) = (cap(s0), cap(s1));
    let Some(ramp) = Ramp::new(s0, s1, a, b).filter(|_| (s1 - s0).abs() >= 0.05) else {
        fx::blur(p, s1, accelerate);
        return;
    };
    let (w, h) = (p.width(), p.height());
    // The levels span the whole ramp, not just the part on this pixmap: a layer cut by a band or
    // the view blends the same levels as the whole layer would.
    let (lo, hi) = (s0.min(s1), s0.max(s1));
    let lv = levels(lo, hi);
    // Each level's pixels: the box they lie in (inclusive), where it is used at all.
    let mut used: [Option<[u32; 4]>; MAX_LEVELS] = [None; MAX_LEVELS];
    let mut mark = |k: usize, x: u32, y: u32| {
        if let Some(u) = used.get_mut(k) {
            *u = Some(match *u {
                Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
                None => [x, y, x, y],
            });
        }
    };
    let mut sel = Vec::with_capacity(w as usize * h as usize);
    for y in 0..h {
        for x in 0..w {
            let (k, f) = bracket(&lv, ramp.sigma(f64::from(x), f64::from(y)));
            mark(usize::from(k), x, y);
            if f > 0 {
                mark(usize::from(k) + 1, x, y);
            }
            sel.push((k, f));
        }
    }
    // Each pixel: Σ weight × level value, the weights of its two levels summing to 255.
    let mut acc = vec![0u16; p.data().len()];
    for (k, s) in lv.iter().enumerate() {
        let Some(Some([x0, y0, x1, y1])) = used.get(k).copied() else { continue };
        // A level is blurred only around the pixels that use it: a pixel's blur reads at most
        // 3 sigma + 3 pixels away (see `crate::backdrop_reach`), so the result is exact.
        let reach = (s * 3.0).ceil() as u32 + 3;
        let (cx, cy) = (x0.saturating_sub(reach), y0.saturating_sub(reach));
        let (cw, ch) = ((x1 + reach + 1).min(w) - cx, (y1 + reach + 1).min(h) - cy);
        let Some(mut level) = tiny_skia::IntRect::from_xywh(cx as i32, cy as i32, cw, ch).and_then(|r| p.clone_rect(r)) else { continue };
        fx::blur(&mut level, *s, accelerate);
        let ld = level.data();
        let row = cw as usize * 4;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let i = (y * w + x) as usize;
                let Some(&(j, f)) = sel.get(i) else { continue };
                let j = usize::from(j);
                let wgt = if j == k {
                    255 - u16::from(f)
                } else if j + 1 == k {
                    u16::from(f)
                } else {
                    continue;
                };
                let li = (y - cy) as usize * row + (x - cx) as usize * 4;
                if let (Some(px), Some(out)) = (ld.get(li..li + 4), acc.get_mut(i * 4..i * 4 + 4)) {
                    for (o, c) in out.iter_mut().zip(px) {
                        *o += wgt * u16::from(*c);
                    }
                }
            }
        }
    }
    for (d, a) in p.data_mut().iter_mut().zip(&acc) {
        *d = ((u32::from(*a) + 127) / 255) as u8;
    }
}
