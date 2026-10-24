//! A dot grid on the page, under its layers (the board canvas): square dots on the device-pixel
//! grid, blended into the image right after the page colour so the layers cover them. Each dot's
//! pixels are computed from its page position alone, so bands and partial renders line up. Nothing
//! is allocated per dot.

use figcraft_color::Color;
use tiny_skia::Pixmap;

use crate::View;

/// Dots every `step` page units from the page origin, `size` page units square (from one to 64
/// device pixels): those on every second line both ways in `color`, the others (the ones a grid
/// twice as coarse leaves out) in `faded`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dots {
    pub step: f64,
    pub size: f64,
    pub color: Color,
    pub faded: Color,
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
    // Each dot on the device pixel nearest its point, from the view's top-left in device pixels.
    let (ox, oy) = (x0 * s, y0 * s);
    let side = ((d.size * s).round() as i64).clamp(1, 64);
    let lead = side / 2;
    let (strong, faded) = (premultiplied(d.color), premultiplied(d.faded));
    let row = w as usize * 4;
    let data = pix.data_mut();
    for j in j0 as i64..=j1 as i64 {
        let top = ((j as f64 * d.step * s - oy).round() as i64).saturating_sub(lead);
        let (ya, yb) = (top.max(0), top.saturating_add(side).min(h));
        if ya >= yb {
            continue;
        }
        for i in i0 as i64..=i1 as i64 {
            let left = ((i as f64 * d.step * s - ox).round() as i64).saturating_sub(lead);
            let (xa, xb) = (left.max(0), left.saturating_add(side).min(w));
            if xa >= xb {
                continue;
            }
            let rgba = if i % 2 == 0 && j % 2 == 0 { strong } else { faded };
            blend_square(data, row, (xa, xb), (ya, yb), rgba);
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
            for (dst, src) in px.iter_mut().zip(rgba) {
                *dst = (src + u32::from(*dst) * keep / 255).min(255) as u8;
            }
        }
    }
}
