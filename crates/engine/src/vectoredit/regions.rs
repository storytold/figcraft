//! Regions in vector edit mode: the closed, filled areas of the edited vector. With the Move tool,
//! the region under the pointer (clear of points and segments) is shown hatched, a press selects
//! it with all its points (Shift adds it, or takes a selected one out) and a drag moves them.
//!
//! Only filled areas count: a vector without a visible fill has no regions to hover or press.
//!
//! Hit testing happens in the vector's local units; clearances are in screen pixels. The hatch is
//! built in screen space: parallel diagonal lines ("/") anchored to the vector's origin (so they
//! move with the shape when the canvas pans and never slide over it), clipped to the region by its
//! own fill rule. Their spacing follows octave folding (see [`hatch_spacing`]): fixed in canvas
//! units within each octave of zoom, so on screen it grows with the zoom and drops back at each
//! step. Rounded
//! points are drawn in, as the fill shows them.

use figcraft_doc::{Document, VectorNetwork, WindingRule};
use figcraft_geom::kurbo::{self, ParamCurveNearest, PathEl};
use figcraft_geom::{Affine, Point, Rect, Shape, Vec2};

use super::{VectorEdit, network};

/// A point this close to the pointer (screen pixels, at any zoom) keeps it off every region. It is
/// tighter than a point hit (6 px): in between, a press takes the region.
pub const REGION_POINT_CLEARANCE: f64 = 3.5;
/// A segment this close to the pointer (screen pixels, at any zoom) keeps it off every region,
/// tighter than for a point.
pub const REGION_SEGMENT_CLEARANCE: f64 = 2.0;
/// The distance between neighbouring hatch lines at 100% (and every power-of-two zoom), screen
/// pixels; see [`hatch_spacing`].
pub const HATCH_GAP: f64 = 8.0;
/// The direction hatch lines run on screen (y down): 45°, bottom left to top right ("/").
pub const HATCH_DIRECTION: Vec2 = Vec2::new(std::f64::consts::FRAC_1_SQRT_2, -std::f64::consts::FRAC_1_SQRT_2);
/// Curves are flattened this close (screen pixels) to clip the hatch.
const FLATTEN_PX: f64 = 0.25;
/// At most this many hatch lines per region (the box is clipped to the view first).
const MAX_LINES: f64 = 20_000.0;

/// Where `zoom` can be used: a positive, finite number.
fn usable(zoom: f64) -> bool {
    zoom.is_finite() && zoom > 0.0
}

/// The on-screen distance between hatch lines at `zoom`, by octave folding: the zoom is folded
/// back into one octave around its nearest power of two, `HATCH_GAP * zoom / 2^round(log2(zoom))`. It is [`HATCH_GAP`] at every
/// power-of-two zoom, grows linearly with the zoom up to √2 times that and drops to 1/√2 times it
/// at each step (√2 · 2ⁿ). `None` for an unusable zoom.
pub fn hatch_spacing(zoom: f64) -> Option<f64> {
    if !usable(zoom) {
        return None;
    }
    let step = zoom.log2().round().clamp(-1000.0, 1000.0);
    let s = HATCH_GAP * zoom / step.exp2();
    (s.is_finite() && s > 0.0).then_some(s)
}

/// Whether vector `n` shows a fill (only filled areas are regions).
fn filled(n: &figcraft_doc::Node) -> bool {
    n.fills.iter().any(|f| f.visible)
}

/// The network as its fill draws it: rounded points drawn in (region indexes are kept).
fn drawn(net: &VectorNetwork) -> std::borrow::Cow<'_, VectorNetwork> {
    match net.rounded() {
        Some(r) => std::borrow::Cow::Owned(r),
        None => std::borrow::Cow::Borrowed(net),
    }
}

/// The region of the active layer under page point `p` at `zoom`: none when a point is within
/// [`REGION_POINT_CLEARANCE`] or a segment within [`REGION_SEGMENT_CLEARANCE`] screen pixels, or
/// outside every region; the last region listed wins where regions overlap.
pub fn region_hit(doc: &Document, ve: &VectorEdit, p: Point, zoom: f64) -> Option<usize> {
    if !usable(zoom) {
        return None;
    }
    let net = network(doc, ve.id)?;
    if net.regions.is_empty() || !doc.node(ve.id).is_some_and(filled) {
        return None;
    }
    let w = doc.world_transform(ve.id);
    let lp = w.inverse() * p;
    if !(lp.x.is_finite() && lp.y.is_finite()) {
        return None;
    }
    // Screen pixels → local units (approximate for non-uniform scale, as point hits are).
    let k = 1.0 / zoom / w.determinant().abs().sqrt().max(1e-9);
    let (pc, sc) = (REGION_POINT_CLEARANCE * k, REGION_SEGMENT_CLEARANCE * k);
    if net.vertices.iter().any(|v| (v.point() - lp).hypot() <= pc) {
        return None;
    }
    if (0..net.segments.len()).filter_map(|i| net.cubic(i)).any(|c| c.nearest(lp, 0.01).distance_sq.sqrt() <= sc) {
        return None;
    }
    drawn(net).region_at(lp)
}

/// The regions to hatch: the selected ones and the hovered one (not while a drag is on), each once.
pub fn hatched_regions(ve: &VectorEdit, dragging: bool) -> Vec<usize> {
    let mut out = ve.regions.clone();
    if let Some(h) = ve.region_hover.filter(|_| !dragging) {
        out.push(h);
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// One unclipped hatch line: its index (its offset from the anchor is `k` spacings) and two ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HatchLine {
    pub k: i64,
    pub a: Point,
    pub b: Point,
}

/// The unit normal of the hatch lines running along `d`.
fn normal(d: Vec2) -> Vec2 {
    Vec2::new(-d.y, d.x)
}

/// Hatch lines covering box `b`: every line `{X : (X − o)·n = k · spacing}` (`n` the normal of
/// direction `d`) that crosses it, each long enough to cross the whole box. Empty for a bad box,
/// direction or spacing, or when there would be too many lines.
pub fn hatch_lines(b: Rect, o: Point, d: Vec2, spacing: f64) -> Vec<HatchLine> {
    let len = d.hypot();
    let finite = [b.x0, b.y0, b.x1, b.y1, o.x, o.y, d.x, d.y, spacing].iter().all(|v| v.is_finite());
    if !finite || spacing <= 0.0 || len <= 1e-12 || b.width() < 0.0 || b.height() < 0.0 {
        return Vec::new();
    }
    let d = d / len;
    let n = normal(d);
    let corners = [Point::new(b.x0, b.y0), Point::new(b.x1, b.y0), Point::new(b.x1, b.y1), Point::new(b.x0, b.y1)];
    let proj = corners.map(|c| (c - o).dot(n));
    let lo = proj.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = proj.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let (first, last) = ((lo / spacing).floor(), (hi / spacing).ceil());
    if !(first.is_finite() && last.is_finite()) || last - first > MAX_LINES {
        return Vec::new();
    }
    let m = b.center();
    let reach = (b.width().hypot(b.height())).max(spacing);
    let mut out = Vec::new();
    let mut k = first;
    while k <= last {
        let q = m + n * (k * spacing - (m - o).dot(n));
        out.push(HatchLine { k: k as i64, a: q - d * reach, b: q + d * reach });
        k += 1.0;
    }
    out
}

/// The parts of segment `a`–`b` inside closed polygons `polys` by fill rule `rule`. The segment's
/// ends must lie outside the polygons (hatch lines reach past the area's box).
pub fn clip_to_area(polys: &[Vec<Point>], rule: WindingRule, a: Point, b: Point) -> Vec<[Point; 2]> {
    let u = b - a;
    let uu = u.hypot2();
    if !uu.is_finite() || uu <= 1e-18 {
        return Vec::new();
    }
    let n = normal(u / uu.sqrt());
    // Where each polygon edge crosses the line, and which way (the winding change).
    let mut xs: Vec<(f64, i32)> = Vec::new();
    for poly in polys {
        let count = poly.len();
        for i in 0..count {
            let (Some(p), Some(q)) = (poly.get(i), poly.get((i + 1) % count)) else { continue };
            let (sp, sq) = ((*p - a).dot(n), (*q - a).dot(n));
            let dir = match (sp <= 0.0, sq <= 0.0) {
                (true, false) => 1,
                (false, true) => -1,
                _ => continue,
            };
            let x = *p + (*q - *p) * (sp / (sp - sq));
            let t = (x - a).dot(u) / uu;
            if t.is_finite() {
                xs.push((t, dir));
            }
        }
    }
    xs.sort_by(|x, y| x.0.total_cmp(&y.0));
    let inside = |w: i32| match rule {
        WindingRule::NonZero => w != 0,
        WindingRule::EvenOdd => w % 2 != 0,
    };
    let mut out = Vec::new();
    let (mut w, mut from) = (0, 0.0_f64);
    for (t, dir) in xs {
        if inside(w) {
            let (t0, t1) = (from.clamp(0.0, 1.0), t.clamp(0.0, 1.0));
            if t1 > t0 {
                out.push([a + u * t0, a + u * t1]);
            }
        }
        w += dir;
        from = t;
    }
    out
}

/// Closed polygons of a path, flattened `tol` close.
fn polygons(path: &kurbo::BezPath, tol: f64) -> Vec<Vec<Point>> {
    let mut polys: Vec<Vec<Point>> = Vec::new();
    kurbo::flatten(path.iter(), tol, |el| match el {
        PathEl::MoveTo(p) => polys.push(vec![p]),
        PathEl::LineTo(p) => {
            if let Some(l) = polys.last_mut() {
                l.push(p);
            }
        }
        _ => {}
    });
    polys.retain(|p| p.len() > 2);
    polys
}

/// The hatch over regions `regions` of the edited vector, as screen-space line segments: `view`
/// maps page to screen and `clip` (screen) limits the lines to what shows. All regions share one
/// pattern (anchored to the vector's origin). Empty when the view's zoom is not a positive finite
/// number.
pub fn region_hatch(doc: &Document, ve: &VectorEdit, regions: &[usize], view: Affine, clip: Option<Rect>) -> Vec<[Point; 2]> {
    let zoom = view.determinant().abs().sqrt();
    let Some(spacing) = hatch_spacing(zoom).filter(|_| !regions.is_empty()) else { return Vec::new() };
    let Some(net) = network(doc, ve.id) else { return Vec::new() };
    let net = drawn(net);
    let m = view * doc.world_transform(ve.id);
    let anchor = m * Point::ZERO;
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for &r in regions {
        if seen.contains(&r) {
            continue;
        }
        seen.push(r);
        let (Some(region), Some(mut path)) = (net.regions.get(r), net.region_path(r)) else { continue };
        path.apply_affine(m);
        let polys = polygons(&path, FLATTEN_PX);
        let full = path.bounding_box();
        let b = clip.map_or(full, |c| full.intersect(c));
        if polys.is_empty() || b.width() <= 0.0 || b.height() <= 0.0 {
            continue;
        }
        // Only the lines crossing what shows, each reaching past the whole area (clipping needs
        // its ends outside).
        let reach = full.width().hypot(full.height());
        for l in hatch_lines(b, anchor, HATCH_DIRECTION, spacing) {
            let mid = l.a.midpoint(l.b);
            let r = reach + (mid - full.center()).hypot() + spacing;
            out.extend(clip_to_area(&polys, region.winding, mid - HATCH_DIRECTION * r, mid + HATCH_DIRECTION * r));
        }
    }
    out
}

impl crate::Session {
    /// Vector edit mode: the hatch to draw over the active layer's selected regions and the hovered
    /// one (not while a drag is on), in screen space (`view`: page → screen; `clip`: the visible
    /// part of the canvas, screen).
    pub fn vector_region_hatch(&self, view: Affine, clip: Option<Rect>) -> Vec<[Point; 2]> {
        let Ok(ds) = self.ds() else { return Vec::new() };
        let Some(ve) = ds.vector_edit.as_ref() else { return Vec::new() };
        region_hatch(&ds.doc, ve, &hatched_regions(ve, self.tool.busy()), view, clip)
    }

    /// The pointer left the canvas: nothing is hovered any more.
    pub fn vector_pointer_left(&mut self) {
        if let Ok(ds) = self.ds_mut()
            && let Some(ve) = ds.vector_edit.as_mut()
            && ve.region_hover.is_some()
        {
            ve.region_hover = None;
            ds.revision += 1;
        }
    }
}
