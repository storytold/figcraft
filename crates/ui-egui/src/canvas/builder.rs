use super::outline::stroke;
use super::overlays::accent;
use super::{Color32, FigApp, Pos2, Shape, Tokens, pos2};

/// Width of the lines hatching the hovered and selected regions in vector edit mode (screen px).
const REGION_HATCH_WIDTH: f32 = 1.4;

/// Vector edit mode: the selected regions and the hovered one hatched in the selection colour,
/// under the segments and points (the engine lays out and clips the lines).
pub(super) fn region_hatch(app: &FigApp, painter: &egui::Painter) {
    let Ok(ds) = app.session.ds() else { return };
    let (ox, oy) = ds.view.page_to_screen(0.0, 0.0);
    let (z, min) = (ds.view.zoom, app.canvas.rect.min);
    let view = figcraft_geom::Affine::new([z, 0.0, 0.0, z, f64::from(min.x) + ox, f64::from(min.y) + oy]);
    let r = painter.clip_rect();
    let clip = figcraft_geom::Rect::new(f64::from(r.min.x), f64::from(r.min.y), f64::from(r.max.x), f64::from(r.max.y));
    for [a, b] in app.session.vector_region_hatch(view, Some(clip)) {
        painter.line_segment([pos2(a.x as f32, a.y as f32), pos2(b.x as f32, b.y as f32)], stroke(accent(painter), REGION_HATCH_WIDTH));
    }
}

/// Shape builder: every region's edges, and the hovered or collected regions hatched (in the
/// danger colour when releasing removes them).
pub(super) fn builder_overlay(app: &FigApp, painter: &egui::Painter, t: &Tokens, arr: &figcraft_engine::shapebuilder::Arrangement) {
    let ov = &app.session.tool.overlay;
    let screen = |r: &figcraft_engine::shapebuilder::Region| -> Vec<Vec<Pos2>> {
        r.polys.iter().filter_map(|poly| clean_poly(poly.iter().map(|p| app.to_screen(p.x, p.y)))).collect()
    };
    // Edges read on any fill: a light halo under a thin accent line.
    for r in &arr.regions {
        for poly in screen(r) {
            for seg in segments(&poly) {
                painter.add(Shape::line(seg.to_vec(), stroke(Color32::from_white_alpha(150), 2.5)));
                painter.add(Shape::line(seg.to_vec(), stroke(accent(painter), 1.0)));
            }
        }
    }
    let col = if ov.builder_remove { t.danger } else { accent(painter) };
    for r in ov.builder_regions.iter().filter_map(|i| arr.regions.get(*i)) {
        hatched(painter, &screen(r), col);
    }
}

/// A region contour in screen space, ready to draw: non-finite and repeated points dropped (the
/// last point back at the first, the points a sliver doubles back on) and sub-pixel slivers left
/// out, since they're invisible anyway.
pub(crate) fn clean_poly(points: impl IntoIterator<Item = Pos2>) -> Option<Vec<Pos2>> {
    let mut out: Vec<Pos2> = Vec::new();
    for q in points {
        if !(q.x.is_finite() && q.y.is_finite()) || out.last().is_some_and(|l| l.distance(q) < 0.01) {
            continue;
        }
        out.push(q);
    }
    if out.len() > 1 && out.first().is_some_and(|f| f.distance(*out.last().unwrap_or(f)) < 0.01) {
        out.pop();
    }
    let n = out.len();
    let twice_area: f32 = (0..n).map(|k| out[k].x * out[(k + 1) % n].y - out[(k + 1) % n].x * out[k].y).sum();
    (n > 2 && twice_area.abs() > 0.4).then_some(out)
}

/// A closed polygon as its segments. Drawn segment by segment rather than as one closed path: a
/// joined path asks the tessellator for a mitre at every hairpin a region's sliver doubles back on,
/// and a zero-length segment there divides by a zero normal, which draws a spike across the canvas.
fn segments(poly: &[Pos2]) -> impl Iterator<Item = [Pos2; 2]> + '_ {
    let n = poly.len();
    (0..n).map(move |k| [poly[k], poly[(k + 1) % n]])
}

/// Polygons hatched and outlined in `col` (an area about to be filled, merged or removed), each
/// line on a light halo so it reads on any fill, even one in `col`.
pub(super) fn hatched(painter: &egui::Painter, polys: &[Vec<Pos2>], col: Color32) {
    let halo = Color32::from_white_alpha(200);
    for l in hatch(polys, 6.0) {
        painter.line_segment(l, stroke(halo, 3.0));
        painter.line_segment(l, stroke(col, 1.25));
    }
    for poly in polys {
        for seg in segments(poly) {
            painter.add(Shape::line(seg.to_vec(), stroke(halo, 4.0)));
            painter.add(Shape::line(seg.to_vec(), stroke(col, 2.0)));
        }
    }
}

/// 45° hatch lines `gap` apart inside polygons (even-odd), in screen space.
fn hatch(polys: &[Vec<Pos2>], gap: f32) -> Vec<[Pos2; 2]> {
    // Rotate so the hatch is horizontal, scan, rotate back.
    let rot = |p: Pos2| pos2((p.x + p.y) * std::f32::consts::FRAC_1_SQRT_2, (p.y - p.x) * std::f32::consts::FRAC_1_SQRT_2);
    let back = |x: f32, y: f32| pos2((x - y) * std::f32::consts::FRAC_1_SQRT_2, (x + y) * std::f32::consts::FRAC_1_SQRT_2);
    let edges: Vec<(Pos2, Pos2)> = polys
        .iter()
        .flat_map(|poly| {
            let pts: Vec<Pos2> = poly.iter().map(|p| rot(*p)).collect();
            let n = pts.len();
            (0..n).filter_map(move |k| Some((*pts.get(k)?, *pts.get((k + 1) % n)?)))
        })
        .collect();
    let (y0, y1) = edges.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), (a, b)| (lo.min(a.y).min(b.y), hi.max(a.y).max(b.y)));
    let mut out = Vec::new();
    if !(y0.is_finite() && y1.is_finite()) || (y1 - y0) / gap > 4000.0 {
        return out;
    }
    let mut y = (y0 / gap).ceil() * gap;
    while y < y1 {
        let mut xs: Vec<f32> = edges
            .iter()
            .filter(|(a, b)| (a.y <= y && y < b.y) || (b.y <= y && y < a.y))
            .map(|(a, b)| a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y))
            .collect();
        xs.sort_by(f32::total_cmp);
        for pair in xs.chunks_exact(2) {
            if let [a, b] = pair {
                out.push([back(*a, y), back(*b, y)]);
            }
        }
        y += gap;
    }
    out
}
