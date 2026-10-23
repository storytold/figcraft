//! FigCraft geometry.
//!
//! Re-exports [`kurbo`] and adds the shape builders a design tool needs: rectangles with independent
//! corner radii and corner smoothing (the "squircle" continuous-curvature corner), regular polygons
//! and stars (with rounded vertices), and ellipse arcs / rings. Coordinates are y-down.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub use kurbo;
pub use kurbo::{Affine, BezPath, Point, Rect, Shape, Size, Vec2};

use kurbo::ParamCurve;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

pub mod measure;
mod transform;
pub use transform::*;

/// Corner radii in clockwise order from the top-left: `[tl, tr, br, bl]`.
pub type Radii = [f64; 4];

fn finite(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

/// A rectangle path at the origin with per-corner radii and corner smoothing (0..=1, Figma's
/// "Corner smoothing"; 0.6 is the iOS look).
///
/// Radii are clamped so adjacent corners never overlap; when smoothing doesn't fit, it is reduced
/// first, then the radius.
pub fn rounded_rect(width: f64, height: f64, radii: Radii, smoothing: f64) -> BezPath {
    let w = finite(width).max(0.0);
    let h = finite(height).max(0.0);
    let mut path = BezPath::new();
    if w <= 0.0 && h <= 0.0 {
        return path;
    }
    let s = finite(smoothing).clamp(0.0, 1.0);
    let max_r = w.min(h) / 2.0;
    let radii = radii.map(|r| finite(r).clamp(0.0, max_r));
    if radii.iter().all(|r| *r <= 0.0) {
        path.move_to((0.0, 0.0));
        path.line_to((w, 0.0));
        path.line_to((w, h));
        path.line_to((0.0, h));
        path.close_path();
        return path;
    }
    // Corners clockwise starting top-right, each: (corner, incoming edge direction, outgoing).
    let corners: [(Point, Vec2, Vec2, f64); 4] = [
        (Point::new(w, 0.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0), radii[1]),
        (Point::new(w, h), Vec2::new(0.0, 1.0), Vec2::new(-1.0, 0.0), radii[2]),
        (Point::new(0.0, h), Vec2::new(-1.0, 0.0), Vec2::new(0.0, -1.0), radii[3]),
        (Point::new(0.0, 0.0), Vec2::new(0.0, -1.0), Vec2::new(1.0, 0.0), radii[0]),
    ];
    let first = corner_profile(radii[0], s, max_r);
    // Start on the top edge just after the top-left corner.
    path.move_to((first.p, 0.0));
    for (c, din, dout, r) in corners {
        let prof = corner_profile(r, s, max_r);
        append_corner(&mut path, c, din, dout, &prof);
    }
    path.close_path();
    path
}

/// The smoothed-corner profile along one corner (lengths measured from the corner point).
#[derive(Clone, Copy, Debug)]
struct CornerProfile {
    r: f64,
    /// Total length the corner occupies along each edge.
    p: f64,
    a: f64,
    b: f64,
    c: f64,
    d: f64,
}

fn corner_profile(r: f64, smoothing: f64, max_p: f64) -> CornerProfile {
    let r = r.max(0.0);
    if r <= 0.0 {
        return CornerProfile { r: 0.0, p: 0.0, a: 0.0, b: 0.0, c: 0.0, d: 0.0 };
    }
    let mut s = smoothing;
    // Reduce smoothing first when the smoothed corner doesn't fit along the edge.
    if (1.0 + s) * r > max_p {
        s = (max_p / r - 1.0).clamp(0.0, s);
    }
    let r = r.min(max_p);
    let p = ((1.0 + s) * r).min(max_p);
    let arc_measure = 90.0 * (1.0 - s);
    let arc_section_length = (arc_measure / 2.0).to_radians().sin() * r * std::f64::consts::SQRT_2;
    let angle_alpha = (90.0 - arc_measure) / 2.0;
    let p3_to_p4 = r * (angle_alpha / 2.0).to_radians().tan();
    let angle_beta = 45.0 * s;
    let c = p3_to_p4 * angle_beta.to_radians().cos();
    let d = c * angle_beta.to_radians().tan();
    let b = ((p - arc_section_length - c - d) / 3.0).max(0.0);
    let a = 2.0 * b;
    CornerProfile { r, p, a, b, c, d }
}

fn append_corner(path: &mut BezPath, corner: Point, din: Vec2, dout: Vec2, prof: &CornerProfile) {
    // position = corner - back * din + side * dout
    let at = |back: f64, side: f64| corner - din * back + dout * side;
    if prof.r <= 0.0 {
        path.line_to(corner);
        return;
    }
    let CornerProfile { r, p, a, b, c, d } = *prof;
    let start = at(p, 0.0);
    path.line_to(start);
    let p3 = at(p - a - b - c, d);
    if a + b + c > 1e-9 {
        path.curve_to(at(p - a, 0.0), at(p - a - b, 0.0), p3);
    }
    let p4 = at(d, p - a - b - c);
    let center = at(r, r);
    let a0 = (p3 - center).atan2();
    let a1 = (p4 - center).atan2();
    let mut sweep = a1 - a0;
    while sweep <= -PI {
        sweep += TAU;
    }
    while sweep > PI {
        sweep -= TAU;
    }
    if sweep.abs() > 1e-9 {
        let arc = kurbo::Arc::new(center, (r, r), a0, sweep, 0.0);
        arc.to_cubic_beziers(0.01, |p1, p2, p| path.curve_to(p1, p2, p));
    }
    let end = at(0.0, p);
    if a + b + c > 1e-9 {
        path.curve_to(at(0.0, p - a - b), at(0.0, p - a), end);
    } else {
        path.line_to(end);
    }
}

/// The vertices of a regular polygon with `count` sides inscribed in the `width`×`height` box,
/// first vertex at the top centre (Figma's polygon).
pub fn polygon_points(width: f64, height: f64, count: u32) -> Vec<Point> {
    let n = count.clamp(3, 120);
    let (rx, ry) = (finite(width) / 2.0, finite(height) / 2.0);
    (0..n)
        .map(|i| {
            let t = -FRAC_PI_2 + TAU * f64::from(i) / f64::from(n);
            Point::new(rx + rx * t.cos(), ry + ry * t.sin())
        })
        .collect()
}

/// The vertices of a star with `count` points and `inner_ratio` (0..1) inner radius.
pub fn star_points(width: f64, height: f64, count: u32, inner_ratio: f64) -> Vec<Point> {
    let n = count.clamp(3, 120);
    let k = finite(inner_ratio).clamp(0.001, 1.0);
    let (rx, ry) = (finite(width) / 2.0, finite(height) / 2.0);
    (0..n * 2)
        .map(|i| {
            let t = -FRAC_PI_2 + PI * f64::from(i) / f64::from(n);
            let f = if i % 2 == 0 { 1.0 } else { k };
            Point::new(rx + rx * f * t.cos(), ry + ry * f * t.sin())
        })
        .collect()
}

/// A closed polygon through `points` with every vertex rounded by `radius` (clamped so arcs fit).
pub fn rounded_polygon(points: &[Point], radius: f64) -> BezPath {
    let mut path = BezPath::new();
    let n = points.len();
    if n < 2 {
        return path;
    }
    let radius = finite(radius).max(0.0);
    if radius <= 0.0 || n < 3 {
        for (i, p) in points.iter().enumerate() {
            if i == 0 {
                path.move_to(*p);
            } else {
                path.line_to(*p);
            }
        }
        path.close_path();
        return path;
    }
    // For each vertex: the tangent points on its two edges and the bezier between them.
    let mut corners: Vec<(Point, Point, Point, Point)> = Vec::with_capacity(n);
    for i in 0..n {
        let (Some(prev), Some(cur), Some(next)) = (points.get((i + n - 1) % n), points.get(i), points.get((i + 1) % n)) else {
            continue;
        };
        let v1 = *prev - *cur;
        let v2 = *next - *cur;
        let (l1, l2) = (v1.hypot(), v2.hypot());
        if l1 < 1e-9 || l2 < 1e-9 {
            corners.push((*cur, *cur, *cur, *cur));
            continue;
        }
        let (u1, u2) = (v1 / l1, v2 / l2);
        let cos = u1.dot(u2).clamp(-1.0, 1.0);
        let angle = cos.acos();
        if angle < 1e-6 || (PI - angle) < 1e-6 {
            corners.push((*cur, *cur, *cur, *cur));
            continue;
        }
        let mut t = radius / (angle / 2.0).tan();
        let max_t = (l1.min(l2)) / 2.0;
        let mut r = radius;
        if t > max_t {
            r = r * max_t / t;
            t = max_t;
        }
        let a = *cur + u1 * t;
        let b = *cur + u2 * t;
        // Cubic arc approximation: handle length k*r with k = 4/3 tan(theta/4), theta = PI - angle.
        let theta = PI - angle;
        let k = 4.0 / 3.0 * (theta / 4.0).tan() * r;
        let c1 = a - u1 * k;
        let c2 = b - u2 * k;
        corners.push((a, c1, c2, b));
    }
    for (i, (a, c1, c2, b)) in corners.iter().enumerate() {
        if i == 0 {
            path.move_to(*a);
        } else {
            path.line_to(*a);
        }
        path.curve_to(*c1, *c2, *b);
    }
    path.close_path();
    path
}

/// An ellipse in the `width`×`height` box; with arc data, a pie/arc/ring: `start` and `sweep` in
/// radians (clockwise from 3 o'clock, y-down) and `inner` radius ratio 0..1.
pub fn ellipse(width: f64, height: f64, start: f64, sweep: f64, inner: f64) -> BezPath {
    let (rx, ry) = (finite(width).max(0.0) / 2.0, finite(height).max(0.0) / 2.0);
    let c = Point::new(rx, ry);
    let sweep = finite(sweep).clamp(-TAU, TAU);
    let start = finite(start);
    let inner = finite(inner).clamp(0.0, 1.0);
    let full = sweep.abs() >= TAU - 1e-9;
    let mut path = BezPath::new();
    if full && inner <= 0.0 {
        return kurbo::Ellipse::new(c, (rx, ry), 0.0).to_path(0.01);
    }
    if full {
        // A ring: outer clockwise, inner counter-clockwise.
        path.extend(kurbo::Ellipse::new(c, (rx, ry), 0.0).path_elements(0.01));
        let inner_path = kurbo::Ellipse::new(c, (rx * inner, ry * inner), 0.0).to_path(0.01);
        path.extend(inner_path.reverse_subpaths());
        return path;
    }
    let outer = kurbo::Arc::new(c, (rx, ry), start, sweep, 0.0);
    path.move_to(c + Vec2::new(rx * start.cos(), ry * start.sin()));
    outer.to_cubic_beziers(0.01, |p1, p2, p| path.curve_to(p1, p2, p));
    if inner > 0.0 {
        let end = start + sweep;
        path.line_to(c + Vec2::new(rx * inner * end.cos(), ry * inner * end.sin()));
        let back = kurbo::Arc::new(c, (rx * inner, ry * inner), end, -sweep, 0.0);
        back.to_cubic_beziers(0.01, |p1, p2, p| path.curve_to(p1, p2, p));
    } else {
        path.line_to(c);
    }
    path.close_path();
    path
}

/// Trait helper: reverse every subpath of a path.
pub trait ReverseSubpaths {
    fn reverse_subpaths(&self) -> BezPath;
}

impl ReverseSubpaths for BezPath {
    fn reverse_subpaths(&self) -> BezPath {
        let mut out = BezPath::new();
        for seg_path in split_subpaths(self) {
            let segs: Vec<kurbo::PathSeg> = seg_path.segments().collect();
            let Some(last) = segs.last() else { continue };
            out.move_to(last.end());
            for s in segs.iter().rev() {
                match s.reverse() {
                    kurbo::PathSeg::Line(l) => out.line_to(l.p1),
                    kurbo::PathSeg::Quad(q) => out.quad_to(q.p1, q.p2),
                    kurbo::PathSeg::Cubic(c) => out.curve_to(c.p1, c.p2, c.p3),
                }
            }
            if matches!(seg_path.elements().last(), Some(kurbo::PathEl::ClosePath)) {
                out.close_path();
            }
        }
        out
    }
}

/// Split a path into its subpaths.
pub fn split_subpaths(path: &BezPath) -> Vec<BezPath> {
    let mut out = Vec::new();
    let mut cur = BezPath::new();
    for el in path.elements() {
        if let kurbo::PathEl::MoveTo(_) = el
            && !cur.elements().is_empty()
        {
            out.push(std::mem::take(&mut cur));
        }
        cur.push(*el);
    }
    if !cur.elements().is_empty() {
        out.push(cur);
    }
    out
}

/// Bounding box of a set of points (`None` when empty).
pub fn points_bounds(points: impl IntoIterator<Item = Point>) -> Option<Rect> {
    let mut it = points.into_iter();
    let first = it.next()?;
    Some(it.fold(Rect::from_points(first, first), |r, p| r.union_pt(p)))
}

/// The axis-aligned bounds of `rect` after `xf`.
pub fn transform_rect_bbox(xf: Affine, rect: Rect) -> Rect {
    let pts = [Point::new(rect.x0, rect.y0), Point::new(rect.x1, rect.y0), Point::new(rect.x1, rect.y1), Point::new(rect.x0, rect.y1)];
    points_bounds(pts.into_iter().map(|p| xf * p)).unwrap_or(Rect::ZERO)
}

#[cfg(test)]
mod tests;
