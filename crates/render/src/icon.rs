//! One SVG icon path rasterised for plugin panel grids: a 24-unit icon box drawn at `px`
//! pixels a side, filled or stroked in one colour (the panel tints each cell with the plugin's
//! colour). Holes follow the canvas rule (non-zero winding); strokes get round caps and joins.

use tiny_skia::{FillRule, LineCap, LineJoin, Paint, Pixmap, Shader, Stroke, Transform};

/// `#RGB`, `#RRGGBB` or `#RRGGBBAA` (the `#` optional) as RGBA bytes.
fn rgba(hex: &str) -> Option<[u8; 4]> {
    let h = hex.trim().strip_prefix('#').unwrap_or(hex.trim());
    let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    let dup = |i: usize| u8::from_str_radix(h.get(i..i + 1)?.repeat(2).as_str(), 16).ok();
    match h.len() {
        3 => Some([dup(0)?, dup(1)?, dup(2)?, 255]),
        6 => Some([byte(0)?, byte(2)?, byte(4)?, 255]),
        8 => Some([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => None,
    }
}

/// Unpremultiply one pixel (egui keeps its images unpremultiplied).
fn unpre(r: u8, g: u8, b: u8, a: u8) -> [u8; 4] {
    if a == 0 || a == 255 {
        return [r, g, b, a];
    }
    let up = |c: u8| ((u16::from(c) * 255 + u16::from(a) / 2) / u16::from(a)).min(255) as u8;
    [up(r), up(g), up(b), a]
}

/// Rasterize SVG path data (`d`, drawn in a 24-unit box) to `px` by `px` RGBA bytes
/// (unpremultiplied rows): stroked `stroke` (colour, 24-unit width) when given, else filled
/// `fill`. `None` for bad path data, a bad colour, or a silly size.
pub fn rasterize_icon(d: &str, fill: Option<&str>, stroke: Option<(&str, f64)>, px: u32) -> Option<Vec<u8>> {
    if !(8..=256).contains(&px) || d.trim().is_empty() || d.len() > 16 * 1024 {
        return None;
    }
    let path = figcraft_geom::BezPath::from_svg(d).ok()?;
    let path = crate::draw::ts_path(&path)?;
    let scale = f64::from(px) / 24.0;
    let mut pix = Pixmap::new(px, px)?;
    let xf = Transform::from_scale(scale as f32, scale as f32);
    match (stroke, fill.and_then(rgba)) {
        (Some((color, width)), _) => {
            let [r, g, b, a] = rgba(color)?;
            if !width.is_finite() || width <= 0.0 || width > 24.0 {
                return None;
            }
            let paint = Paint {
                shader: Shader::SolidColor(tiny_skia::Color::from_rgba8(r, g, b, a)),
                anti_alias: true,
                ..Paint::default()
            };
            let stroke = Stroke {
                // In user space: the transform scales it to device pixels.
                width: width as f32,
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                ..Stroke::default()
            };
            pix.stroke_path(&path, &paint, &stroke, xf, None);
        }
        (None, Some([r, g, b, a])) => {
            let paint = Paint {
                shader: Shader::SolidColor(tiny_skia::Color::from_rgba8(r, g, b, a)),
                anti_alias: true,
                ..Paint::default()
            };
            pix.fill_path(&path, &paint, FillRule::Winding, xf, None);
        }
        _ => return None,
    }
    let mut out = Vec::with_capacity(pix.data().len());
    for px in pix.pixels() {
        out.extend_from_slice(&unpre(px.red(), px.green(), px.blue(), px.alpha()));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEART: &str = "M12 21C7 16.5 3 13 3 8.8 3 6 5 4 7.5 4c1.7 0 3.3.9 4.5 2.6C13.2 4.9 14.8 4 16.5 4 19 4 21 6 21 8.8c0 4.2-4 7.7-9 12.2Z";

    #[test]
    fn fills_and_strokes_rasterise_to_solid_pixels() {
        let px = 48;
        let filled = rasterize_icon(HEART, Some("#F24822"), None, px).expect("fills");
        assert_eq!(filled.len(), (px * px * 4) as usize);
        let solid = filled.chunks_exact(4).filter(|p| p[3] == 255).count();
        assert!(solid > 100, "a filled heart covers pixels: {solid}");
        let r = filled.chunks_exact(4).find(|p| p[3] == 255).expect("a solid pixel");
        assert_eq!([r[0], r[1], r[2]], [0xF2, 0x48, 0x22]);
        let stroked = rasterize_icon("M4 12L20 12", None, Some(("#0D99FF", 2.0)), px).expect("strokes");
        let blue = stroked.chunks_exact(4).filter(|p| p[3] > 0 && p[2] > 200).count();
        assert!(blue > 10, "a stroked line paints pixels: {blue}");
        // A stroke wins when both are given (an outline icon stays an outline).
        let both = rasterize_icon("M4 12L20 12", Some("#F24822"), Some(("#0D99FF", 2.0)), px).expect("both");
        let red = both.chunks_exact(4).filter(|p| p[3] == 255 && p[0] > 200 && p[2] < 100).count();
        assert_eq!(red, 0, "no fill leaks into a stroked icon");
        // Strokes are user-space thin: a plus of two 14-unit strokes at width 2 covers ~10%.
        let plus = rasterize_icon("M12 5v14M5 12h14", Some("#111111"), Some(("#111111", 2.0)), px).expect("plus");
        let painted = plus.chunks_exact(4).filter(|p| p[3] > 20).count() as f64 / (px * px) as f64;
        assert!(painted < 0.25, "strokes double-scaled look bold: {painted}");
    }

    #[test]
    fn junk_is_refused() {
        assert!(rasterize_icon("", Some("#000000"), None, 48).is_none());
        assert!(rasterize_icon("M4 12L20 12", Some("#000000"), None, 7).is_none());
        assert!(rasterize_icon("M4 12L20 12", Some("nope"), None, 48).is_none());
        assert!(rasterize_icon("M4 12L20 12", None, None, 48).is_none());
        assert!(rasterize_icon("M4 12L20 12", None, Some(("#000000", 0.0)), 48).is_none());
    }
}
