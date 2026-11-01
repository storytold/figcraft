//! Grain effects: noise (grains over the layer's pixels) and texture (the layer's pixels scattered
//! into a rough edge).
//!
//! Grains are cells of the effect's size in the layer's own coordinates, each with a random value
//! hashed from its position and the layer: they stay put when the canvas pans or renders in pieces,
//! scale with the layer when zooming, and every export draws the same grains.

use figcraft_doc::{Noise, NoiseMode, Texture};
use figcraft_geom::Affine;
use tiny_skia::Pixmap;

/// A well-mixed 64-bit hash of a grain cell.
fn hash(ix: i64, iy: i64, seed: u32) -> u64 {
    let mut h = (ix as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (iy as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ u64::from(seed).wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    h ^ (h >> 33)
}

/// 16 bits of `h` from bit `at`, as 0..1 (1 excluded: a density of 1 draws every grain).
fn unit(h: u64, at: u32) -> f32 {
    f32::from(((h >> at) & 0xFFFF) as u16) / 65536.0
}

/// Cell positions are rounded to 1 / `STEPS_PER_CELL` of a cell before they're cut into cells. A
/// pixel centre can fall exactly on a cell edge (at a zoom of 1.7, a 1-unit cell is 1.7 pixels
/// across: every tenth edge does), and a view whose origin is a whole number of pixels off (a
/// patch, a band, a pan) reaches it with another rounding error. Unrounded, a row of grains lands
/// in the next cell; rounded, the edge moves 1 / 2²¹ of a cell aside, off the centres layers at
/// round positions and zooms put it on.
const STEPS_PER_CELL: f64 = 1_048_576.0;

/// Maps layer pixels to the grain cells under their centres.
struct Cells {
    /// Layer pixels → local units, divided by the grain size: `[a, b, c, d, e, f]`.
    m: [f64; 6],
}

impl Cells {
    /// `xf` maps local units to layer pixels; `grain` is the cell size in local units.
    fn new(xf: Affine, grain: (f64, f64)) -> Option<Cells> {
        let inv = xf.inverse();
        let c = inv.as_coeffs();
        let (gx, gy) = grain;
        let m = [c[0] / gx, c[1] / gy, c[2] / gx, c[3] / gy, c[4] / gx, c[5] / gy];
        m.iter().all(|v| v.is_finite()).then_some(Cells { m })
    }
    fn at(&self, x: u32, y: u32) -> (i64, i64) {
        let (px, py) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
        let [a, b, c, d, e, f] = self.m;
        let cell = |v: f64| ((v * STEPS_PER_CELL).round() / STEPS_PER_CELL).floor() as i64;
        (cell(a * px + c * py + e), cell(b * px + d * py + f))
    }
}

/// Draw `nz`'s grains over the pixels of `layer` (local → layer pixels `xf`), each grain at the
/// alpha of the pixel under it, blended with the noise's blend mode. Grains are twice the noise
/// size across.
pub fn noise(layer: &mut Pixmap, nz: &Noise, xf: Affine, seed: u32) {
    if let Some(g) = grains(layer, nz, xf, seed) {
        crate::paint::composite(layer, (0, 0), &g, 1.0, nz.blend);
    }
}

/// `nz`'s grains for the pixels of `layer` (see [`noise`]), to draw over it with the noise's blend
/// mode; `None` when there are none.
pub fn grains(layer: &Pixmap, nz: &Noise, xf: Affine, seed: u32) -> Option<Pixmap> {
    let (sx, sy) = nz.grain();
    let cells = Cells::new(xf, (sx * 2.0, sy * 2.0))?;
    let density = if nz.density.is_finite() { nz.density.clamp(0.0, 1.0) as f32 } else { 0.0 };
    if density <= 0.0 {
        return None;
    }
    let (w, h) = (layer.width(), layer.height());
    let mut grains = Pixmap::new(w, h)?;
    let (c1, c2) = (nz.color.sanitized(), nz.secondary.sanitized());
    let multi_alpha = if nz.opacity.is_finite() { nz.opacity.clamp(0.0, 1.0) } else { 0.0 };
    let src = layer.data();
    let dst = grains.data_mut();
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let a = f32::from(src.get(i + 3).copied().unwrap_or(0)) / 255.0;
            if a <= 0.0 {
                continue;
            }
            let (ix, iy) = cells.at(x, y);
            let hv = hash(ix, iy, seed);
            if unit(hv, 0) >= density {
                continue;
            }
            let [r, g, b, ca] = match nz.mode {
                NoiseMode::Mono => [c1.r, c1.g, c1.b, c1.a],
                NoiseMode::Duo if hv >> 63 == 1 => [c2.r, c2.g, c2.b, c2.a],
                NoiseMode::Duo => [c1.r, c1.g, c1.b, c1.a],
                NoiseMode::Multi => [unit(hv, 16), unit(hv, 32), unit(hv, 48), multi_alpha],
            };
            let al = ca * a;
            let q = |v: f32| (v * al * 255.0).round().clamp(0.0, 255.0) as u8;
            if let Some(px) = dst.get_mut(i..i + 4) {
                px.copy_from_slice(&[q(r), q(g), q(b), (al * 255.0).round().clamp(0.0, 255.0) as u8]);
            }
        }
    }
    Some(grains)
}

/// Scatter `layer`'s pixels by up to the texture's radius (`scale` device pixels per unit), one
/// random move per grain (twice the texture size across): edges turn rough and grainy. Clipped to
/// the shape, the result stays within the layer's own pixels.
pub fn texture(layer: &mut Pixmap, t: &Texture, xf: Affine, scale: f64, seed: u32) {
    let r = t.radius * scale;
    if !(r.is_finite() && r >= 0.5) {
        return;
    }
    let r = r.min(2048.0) as f32;
    let (sx, sy) = t.grain();
    let Some(cells) = Cells::new(xf, (sx * 2.0, sy * 2.0)) else { return };
    let (w, h) = (layer.width(), layer.height());
    let src = layer.data().to_vec();
    let dst = layer.data_mut();
    for y in 0..h {
        for x in 0..w {
            let (ix, iy) = cells.at(x, y);
            let hv = hash(ix, iy, seed ^ 0x7E57_u32);
            // A uniform point in the disc of radius r.
            let (ang, dist) = (unit(hv, 0) * std::f32::consts::TAU, unit(hv, 16).sqrt() * r);
            // The move in whole pixels, apart from `x` and `y`: the same pixel of the page moves
            // the same way wherever the layer's pixels start.
            let (mx, my) = ((0.5 + ang.cos() * dist).floor() as i64, (0.5 + ang.sin() * dist).floor() as i64);
            let (fx, fy) = (i64::from(x) + mx, i64::from(y) + my);
            let i = ((y * w + x) * 4) as usize;
            let mut px = [0u8; 4];
            if fx >= 0 && fy >= 0 && fx < i64::from(w) && fy < i64::from(h) {
                let j = ((fy * i64::from(w) + fx) * 4) as usize;
                if let Some(s) = src.get(j..j + 4) {
                    px.copy_from_slice(s);
                }
            }
            if t.clip_to_shape {
                let keep = u32::from(src.get(i + 3).copied().unwrap_or(0));
                px = px.map(|c| ((u32::from(c) * keep + 127) / 255) as u8);
            }
            if let Some(d) = dst.get_mut(i..i + 4) {
                d.copy_from_slice(&px);
            }
        }
    }
}
