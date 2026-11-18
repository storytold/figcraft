//! Raster effects on premultiplied RGBA8 pixmaps: blur (3× box ≈ Gaussian), dilate/erode (spread),
//! alpha extraction and tinting.

use tiny_skia::Pixmap;

/// Box radii for three passes approximating a Gaussian of `sigma` (Kovesi).
fn box_sizes(sigma: f64) -> [usize; 3] {
    let n = 3.0;
    let w_ideal = (12.0 * sigma * sigma / n + 1.0).sqrt();
    let mut wl = w_ideal.floor() as i64;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wu = wl + 2;
    let m_ideal = (12.0 * sigma * sigma - n * (wl * wl) as f64 - 4.0 * n * wl as f64 - 3.0 * n) / (-4.0 * wl as f64 - 4.0);
    let m = m_ideal.round() as i64;
    let r = |i: i64| -> usize {
        let w = if i < m { wl } else { wu };
        ((w.max(1) - 1) / 2) as usize
    };
    [r(0), r(1), r(2)]
}

/// Pixmaps from this many pixels blur and transpose in parallel (desktop).
#[cfg(not(target_arch = "wasm32"))]
const PARALLEL_PIXELS: usize = 1 << 16;

/// Whether `pixels` go across rayon: large pixmaps, and only from outside its pool. A blur on a
/// pool thread (a band of `render_view`) is already one of several running in parallel.
#[cfg(not(target_arch = "wasm32"))]
fn parallel(pixels: usize) -> bool {
    pixels >= PARALLEL_PIXELS && rayon::current_thread_index().is_none()
}

/// Rows (or transposed columns) per parallel job.
const ROWS_PER_JOB: usize = 16;

/// Run `job` on matching chunks of ROWS_PER_JOB rows of `src` and `dst` (`w` pixels each), in
/// parallel for large pixmaps on desktop (see [`parallel`]).
fn row_jobs(src: &[[u8; 4]], dst: &mut [[u8; 4]], w: usize, job: impl Fn(&[[u8; 4]], &mut [[u8; 4]]) + Sync + Send) {
    let n = w.saturating_mul(ROWS_PER_JOB).max(1);
    #[cfg(not(target_arch = "wasm32"))]
    if parallel(dst.len()) {
        use rayon::prelude::*;
        src.par_chunks(n).zip(dst.par_chunks_mut(n)).for_each(|(s, d)| job(s, d));
        return;
    }
    src.chunks(n).zip(dst.chunks_mut(n)).for_each(|(s, d)| job(s, d));
}

/// One row box-blurred by radius `r` (transparent past its ends) into `dst`, the same length.
fn box_blur_row(src: &[[u8; 4]], dst: &mut [[u8; 4]], r: usize) {
    if r == 0 {
        dst.copy_from_slice(src);
        return;
    }
    let div = (2 * r + 1) as u32;
    let mut acc = [0u32; 4];
    for p in src.iter().take(r + 1) {
        for (a, c) in acc.iter_mut().zip(p) {
            *a += u32::from(*c);
        }
    }
    for (x, d) in dst.iter_mut().enumerate() {
        *d = acc.map(|a| (a / div) as u8);
        if let Some(p) = src.get(x + r + 1) {
            for (a, c) in acc.iter_mut().zip(p) {
                *a += u32::from(*c);
            }
        }
        if let Some(p) = x.checked_sub(r).and_then(|i| src.get(i)) {
            for (a, c) in acc.iter_mut().zip(p) {
                *a = a.saturating_sub(u32::from(*c));
            }
        }
    }
}

/// Box-blur each `w`-pixel row of `src` by each of `radii` in turn into `dst`. A row is blurred
/// through all passes while it is in cache; a transparent row stays transparent.
fn box_blur_rows(src: &[[u8; 4]], dst: &mut [[u8; 4]], w: usize, radii: [usize; 3]) {
    if w == 0 {
        return;
    }
    row_jobs(src, dst, w, |s, d| {
        let mut t = vec![[0u8; 4]; w];
        for (s, d) in s.chunks(w).zip(d.chunks_mut(w)) {
            if s.iter().all(|p| *p == [0; 4]) {
                d.fill([0; 4]);
                continue;
            }
            let t = t.get_mut(..s.len()).unwrap_or_default();
            box_blur_row(s, d, radii[0]);
            box_blur_row(d, t, radii[1]);
            box_blur_row(t, d, radii[2]);
        }
    });
}

/// `src` (`w` × `h` pixels) transposed into `dst` (`h` × `w`), a block of columns at a time.
fn transpose(src: &[[u8; 4]], dst: &mut [[u8; 4]], w: usize, h: usize) {
    if w == 0 || h == 0 {
        return;
    }
    // Job `i` writes rows `x0..` of `dst` (ROWS_PER_JOB of them): columns `x0..` of `src`.
    let job = |(i, d): (usize, &mut [[u8; 4]])| {
        let (x0, cols) = (i * ROWS_PER_JOB, d.len() / h);
        for (y, row) in src.chunks(w).enumerate() {
            let Some(part) = row.get(x0..x0 + cols) else { continue };
            for (k, p) in part.iter().enumerate() {
                if let Some(o) = d.get_mut(k * h + y) {
                    *o = *p;
                }
            }
        }
    };
    let n = h * ROWS_PER_JOB;
    #[cfg(not(target_arch = "wasm32"))]
    if parallel(dst.len()) {
        use rayon::prelude::*;
        dst.par_chunks_mut(n).enumerate().for_each(job);
        return;
    }
    dst.chunks_mut(n).enumerate().for_each(job);
}

/// The largest blur standard deviation drawn (pixels): far wider than any layer (at most 16,384
/// pixels a side), so wider blurs look the same (everything averages out to nothing). The box
/// passes cost the same at any radius, so the documented blur radii (up to 10,000 units) draw
/// in full at any zoom.
pub const MAX_SIGMA: f64 = 1_000_000.0;

/// Gaussian-like blur of the whole pixmap with standard deviation `sigma` (pixels): three box
/// passes along the rows, then three down the columns (as rows of the transposed pixmap).
pub fn blur(p: &mut Pixmap, sigma: f64) {
    if !(sigma.is_finite() && sigma > 0.3) {
        return;
    }
    let sigma = sigma.min(MAX_SIGMA);
    let (w, h) = (p.width() as usize, p.height() as usize);
    let radii = box_sizes(sigma);
    let n = p.data().len() / 4;
    let mut a = vec![[0u8; 4]; n];
    let mut b = vec![[0u8; 4]; n];
    box_blur_rows(p.data().as_chunks::<4>().0, &mut a, w, radii);
    transpose(&a, &mut b, w, h);
    box_blur_rows(&b, &mut a, h, radii);
    transpose(&a, p.data_mut().as_chunks_mut::<4>().0, h, w);
}

/// For each `x`, the largest (`grow`) or smallest value of `src` over `x - r ..= x + r` (clipped
/// to its ends) into `out`, in constant time per value whatever `r` (van Herk / Gil-Werman: the
/// window spans at most two blocks of `2r + 1`, from the running extremes within each block).
/// `scratch` holds the working lines.
fn window_extremes(src: &[u8], out: &mut [u8], r: usize, grow: bool, scratch: &mut [Vec<u8>; 3]) {
    let op = |a: u8, b: u8| if grow { a.max(b) } else { a.min(b) };
    // Past the ends: the value that changes nothing (the window is clipped there).
    let id = if grow { 0 } else { 255 };
    let [pad, fwd, back] = scratch;
    pad.clear();
    pad.resize(r, id);
    pad.extend_from_slice(src);
    pad.resize(src.len() + 2 * r, id);
    let k = 2 * r + 1;
    fwd.clear();
    for block in pad.chunks(k) {
        let mut m = id;
        fwd.extend(block.iter().map(|v| {
            m = op(m, *v);
            m
        }));
    }
    back.clear();
    back.resize(pad.len(), id);
    for (block, b) in pad.chunks(k).zip(back.chunks_mut(k)) {
        let mut m = id;
        for (v, o) in block.iter().zip(b.iter_mut()).rev() {
            m = op(m, *v);
            *o = m;
        }
    }
    for (x, o) in out.iter_mut().enumerate() {
        *o = op(back.get(x).copied().unwrap_or(id), fwd.get(x + 2 * r).copied().unwrap_or(id));
    }
}

/// Grow (`r > 0`) or shrink (`r < 0`) the alpha of a pixmap by `r` pixels (square structuring
/// element, separable max/min). Colour channels follow alpha (tinted afterwards anyway).
pub fn morph(p: &mut Pixmap, r: i32) {
    if r == 0 {
        return;
    }
    let grow = r > 0;
    let r = r.unsigned_abs().min(400) as usize;
    let (w, h) = (p.width() as usize, p.height() as usize);
    if w == 0 || h == 0 {
        return;
    }
    let alpha: Vec<u8> = p.data().as_chunks::<4>().0.iter().map(|c| c[3]).collect();
    let mut scratch: [Vec<u8>; 3] = Default::default();
    // Shrinking treats outside as empty: the first and last `r` of each line go.
    let edges = |line: &mut [u8]| {
        if !grow {
            let n = line.len();
            for (i, v) in line.iter_mut().enumerate() {
                if i < r || i + r >= n {
                    *v = 0;
                }
            }
        }
    };
    let mut a1 = vec![0u8; alpha.len()];
    for (s, o) in alpha.chunks(w).zip(a1.chunks_mut(w)) {
        window_extremes(s, o, r, grow, &mut scratch);
        edges(o);
    }
    // Down the columns.
    let (mut col, mut out) = (vec![0u8; h], vec![0u8; h]);
    let mut a2 = vec![0u8; a1.len()];
    for x in 0..w {
        for (c, v) in col.iter_mut().zip(a1.iter().skip(x).step_by(w)) {
            *c = *v;
        }
        window_extremes(&col, &mut out, r, grow, &mut scratch);
        edges(&mut out);
        for (d, v) in a2.iter_mut().skip(x).step_by(w).zip(&out) {
            *d = *v;
        }
    }
    for (px, a) in p.data_mut().as_chunks_mut::<4>().0.iter_mut().zip(a2) {
        px.copy_from_slice(&[a, a, a, a]);
    }
}

/// Replace every pixel with `color` (non-premultiplied RGBA 0..1) at the pixel's alpha × color alpha.
pub fn tint(p: &mut Pixmap, color: [f32; 4]) {
    let [r, g, b, a] = color;
    for px in p.data_mut().as_chunks_mut::<4>().0 {
        let alpha = f32::from(px.get(3).copied().unwrap_or(0)) / 255.0 * a;
        let q = |c: f32| (c * alpha * 255.0).round().clamp(0.0, 255.0) as u8;
        px.copy_from_slice(&[q(r), q(g), q(b), (alpha * 255.0).round().clamp(0.0, 255.0) as u8]);
    }
}

/// Invert the alpha (premultiplied white with alpha 1-a): used for inner shadows.
pub fn invert_alpha(p: &mut Pixmap) {
    for px in p.data_mut().as_chunks_mut::<4>().0 {
        let a = 255 - px.get(3).copied().unwrap_or(0);
        px.copy_from_slice(&[a, a, a, a]);
    }
}

/// Multiply every pixel of `p` by the alpha of `m` (same size); `invert` uses 1-alpha.
pub fn mask_by(p: &mut Pixmap, m: &Pixmap, invert: bool) {
    for (px, mp) in p.data_mut().as_chunks_mut::<4>().0.iter_mut().zip(m.data().chunks_exact(4)) {
        let ma = u32::from(mp.get(3).copied().unwrap_or(0));
        let ma = if invert { 255 - ma } else { ma };
        for c in px.iter_mut() {
            *c = ((u32::from(*c) * ma + 127) / 255) as u8;
        }
    }
}

/// Call `f(dst_row, src_row, row, first_col)` for each row where `src`, with its top-left at `at`
/// on `dst`, overlaps it (premultiplied RGBA bytes of the overlapping columns; `row` and
/// `first_col` in `src`'s pixels). Nothing past either edge is visited.
pub(crate) fn overlap_rows(dst: &mut Pixmap, at: (i32, i32), src: &Pixmap, mut f: impl FnMut(&mut [u8], &[u8], i64, i64)) {
    let (dw, dh) = (i64::from(dst.width()), i64::from(dst.height()));
    let (sw, sh) = (i64::from(src.width()), i64::from(src.height()));
    let (ax, ay) = (i64::from(at.0), i64::from(at.1));
    let (c0, c1) = ((-ax).max(0), sw.min(dw - ax));
    let (r0, r1) = ((-ay).max(0), sh.min(dh - ay));
    if c1 <= c0 || r1 <= r0 {
        return;
    }
    let len = (c1 - c0) as usize * 4;
    let s = src.data();
    let d = dst.data_mut();
    for r in r0..r1 {
        let (si, di) = ((r * sw + c0) as usize * 4, ((r + ay) * dw + ax + c0) as usize * 4);
        if let (Some(sp), Some(dp)) = (s.get(si..si + len), d.get_mut(di..di + len)) {
            f(dp, sp, r, c0);
        }
    }
}

/// Composite `src` at `opacity` onto `dst` (top-left at `at`) with plus-darker (linear burn:
/// `max(0, backdrop + source - 1)`, through the W3C general compositing formula), which
/// tiny-skia lacks.
pub fn plus_darker(dst: &mut Pixmap, at: (i32, i32), src: &Pixmap, opacity: f32) {
    let o = if opacity.is_finite() { opacity.clamp(0.0, 1.0) } else { 0.0 };
    overlap_rows(dst, at, src, |dp, sp, _, _| {
        for (d, s) in dp.chunks_exact_mut(4).zip(sp.chunks_exact(4)) {
            let f = |v: u8| f32::from(v) / 255.0;
            let sa = f(s[3]) * o;
            if sa <= 0.0 {
                continue;
            }
            let da = f(d[3]);
            for c in 0..3 {
                let (sc, dc) = (f(s[c]) * o, f(d[c]));
                let v = sc * (1.0 - da) + dc * (1.0 - sa) + (sa * dc + da * sc - sa * da).max(0.0);
                d[c] = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            d[3] = ((sa + da - sa * da).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The blur as first written (one box pass at a time over the whole pixmap, bounds-checked
    /// byte by byte): the faster one must give the same bytes.
    fn reference_blur(p: &mut Pixmap, sigma: f64) {
        fn box_blur_h(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize) {
            if r == 0 {
                dst.copy_from_slice(src);
                return;
            }
            let div = (2 * r + 1) as u32;
            for y in 0..h {
                let row = y * w * 4;
                let mut acc = [0u32; 4];
                for x in 0..=r.min(w.saturating_sub(1)) {
                    for c in 0..4 {
                        acc[c] += u32::from(src.get(row + x * 4 + c).copied().unwrap_or(0));
                    }
                }
                for x in 0..w {
                    for c in 0..4 {
                        if let Some(d) = dst.get_mut(row + x * 4 + c) {
                            *d = (acc[c] / div) as u8;
                        }
                    }
                    let add = x + r + 1;
                    if add < w {
                        for c in 0..4 {
                            acc[c] += u32::from(src.get(row + add * 4 + c).copied().unwrap_or(0));
                        }
                    }
                    if x >= r {
                        let sub = x - r;
                        for c in 0..4 {
                            acc[c] = acc[c].saturating_sub(u32::from(src.get(row + sub * 4 + c).copied().unwrap_or(0)));
                        }
                    }
                }
            }
        }
        fn transpose(src: &[u8], dst: &mut [u8], w: usize, h: usize) {
            for y in 0..h {
                for x in 0..w {
                    let (s, d) = ((y * w + x) * 4, (x * h + y) * 4);
                    dst[d..d + 4].copy_from_slice(&src[s..s + 4]);
                }
            }
        }
        if !(sigma.is_finite() && sigma > 0.3) {
            return;
        }
        let (w, h) = (p.width() as usize, p.height() as usize);
        let radii = box_sizes(sigma.min(MAX_SIGMA));
        let mut a = p.data().to_vec();
        let mut b = vec![0u8; a.len()];
        for r in radii {
            box_blur_h(&a, &mut b, w, h, r);
            std::mem::swap(&mut a, &mut b);
        }
        transpose(&a, &mut b, w, h);
        std::mem::swap(&mut a, &mut b);
        for r in radii {
            box_blur_h(&a, &mut b, h, w, r);
            std::mem::swap(&mut a, &mut b);
        }
        transpose(&a, &mut b, h, w);
        p.data_mut().copy_from_slice(&b);
    }

    /// The spread as first written (every pixel looks at its whole window).
    fn reference_morph(p: &mut Pixmap, r: i32) {
        if r == 0 {
            return;
        }
        let grow = r > 0;
        let r = r.unsigned_abs().min(400) as usize;
        let (w, h) = (p.width() as usize, p.height() as usize);
        let alpha: Vec<u8> = p.data().chunks(4).map(|c| c[3]).collect();
        let pick = |v: u8, s: u8| if grow { v.max(s) } else { v.min(s) };
        let mut a1 = vec![0u8; alpha.len()];
        for y in 0..h {
            for x in 0..w {
                let mut v: u8 = if grow { 0 } else { 255 };
                for k in x.saturating_sub(r)..=(x + r).min(w - 1) {
                    v = pick(v, alpha[y * w + k]);
                }
                a1[y * w + x] = if !grow && (x < r || x + r >= w) { 0 } else { v };
            }
        }
        let mut a2 = vec![0u8; a1.len()];
        for x in 0..w {
            for y in 0..h {
                let mut v: u8 = if grow { 0 } else { 255 };
                for k in y.saturating_sub(r)..=(y + r).min(h - 1) {
                    v = pick(v, a1[k * w + x]);
                }
                a2[y * w + x] = if !grow && (y < r || y + r >= h) { 0 } else { v };
            }
        }
        for (px, a) in p.data_mut().chunks_mut(4).zip(a2) {
            px.copy_from_slice(&[a, a, a, a]);
        }
    }

    /// A `w` × `h` pixmap of premultiplied noise in a few rectangles, transparent elsewhere (whole
    /// transparent rows and columns, as layers have around their content).
    fn noise(w: u32, h: u32, seed: u32) -> Pixmap {
        let mut p = Pixmap::new(w, h).unwrap();
        let mut s = seed.wrapping_mul(2_654_435_761).max(1);
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            s
        };
        let (wu, hu) = (w as usize, h as usize);
        for (x0, y0, x1, y1) in [(wu / 8, hu / 6, wu / 2, hu / 2), (wu / 3, hu / 2, wu.saturating_sub(3), hu * 5 / 6), (0, 0, 2, hu)] {
            for y in y0..y1.min(hu) {
                for x in x0..x1.min(wu) {
                    let a = (next() % 256) as u8;
                    let c = |v: u32| ((v % 256) * u32::from(a) / 255) as u8;
                    let i = (y * wu + x) * 4;
                    p.data_mut()[i..i + 4].copy_from_slice(&[c(next()), c(next()), c(next()), a]);
                }
            }
        }
        p
    }

    #[test]
    fn blur_matches_the_reference_bytes() {
        // Small (one thread), wide, tall and large enough to blur in parallel.
        for (i, (w, h)) in [(1, 1), (7, 3), (40, 31), (301, 17), (13, 290), (300, 230)].into_iter().enumerate() {
            for sigma in [0.2, 0.5, 1.0, 2.5, 7.3, 24.0, 160.0, 5000.0] {
                let mut fast = noise(w, h, i as u32 + 1);
                let mut slow = fast.clone();
                blur(&mut fast, sigma);
                reference_blur(&mut slow, sigma);
                assert!(fast.data() == slow.data(), "{w}×{h} sigma {sigma}");
            }
        }
    }

    /// On a rayon pool thread (a band of a parallel render) the blur stays on that thread and
    /// gives the same bytes.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn blur_on_a_pool_thread_matches_the_reference_bytes() {
        let (fast, slow) = rayon::join(
            || {
                assert!(rayon::current_thread_index().is_some());
                let mut p = noise(300, 230, 3);
                blur(&mut p, 9.5);
                p
            },
            || {
                let mut p = noise(300, 230, 3);
                reference_blur(&mut p, 9.5);
                p
            },
        );
        assert!(fast.data() == slow.data());
    }

    #[test]
    fn morph_matches_the_reference_bytes() {
        for (i, (w, h)) in [(1, 1), (5, 9), (40, 31), (301, 17), (13, 290), (300, 230)].into_iter().enumerate() {
            for r in [-500, -40, -6, -2, -1, 1, 2, 5, 33, 400, 1000] {
                let mut fast = noise(w, h, i as u32 + 7);
                let mut slow = fast.clone();
                morph(&mut fast, r);
                reference_morph(&mut slow, r);
                assert!(fast.data() == slow.data(), "{w}×{h} r {r}");
            }
        }
    }

    #[test]
    fn blur_spreads_and_conserves() {
        let mut p = Pixmap::new(64, 64).unwrap();
        let i = (32 * 64 + 32) * 4;
        p.data_mut()[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
        let before: u32 = p.data().iter().map(|v| u32::from(*v)).sum();
        blur(&mut p, 3.0);
        let after: u32 = p.data().iter().map(|v| u32::from(*v)).sum();
        assert!(p.data()[i + 3] < 255);
        // Integer rounding loses a little.
        assert!(after <= before && after > 0, "{before} {after}");
    }

    #[test]
    fn morph_grows() {
        let mut p = Pixmap::new(20, 20).unwrap();
        let i = (10 * 20 + 10) * 4;
        p.data_mut()[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
        morph(&mut p, 2);
        let count = p.data().as_chunks::<4>().0.iter().filter(|c| c[3] == 255).count();
        assert_eq!(count, 25);
        morph(&mut p, -2);
        let count = p.data().as_chunks::<4>().0.iter().filter(|c| c[3] == 255).count();
        assert_eq!(count, 1);
    }

    #[test]
    fn box_sizes_sane() {
        for s in [0.5, 1.0, 4.0, 50.0] {
            let b = box_sizes(s);
            assert!(b.iter().all(|r| *r < 1000));
        }
    }
}
