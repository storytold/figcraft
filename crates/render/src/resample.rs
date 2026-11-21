//! Images drawn smaller than their pixels, on the canvas and in exports: each device pixel is the
//! average of the image area it covers, wherever the image falls between pixels (the reference
//! tool's exports; sampling the image at pixel centres instead keeps every edge and fine line
//! aliased, and a copy scaled down first then shifted a fraction of a pixel comes out blurred).
//! Past its edges the image extends its edge pixels, as the pattern it replaces does.

use tiny_skia::{IntRect, Pixmap, Rect, Transform};

/// The most weights the two passes may work through (the image pixels each device column averages
/// times those each device row does), so one huge image shown small doesn't hold up an export:
/// past it the image is sampled as before.
const MAX_WORK: u64 = 1 << 28;

/// `src` (premultiplied) as it shows over device area `area` under `m` (image pixels → device),
/// averaged per device pixel; its pixel (0, 0) lies at the area's top-left. Each pixel averages
/// only the part of it inside `clip` (the shape the image fills, in device pixels), so where the
/// shape's edge cuts a pixel, its antialiasing alone fades it (once, not again for the image).
/// `None` unless `m` is axis-aligned and makes the image smaller along both axes.
pub(crate) fn area_averaged(src: &Pixmap, m: Transform, area: IntRect, clip: Rect) -> Option<Pixmap> {
    let (sx, sy) = (f64::from(m.sx), f64::from(m.sy));
    let (tx, ty) = (f64::from(m.tx), f64::from(m.ty));
    let straight = m.kx.abs() < 1e-6 && m.ky.abs() < 1e-6;
    if !straight || !(sx.abs() < 1.0 && sy.abs() < 1.0 && sx.abs() > 1e-6 && sy.abs() > 1e-6) || !tx.is_finite() || !ty.is_finite() {
        return None;
    }
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let (dw, dh) = (area.width() as usize, area.height() as usize);
    let cols = spans(area.x(), dw, (clip.left().into(), clip.right().into()), tx, sx, sw)?;
    let rows = spans(area.y(), dh, (clip.top().into(), clip.bottom().into()), ty, sy, sh)?;
    let work = |s: &[Span]| s.iter().map(|s| s.weights.len() as u64).sum::<u64>();
    if work(&cols).saturating_mul(work(&rows)) > MAX_WORK {
        return None;
    }
    let _held = crate::layer_budget::Held::take(area.width(), area.height(), 1)?;
    let mut out = Pixmap::new(area.width(), area.height())?;
    let data = src.data();
    // Each device row: the image rows it covers, each averaged across into the device columns
    // and weighted in (only one device row of scratch, whatever the image's size).
    let mut acc = vec![[0.0_f32; 4]; dw];
    for (y, s) in rows.iter().enumerate() {
        acc.iter_mut().for_each(|a| *a = [0.0; 4]);
        for (k, wy) in s.weights.iter().enumerate() {
            let r = s.first + k;
            let row = data.get(r * sw * 4..(r + 1) * sw * 4)?;
            for (c, o) in cols.iter().zip(acc.iter_mut()) {
                for (j, wx) in c.weights.iter().enumerate() {
                    let px = row.get((c.first + j) * 4..(c.first + j) * 4 + 4)?;
                    let w = wx * wy;
                    for (a, v) in o.iter_mut().zip(px) {
                        *a += w * f32::from(*v);
                    }
                }
            }
        }
        let to = |v: f32| v.round().clamp(0.0, 255.0) as u8;
        let line = out.data_mut().get_mut(y * dw * 4..(y + 1) * dw * 4)?;
        for (dst, a) in line.chunks_exact_mut(4).zip(&acc) {
            let alpha = to(a[3]);
            // Premultiplied: no channel above the alpha, whatever rounding did.
            dst.copy_from_slice(&[to(a[0]).min(alpha), to(a[1]).min(alpha), to(a[2]).min(alpha), alpha]);
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
/// pixel `u` lies at `t + u * s` (device): the overlap of each with the part of the pixel inside
/// `clip` (device; all of it when none is), as a share of that part. What lies past the image's
/// `len` pixels counts as its edge pixel. `None` for an empty image.
fn spans(start: i32, n: usize, clip: (f64, f64), t: f64, s: f64, len: usize) -> Option<Vec<Span>> {
    let last = len.checked_sub(1)?;
    let spans = (0..n)
        .map(|i| {
            let d = f64::from(start) + i as f64;
            let (d0, d1) = (d.max(clip.0), (d + 1.0).min(clip.1));
            let (d0, d1) = if d1 > d0 { (d0, d1) } else { (d, d + 1.0) };
            let (a, b) = ((d0 - t) / s, (d1 - t) / s);
            let (a, b) = (a.min(b), a.max(b));
            let total = b - a;
            let clamp = |u: f64| u.clamp(0.0, last as f64) as usize;
            let (first, end) = (clamp(a.floor()), clamp(b.ceil() - 1.0));
            let mut weights = vec![0.0_f32; end - first + 1];
            // What lies before the image goes to its first pixel and past it to its last, each
            // in one share; each image pixel in between takes its own overlap.
            let mut add = |at: usize, w: f64| {
                if let Some(slot) = at.checked_sub(first).and_then(|k| weights.get_mut(k)) {
                    *slot += (w.max(0.0) / total) as f32;
                }
            };
            add(first, b.min(0.0) - a);
            add(end, b - a.max(len as f64));
            let (lo, hi) = (a.max(0.0), b.min(len as f64));
            let mut u = lo.floor();
            while u < hi {
                add(u as usize, hi.min(u + 1.0) - lo.max(u));
                u += 1.0;
            }
            Span { first, weights }
        })
        .collect();
    Some(spans)
}
