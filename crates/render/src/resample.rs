//! Images drawn smaller than their pixels, on the canvas and in exports: each device pixel is the
//! average of the image area it covers, wherever the image falls between pixels (the reference
//! tool's exports; sampling the image at pixel centres instead keeps every edge and fine line
//! aliased, and a copy scaled down first then shifted a fraction of a pixel comes out blurred).
//! Past its edges the image extends its edge pixels, as the pattern it replaces does.

use tiny_skia::{IntRect, Pixmap, Transform};

/// The most weights the two passes may work through (device pixels times the image pixels each
/// averages), so one huge image shown small doesn't hold up an export: past it the image is
/// sampled as before.
const MAX_WORK: u64 = 1 << 28;

/// `src` (premultiplied) as it shows over device area `area` under `m` (image pixels → device),
/// averaged per device pixel; its pixel (0, 0) lies at the area's top-left. `None` unless `m` is
/// axis-aligned and makes the image smaller along both axes.
pub(crate) fn area_averaged(src: &Pixmap, m: Transform, area: IntRect) -> Option<Pixmap> {
    let (sx, sy) = (f64::from(m.sx), f64::from(m.sy));
    let (tx, ty) = (f64::from(m.tx), f64::from(m.ty));
    let straight = m.kx.abs() < 1e-6 && m.ky.abs() < 1e-6;
    if !straight || !(sx.abs() < 1.0 && sy.abs() < 1.0 && sx.abs() > 1e-6 && sy.abs() > 1e-6) || !tx.is_finite() || !ty.is_finite() {
        return None;
    }
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let (dw, dh) = (area.width() as usize, area.height() as usize);
    let cols = spans(area.x(), dw, tx, sx, sw);
    let rows = spans(area.y(), dh, ty, sy, sh);
    // Only the image rows some device row reads are averaged across.
    let (r0, r1) = rows.iter().fold((usize::MAX, 0), |(a, b), s| (a.min(s.first), b.max(s.first + s.weights.len())));
    let r1 = r1.min(sh);
    if r0 >= r1 {
        return None;
    }
    let work = |s: &[Span]| s.iter().map(|s| s.weights.len() as u64).sum::<u64>();
    if work(&cols).saturating_mul((r1 - r0) as u64).saturating_add(work(&rows).saturating_mul(dw as u64)) > MAX_WORK {
        return None;
    }
    let data = src.data();
    // Across: each image row in use, averaged into the device columns.
    let mut across = vec![[0.0_f32; 4]; (r1 - r0) * dw];
    for (r, out) in (r0..r1).zip(across.chunks_mut(dw)) {
        let row = data.get(r * sw * 4..(r + 1) * sw * 4)?;
        for (c, o) in cols.iter().zip(out.iter_mut()) {
            for (k, w) in c.weights.iter().enumerate() {
                let px = row.get((c.first + k) * 4..(c.first + k) * 4 + 4)?;
                for (acc, v) in o.iter_mut().zip(px) {
                    *acc += w * f32::from(*v);
                }
            }
        }
    }
    // Down: the device rows from those.
    let mut out = Pixmap::new(area.width(), area.height())?;
    for (y, s) in rows.iter().enumerate() {
        for x in 0..dw {
            let mut acc = [0.0_f32; 4];
            for (k, w) in s.weights.iter().enumerate() {
                let px = across.get((s.first + k - r0) * dw + x)?;
                for (a, v) in acc.iter_mut().zip(px) {
                    *a += w * v;
                }
            }
            let to = |v: f32| v.round().clamp(0.0, 255.0) as u8;
            let alpha = to(acc[3]);
            let dst = out.data_mut().get_mut((y * dw + x) * 4..(y * dw + x) * 4 + 4)?;
            // Premultiplied: no channel above the alpha, whatever rounding did.
            dst.copy_from_slice(&[to(acc[0]).min(alpha), to(acc[1]).min(alpha), to(acc[2]).min(alpha), alpha]);
        }
    }
    Some(out)
}

/// The image pixels one device column (or row) averages: the first, and each one's weight.
struct Span {
    first: usize,
    weights: Vec<f32>,
}

/// For `n` device pixels from `start`, the image pixels each covers along an axis where image
/// pixel `u` lies at `t + u * s` (device): the overlap of each with the pixel, as a share of the
/// pixel's width in image pixels. What lies past the image's `len` pixels counts as its edge
/// pixel.
fn spans(start: i32, n: usize, t: f64, s: f64, len: usize) -> Vec<Span> {
    let last = len.saturating_sub(1);
    (0..n)
        .map(|i| {
            let d = f64::from(start) + i as f64;
            let (a, b) = ((d - t) / s, (d + 1.0 - t) / s);
            let (a, b) = (a.min(b), a.max(b));
            let total = b - a;
            let clamp = |u: f64| u.clamp(0.0, last as f64) as usize;
            let (first, end) = (clamp(a.floor()), clamp(b.ceil() - 1.0));
            let mut weights = vec![0.0_f32; end - first + 1];
            // Each image pixel's own overlap; what lies before the image goes to the first and
            // past it to the last.
            let mut u = a.floor();
            while u < b {
                let w = (b.min(u + 1.0) - a.max(u)) / total;
                if let Some(slot) = weights.get_mut(clamp(u) - first) {
                    *slot += w as f32;
                }
                u += 1.0;
            }
            Span { first, weights }
        })
        .collect()
}
