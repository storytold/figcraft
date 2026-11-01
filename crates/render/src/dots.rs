//! A grid on the page, under its layers (the board canvas): square dots or lines on the
//! device-pixel grid, blended into the image right after the page colour so the layers cover them.
//! Each mark's pixels are computed from its page position alone, so bands and partial renders line
//! up. Nothing is allocated per mark.

use figcraft_color::Color;
use tiny_skia::Pixmap;

use crate::View;

/// Dots (or lines both ways) every `step` page units from the page origin, `size` page units
/// across (from one to 64 device pixels): those on every `major`th line both ways (every line of
/// the `major`th, for lines) in `color`, the others (the ones a grid `major` times as coarse leaves
/// out) in `faded`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dots {
    pub step: f64,
    pub size: f64,
    pub color: Color,
    pub faded: Color,
    /// Lines instead of dots.
    pub lines: bool,
    /// Every how many steps a mark is strong (2 or more; less reads as 2).
    pub major: u32,
}

/// The most dots one render draws: a step far too small for the view draws none.
const MAX_DOTS: f64 = 4_000_000.0;

/// `c` as premultiplied 8-bit RGBA.
fn premultiplied(c: Color) -> [u32; 4] {
    let [r, g, b, a] = c.to_rgba8().map(u32::from);
    [r * a / 255, g * a / 255, b * a / 255, a]
}

/// Draw `d` into `pix`, the render of `view`.
pub(crate) fn draw(pix: &mut Pixmap, view: &View, d: &Dots) {
    let s = view.scale;
    let ok = |v: f64| v.is_finite() && v > 0.0;
    if !(ok(d.step) && ok(d.size) && ok(s) && view.origin.0.is_finite() && view.origin.1.is_finite()) {
        return;
    }
    let (w, h) = (i64::from(pix.width()), i64::from(pix.height()));
    let (x0, y0) = view.origin;
    let (i0, i1) = ((x0 / d.step).floor() - 1.0, ((x0 + w as f64 / s) / d.step).ceil() + 1.0);
    let (j0, j1) = ((y0 / d.step).floor() - 1.0, ((y0 + h as f64 / s) / d.step).ceil() + 1.0);
    let count = (i1 - i0 + 1.0) * (j1 - j0 + 1.0);
    if !(count.is_finite() && count <= MAX_DOTS) {
        return;
    }
    // Each mark on the device pixel nearest its point, from the view's top-left in device pixels.
    let (ox, oy) = (x0 * s, y0 * s);
    let side = ((d.size * s).round() as i64).clamp(1, 64);
    let lead = side / 2;
    let colors = [premultiplied(d.faded), premultiplied(d.color)];
    let major = i64::from(d.major.max(2));
    let strong = |k: i64| k.rem_euclid(major) == 0;
    // The device-pixel span of the mark on line `k` along an axis `len` pixels long from `o`.
    let span = |k: i64, o: f64, len: i64| {
        let a = ((k as f64 * d.step * s - o).round() as i64).saturating_sub(lead);
        (a.max(0), a.saturating_add(side).min(len))
    };
    let row = w as usize * 4;
    let data = pix.data_mut();
    if d.lines {
        lines(
            data,
            row,
            (w, h),
            [(i0 as i64, i1 as i64), (j0 as i64, j1 as i64)],
            &|k, vertical| {
                let (o, len) = if vertical { (ox, w) } else { (oy, h) };
                (span(k, o, len), strong(k))
            },
            colors,
        );
        return;
    }
    for j in j0 as i64..=j1 as i64 {
        let (ya, yb) = span(j, oy, h);
        if ya >= yb {
            continue;
        }
        for i in i0 as i64..=i1 as i64 {
            let (xa, xb) = span(i, ox, w);
            if xa >= xb {
                continue;
            }
            let rgba = colors[usize::from(strong(i) && strong(j))];
            blend_square(data, row, (xa, xb), (ya, yb), rgba);
        }
    }
}

/// Which line covers each device pixel along an axis: 0 none, 1 a faded one, 2 a strong one.
type Cover = Vec<u8>;

/// Draw lines both ways: the vertical ones `ks[0]`, the horizontal ones `ks[1]`, each line's span
/// and strength from `mark(k, vertical)`. A crossing is blended once, in the stronger colour.
fn lines(
    data: &mut [u8],
    row: usize,
    (w, h): (i64, i64),
    ks: [(i64, i64); 2],
    mark: &dyn Fn(i64, bool) -> ((i64, i64), bool),
    colors: [[u32; 4]; 2],
) {
    let cover = |(k0, k1): (i64, i64), vertical: bool, len: i64| {
        let mut c: Cover = vec![0; len.max(0) as usize];
        for k in k0..=k1 {
            let ((a, b), strong) = mark(k, vertical);
            for v in c.get_mut(a.max(0) as usize..b.max(a).max(0) as usize).into_iter().flatten() {
                *v = (*v).max(1 + u8::from(strong));
            }
        }
        c
    };
    let (cols, rows) = (cover(ks[0], true, w), cover(ks[1], false, h));
    // The columns' runs, so a row without a line only visits the vertical lines.
    let mut runs: Vec<(usize, usize, u8)> = Vec::new();
    for (x, &c) in cols.iter().enumerate() {
        match runs.last_mut() {
            Some(r) if r.1 == x && r.2 == c => r.1 = x + 1,
            _ if c > 0 => runs.push((x, x + 1, c)),
            _ => {}
        }
    }
    for (y, &rc) in rows.iter().enumerate() {
        let start = y * row;
        if rc == 0 {
            for &(a, b, c) in &runs {
                let rgba = colors[usize::from(c.saturating_sub(1).min(1))];
                blend_square(data, row, (a as i64, b as i64), (y as i64, y as i64 + 1), rgba);
            }
            continue;
        }
        let Some(line) = data.get_mut(start..start + w as usize * 4) else { continue };
        for (px, &cc) in line.chunks_exact_mut(4).zip(&cols) {
            blend_px(px, colors[usize::from(rc.max(cc).saturating_sub(1).min(1))]);
        }
    }
}

/// Blend premultiplied `rgba` source-over into the pixels `xs` × `ys` of `data`, rows `row` bytes
/// long.
fn blend_square(data: &mut [u8], row: usize, (xa, xb): (i64, i64), (ya, yb): (i64, i64), rgba: [u32; 4]) {
    let keep = 255 - rgba[3];
    if rgba[3] == 0 {
        return;
    }
    for y in ya..yb {
        let start = y as usize * row;
        let Some(line) = data.get_mut(start + xa as usize * 4..start + xb as usize * 4) else { continue };
        for px in line.chunks_exact_mut(4) {
            blend_with(px, rgba, keep);
        }
    }
}

/// Blend premultiplied `rgba` source-over into the pixel `px`.
fn blend_px(px: &mut [u8], rgba: [u32; 4]) {
    if rgba[3] > 0 {
        blend_with(px, rgba, 255 - rgba[3]);
    }
}

fn blend_with(px: &mut [u8], rgba: [u32; 4], keep: u32) {
    for (dst, src) in px.iter_mut().zip(rgba) {
        *dst = (src + u32::from(*dst) * keep / 255).min(255) as u8;
    }
}
