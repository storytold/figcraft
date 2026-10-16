//! The glass effect: the backdrop seen through a pane whose edges curve down like a lens.
//!
//! Within `depth` of the outline the surface curves: the backdrop there is bent towards the inside
//! (strongest at the very edge, `refraction` × `depth` far), each colour channel by a slightly
//! different amount (`dispersion`, a rainbow fringe), and the rim catches the light: brightest
//! where it faces `light_angle`, half as bright on the opposite side, spread wider by `splay`.
//! `frost` blurs the backdrop first. Distances to the outline come from an exact Euclidean
//! distance transform of the shape's coverage.

use figcraft_doc::Glass;
use figcraft_geom::Affine;
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
    let cov = spread_alpha(std::slice::from_ref(shape), 0.0, Affine::translate((f64::from(m), f64::from(m))) * xf, w + 2 * m, h + 2 * m)?;
    let dist = inside_distance(&cov);
    let (cw, ch) = (cov.width() as usize, cov.height() as usize);
    let normals = smooth_normals(&dist, cw, ch, r);
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
    let cd = cov.data();
    let d_at = |x: usize, y: usize| dist.get(y * cw + x).copied().unwrap_or(0.0);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let (mx, my) = (x + m as usize, y + m as usize);
            let c = f32::from(cd.get((my * cw + mx) * 4 + 3).copied().unwrap_or(0)) / 255.0;
            if c <= 0.0 {
                continue;
            }
            let d = d_at(mx, my);
            let inward = normals.get(my * cw + mx).copied().unwrap_or((0.0, 0.0));
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
