//! FigCraft path operations.
//!
//! - [`boolean`]: Union, Subtract, Intersect and Exclude over any number of closed paths (robust
//!   sweep-line topology from `linesweeper`); results use the even-odd rule.
//! - [`stroke_outline`]: a stroke as a fill, with Figma's Inside / Center / Outside alignment,
//!   dashes, caps, joins, arrowheads, variable width profiles ([`width`]) and brushes ([`brush`]).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod brush;
#[cfg(test)]
mod brush_tests;
#[cfg(test)]
mod ops_tests;
mod points;
#[cfg(test)]
mod rounded_side_strokes_tests;
#[cfg(test)]
mod sides_tests;
pub mod width;
#[cfg(test)]
mod width_tests;

use figcraft_doc::{BoolOp, StrokeAlign, StrokeCap, StrokeJoin, StrokeProps, VectorNetwork};
use figcraft_geom::kurbo::{self, BezPath, Cap, Join, PathEl, PathSeg, Point, Shape, Vec2};

#[derive(Debug, PartialEq)]
pub enum PathOpsError {
    /// The inputs contain NaN or infinite coordinates.
    BadInput,
}

impl std::fmt::Display for PathOpsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "path contains invalid coordinates")
    }
}

/// Close every open subpath (a filled open path paints as if closed).
pub fn closed(path: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    let mut open = false;
    for el in path.elements() {
        match el {
            PathEl::MoveTo(_) => {
                if open {
                    out.close_path();
                }
                open = true;
            }
            PathEl::ClosePath => open = false,
            _ => {}
        }
        out.push(*el);
    }
    if open {
        out.close_path();
    }
    out
}

/// Every coordinate is finite and of a sane magnitude (the sweep needs bounded input).
pub use figcraft_geom::measure::finite_path as finite;

/// Run the sweep library on `f`. Version 0.4 can panic on rare near-degenerate input (an internal
/// `unwrap` while ordering curves, hit by some font outlines): that is bad input, not a crash.
// shortcut: no catch where panics abort (wasm32); drop this once a linesweeper release no longer panics there.
fn sweep<T>(f: impl FnOnce() -> Result<T, PathOpsError>) -> Result<T, PathOpsError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(Err(PathOpsError::BadInput))
}

fn binary(a: &BezPath, b: &BezPath, op: linesweeper::BinaryOp) -> Result<BezPath, PathOpsError> {
    if a.elements().is_empty() && b.elements().is_empty() {
        return Ok(BezPath::new());
    }
    if !finite(a) || !finite(b) {
        return Err(PathOpsError::BadInput);
    }
    let contours = sweep(|| linesweeper::binary_op(&closed(a), &closed(b), linesweeper::FillRule::NonZero, op).map_err(|_| PathOpsError::BadInput))?;
    let mut out = BezPath::new();
    for c in contours.contours() {
        out.extend(c.path.elements().iter().copied());
    }
    Ok(out)
}

/// Normalise a single path's winding (non-zero) into simple even-odd contours.
pub fn simplify(path: &BezPath) -> Result<BezPath, PathOpsError> {
    binary(path, &BezPath::new(), linesweeper::BinaryOp::Union)
}

/// Combine `acc` (the operands so far) with the next operand `p` by `op`.
pub fn boolean_step(acc: &BezPath, p: &BezPath, op: BoolOp) -> Result<BezPath, PathOpsError> {
    let op = match op {
        BoolOp::Union => linesweeper::BinaryOp::Union,
        BoolOp::Intersect => linesweeper::BinaryOp::Intersection,
        BoolOp::Exclude => linesweeper::BinaryOp::Xor,
        BoolOp::Subtract => linesweeper::BinaryOp::Difference,
    };
    binary(acc, p, op)
}

/// Apply a boolean operation to `paths` in order (bottom to top, like a boolean group's children):
/// Subtract removes every later shape from the first one.
pub fn boolean(paths: &[BezPath], op: BoolOp) -> Result<BezPath, PathOpsError> {
    boolean_ops(paths.iter().map(|p| (p, op)))
}

/// Fold `operands` bottom to top, each combined with the result of those below it by its own
/// operation (the bottom operand's operation is unused): what a boolean group whose operands carry
/// their own operations paints. A lone operand comes out simplified.
pub fn boolean_ops<'a>(operands: impl IntoIterator<Item = (&'a BezPath, BoolOp)>) -> Result<BezPath, PathOpsError> {
    let mut it = operands.into_iter();
    let Some((first, _)) = it.next() else { return Ok(BezPath::new()) };
    let mut acc: Option<BezPath> = None;
    for (p, op) in it {
        acc = Some(boolean_step(acc.as_ref().unwrap_or(first), p, op)?);
    }
    match acc {
        Some(a) => Ok(a),
        None => simplify(first),
    }
}

/// Per-input winding numbers for [`faces`] (index = input path; trailing zeros trimmed so equal
/// coverage compares equal).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Windings(Vec<i32>);

impl Windings {
    fn trimmed(mut self) -> Self {
        while self.0.last() == Some(&0) {
            self.0.pop();
        }
        self
    }
}

impl std::ops::Add for Windings {
    type Output = Self;
    fn add(mut self, rhs: Self) -> Self {
        self += rhs;
        self
    }
}

impl std::ops::AddAssign for Windings {
    fn add_assign(&mut self, rhs: Self) {
        if self.0.len() < rhs.0.len() {
            self.0.resize(rhs.0.len(), 0);
        }
        for (a, b) in self.0.iter_mut().zip(rhs.0) {
            *a += b;
        }
        *self = std::mem::take(self).trimmed();
    }
}

impl linesweeper::topology::WindingNumber for Windings {
    type Tag = usize;
    fn single(tag: usize, positive: bool) -> Self {
        let mut v = vec![0; tag + 1];
        if let Some(w) = v.last_mut() {
            *w = if positive { 1 } else { -1 };
        }
        Self(v)
    }
    fn of_tag(&self, tag: usize) -> Self {
        let mut v = vec![0; tag + 1];
        if let (Some(w), Some(src)) = (v.last_mut(), self.0.get(tag)) {
            *w = *src;
        }
        Self(v).trimmed()
    }
}

/// One face of the arrangement of several closed paths: a connected area that the same set of
/// inputs covers.
#[derive(Clone, Debug)]
pub struct ArrangementFace {
    /// The face's outer contour and its holes (fill with either rule).
    pub path: BezPath,
    /// The inputs covering the face, by index, ascending.
    pub cover: Vec<usize>,
}

/// The most inputs [`faces`] accepts, and the most faces it returns.
pub const MAX_ARRANGEMENT_INPUTS: usize = 256;
const MAX_FACES: usize = 4096;

/// Split the plane by the outlines of `paths` (each filled with its own rule: `true` = even-odd)
/// into faces: the connected areas covered by the same inputs. Areas no input covers are left
/// out. Open subpaths are closed first.
pub fn faces(paths: &[(BezPath, bool)]) -> Result<Vec<ArrangementFace>, PathOpsError> {
    if paths.len() > MAX_ARRANGEMENT_INPUTS || paths.iter().any(|(p, _)| !finite(p)) {
        return Err(PathOpsError::BadInput);
    }
    let closed_paths: Vec<BezPath> = paths.iter().map(|(p, _)| closed(p)).collect();
    if closed_paths.iter().all(|p| p.elements().is_empty()) {
        return Ok(Vec::new());
    }
    let bbox = closed_paths.iter().filter(|p| !p.elements().is_empty()).map(Shape::bounding_box).reduce(|a, b| a.union(b));
    let m = bbox.map(|b| b.x0.abs().max(b.x1.abs()).max(b.y0.abs()).max(b.y1.abs())).unwrap_or(0.0);
    let eps = (m * f64::EPSILON * 64.0).max(1e-6);
    let even_odd: Vec<bool> = paths.iter().map(|(_, e)| *e).collect();
    sweep(|| arrange(&closed_paths, &even_odd, eps))
}

/// [`faces`] of the closed `paths`, swept with tolerance `eps`.
fn arrange(closed_paths: &[BezPath], even_odd: &[bool], eps: f64) -> Result<Vec<ArrangementFace>, PathOpsError> {
    let topo = linesweeper::topology::Topology::<Windings>::from_paths(closed_paths.iter().enumerate().map(|(i, p)| (p, i)), eps)
        .map_err(|_| PathOpsError::BadInput)?;
    let cover_of = |w: &Windings| -> Vec<usize> {
        w.0.iter()
            .enumerate()
            .filter(|(i, n)| if even_odd.get(*i).copied().unwrap_or(false) { *n % 2 != 0 } else { **n != 0 })
            .map(|(i, _)| i)
            .collect()
    };
    // Every coverage that occurs on either side of an output segment.
    let mut covers: std::collections::BTreeSet<Vec<usize>> = std::collections::BTreeSet::new();
    for seg in topo.segment_indices() {
        let half = seg.first_half();
        for w in [topo.winding_clockwise(half), topo.winding_counter_clockwise(half)] {
            let c = cover_of(w);
            if !c.is_empty() {
                covers.insert(c);
            }
        }
    }
    let mut out = Vec::new();
    for cover in covers {
        let contours = topo.contours(|w| cover_of(w) == cover);
        let all: Vec<&linesweeper::topology::Contour> = contours.contours().collect();
        // Each outer contour with its direct holes is one connected face (islands inside the
        // holes are faces of their own).
        for (i, c) in all.iter().enumerate().filter(|(_, c)| c.outer) {
            let mut path = c.path.clone();
            for hole in all.iter().filter(|h| !h.outer && h.parent.map(|p| p.0) == Some(i)) {
                path.extend(hole.path.elements().iter().copied());
            }
            out.push(ArrangementFace { path, cover: cover.clone() });
            if out.len() >= MAX_FACES {
                return Ok(out);
            }
        }
    }
    Ok(out)
}

/// The union of disjoint faces (shared edges dissolve) as one clean path.
pub fn union_faces<'a>(faces: impl IntoIterator<Item = &'a BezPath>) -> Result<BezPath, PathOpsError> {
    let mut all = BezPath::new();
    for f in faces {
        all.extend(f.elements().iter().copied());
    }
    simplify(&all)
}

pub(crate) fn kcap(c: StrokeCap) -> Cap {
    match c {
        StrokeCap::Round | StrokeCap::CircleFilled => Cap::Round,
        StrokeCap::Square | StrokeCap::DiamondFilled => Cap::Square,
        _ => Cap::Butt,
    }
}

pub(crate) fn kjoin(j: StrokeJoin) -> Join {
    match j {
        StrokeJoin::Miter => Join::Miter,
        StrokeJoin::Bevel => Join::Bevel,
        StrokeJoin::Round => Join::Round,
    }
}

/// The widest stroke outlined: past this the rasteriser's fixed-point maths overflows and panics,
/// and nothing visible changes (a stroke this wide covers any canvas already).
pub const MAX_STROKE_WIDTH: f64 = 1e6;

/// Plain centred stroke of `path` at `width` (capped at [`MAX_STROKE_WIDTH`]).
pub fn stroke_center(path: &BezPath, width: f64, props: &StrokeProps) -> BezPath {
    if !(width.is_finite() && width > 0.0 && finite(path)) {
        return BezPath::new();
    }
    let width = width.min(MAX_STROKE_WIDTH);
    let mut style = kurbo::Stroke::new(width)
        .with_join(kjoin(props.join))
        .with_miter_limit(props.miter_limit.clamp(1.0, 100.0))
        .with_start_cap(kcap(props.cap_start))
        .with_end_cap(kcap(props.cap_end));
    if let Some(pat) = width::dash_pattern(props) {
        style = style.with_dashes(0.0, pat);
    }
    kurbo::stroke(path.iter(), &style, &kurbo::StrokeOpts::default(), 0.05)
}

/// The painted stroke area of a shape as a fill path (local coordinates). `closed` says whether the
/// shape is closed (Inside/Outside only apply to closed shapes). A width profile makes the width
/// vary along the path (fill the result non-zero when centred).
pub fn stroke_outline(path: &BezPath, props: &StrokeProps, closed_shape: bool) -> BezPath {
    stroke_outline_of(path, props, closed_shape, None)
}

/// [`stroke_outline`] of a shape whose stroke follows vector network `net` (`path` is its stroke
/// path): points with a cap or join of their own draw them there.
pub fn stroke_outline_of(path: &BezPath, props: &StrokeProps, closed_shape: bool, net: Option<&VectorNetwork>) -> BezPath {
    if !(props.weight.is_finite() && props.weight > 0.0) {
        return BezPath::new();
    }
    let capped;
    let props = if props.weight > MAX_STROKE_WIDTH {
        capped = StrokeProps { weight: MAX_STROKE_WIDTH, ..props.clone() };
        &capped
    } else {
        props
    };
    let w = props.weight;
    let styled = net.and_then(|n| points::Styled::new(n, props));
    // Centred at `width` (twice the weight for Inside/Outside, which clip half of it away).
    let centred = |width: f64| match (&styled, &props.brush, props.profile()) {
        (Some(st), ..) => st.outline(width, props),
        (None, Some(b), _) => brush::outline(path, width, props, b).unwrap_or_else(|| stroke_center(path, width, props)),
        (None, None, Some(p)) => width::variable_outline(path, width, props, p).unwrap_or_else(|| stroke_center(path, width, props)),
        (None, None, None) => stroke_center(path, width, props),
    };
    let align = if closed_shape { props.align } else { StrokeAlign::Center };
    let mut out = match align {
        StrokeAlign::Center => centred(w),
        StrokeAlign::Inside => {
            let s = centred(w * 2.0);
            binary(&s, path, linesweeper::BinaryOp::Intersection).unwrap_or(s)
        }
        StrokeAlign::Outside => {
            let s = centred(w * 2.0);
            binary(&s, path, linesweeper::BinaryOp::Difference).unwrap_or(s)
        }
    };
    if let Some(st) = styled.as_ref().filter(|_| !closed_shape) {
        out.extend(st.arrowheads(w, props).elements().iter().copied());
    } else if !closed_shape {
        for (at_end, cap) in [(false, props.cap_start), (true, props.cap_end)] {
            if cap.is_arrow()
                && let Some(head) = arrowhead(path, cap, w, at_end)
            {
                out.extend(head.elements().iter().copied());
            }
        }
    }
    out
}

/// End point and outward direction of an open path at its start or end.
fn path_end(path: &BezPath, at_end: bool) -> Option<(Point, Vec2)> {
    let segs: Vec<PathSeg> = path.segments().collect();
    let seg = if at_end { segs.last()? } else { segs.first()? };
    use kurbo::ParamCurve;
    use kurbo::ParamCurveDeriv;
    let (p, d) = if at_end {
        let d = match seg {
            PathSeg::Line(l) => l.p1 - l.p0,
            PathSeg::Quad(q) => q.deriv().eval(1.0).to_vec2(),
            PathSeg::Cubic(c) => {
                let d = c.deriv().eval(1.0).to_vec2();
                if d.hypot() < 1e-9 { c.p3 - c.p2 } else { d }
            }
        };
        (seg.end(), d)
    } else {
        let d = match seg {
            PathSeg::Line(l) => l.p0 - l.p1,
            PathSeg::Quad(q) => -q.deriv().eval(0.0).to_vec2(),
            PathSeg::Cubic(c) => {
                let d = -c.deriv().eval(0.0).to_vec2();
                if d.hypot() < 1e-9 { c.p0 - c.p1 } else { d }
            }
        };
        (seg.start(), d)
    };
    let len = d.hypot();
    if len < 1e-12 {
        return None;
    }
    Some((p, d / len))
}

/// The filled arrowhead shape for `cap` at one end of an open path.
pub fn arrowhead(path: &BezPath, cap: StrokeCap, weight: f64, at_end: bool) -> Option<BezPath> {
    let (tip, dir) = path_end(path, at_end)?;
    let n = Vec2::new(-dir.y, dir.x);
    let s = (weight * 3.0).max(6.0);
    let mut p = BezPath::new();
    match cap {
        StrokeCap::TriangleArrow => {
            p.move_to(tip + dir * (weight * 0.5));
            p.line_to(tip - dir * s + n * (s * 0.6));
            p.line_to(tip - dir * s - n * (s * 0.6));
            p.close_path();
        }
        StrokeCap::ReversedTriangle => {
            p.move_to(tip - dir * s);
            p.line_to(tip + n * (s * 0.6));
            p.line_to(tip - n * (s * 0.6));
            p.close_path();
        }
        StrokeCap::LineArrow => {
            let mut l = BezPath::new();
            l.move_to(tip - dir * s + n * (s * 0.7));
            l.line_to(tip);
            l.line_to(tip - dir * s - n * (s * 0.7));
            let props =
                StrokeProps { weight, cap_start: StrokeCap::Round, cap_end: StrokeCap::Round, join: StrokeJoin::Round, ..StrokeProps::default() };
            return Some(stroke_center(&l, weight, &props));
        }
        StrokeCap::CircleFilled => {
            p = kurbo::Circle::new(tip, s * 0.4).to_path(0.05);
        }
        StrokeCap::DiamondFilled => {
            let r = s * 0.5;
            p.move_to(tip + dir * r);
            p.line_to(tip + n * r);
            p.line_to(tip - dir * r);
            p.line_to(tip - n * r);
            p.close_path();
        }
        _ => return None,
    }
    Some(p)
}

/// Per-side stroke of a rectangle `[top, right, bottom, left]`, painted inside/center/outside.
pub fn rect_side_strokes(w: f64, h: f64, sides: [f64; 4], align: StrokeAlign) -> BezPath {
    let [t, r, b, l] = sides.map(|v| if v.is_finite() { v.max(0.0) } else { 0.0 });
    let f = match align {
        StrokeAlign::Inside => 0.0,
        StrokeAlign::Center => 0.5,
        StrokeAlign::Outside => 1.0,
    };
    let rect = |x0: f64, y0: f64, x1: f64, y1: f64| figcraft_geom::Rect::new(x0, y0, x1, y1).to_path(0.1);
    let mut p = BezPath::new();
    // Outer box edges.
    let (ox0, oy0, ox1, oy1) = (-l * f, -t * f, w + r * f, h + b * f);
    if t > 0.0 {
        p.extend(rect(ox0, oy0, ox1, oy0 + t).elements().iter().copied());
    }
    if b > 0.0 {
        p.extend(rect(ox0, oy1 - b, ox1, oy1).elements().iter().copied());
    }
    if l > 0.0 {
        p.extend(rect(ox0, oy0 + t, ox0 + l, oy1 - b).elements().iter().copied());
    }
    if r > 0.0 {
        p.extend(rect(ox1 - r, oy0 + t, ox1, oy1 - b).elements().iter().copied());
    }
    p
}

/// Per-side stroke of a rectangle with corner radii `[tl, tr, br, bl]`, like CSS borders: the outer
/// edge is the rounded rectangle (center and outside grow its box by half or all of each side's
/// weight, and each non-zero radius with it); the inner edge is inset by each side's weight, with
/// elliptical corners `max(0, R − weight)` across and down (square where either is 0). Corners
/// that stay circular are smoothed like the shape's. An even-odd ring whose inner edge runs against
/// the outer one, so it is a ring under non-zero too.
pub fn rounded_side_strokes(w: f64, h: f64, radii: [f64; 4], smoothing: f64, sides: [f64; 4], align: StrokeAlign) -> BezPath {
    let [t, r, b, l] = sides.map(non_negative);
    if t + r + b + l <= 0.0 {
        return BezPath::new();
    }
    let f = match align {
        StrokeAlign::Inside => 0.0,
        StrokeAlign::Center => 0.5,
        StrokeAlign::Outside => 1.0,
    };
    // Clamped like the fill: at most half the shorter side.
    let max_r = non_negative(w.min(h)) / 2.0;
    let radii = radii.map(|v| non_negative(v).min(max_r));
    // Each corner's radius with the weights across and down it, clockwise from the top-left.
    let corners = [(radii[0], l, t), (radii[1], r, t), (radii[2], r, b), (radii[3], l, b)];
    let outer = if align == StrokeAlign::Inside {
        figcraft_geom::rounded_rect(w, h, radii, smoothing)
    } else {
        let grown = corners.map(|(rr, x, y)| if rr > 0.0 { (rr + x * f, rr + y * f) } else { (0.0, 0.0) });
        corner_rect(figcraft_geom::Rect::new(-l * f, -t * f, w + r * f, h + b * f), grown, smoothing)
    };
    let k = 1.0 - f;
    let inner = figcraft_geom::Rect::new(l * k, t * k, w - r * k, h - b * k);
    if inner.width() <= 0.0 || inner.height() <= 0.0 {
        return outer;
    }
    let (inset, fit) = fit_corners(inner, corners.map(|(rr, x, y)| ((rr - x * k).max(0.0), (rr - y * k).max(0.0))));
    // The hole's corners sit on the outer ones, inside them, unless they were shrunk to fit or
    // smoothed: then it may leave the outer edge.
    ring(outer, &corner_rect(inner, inset, smoothing), fit < 1.0 || smoothing > 0.0)
}

/// Rectangle `r` with `(rx, ry)` corners, smoothed like `figcraft_geom::rounded_rect` where every
/// corner is circular (smoothing is defined for those only).
fn corner_rect(r: figcraft_geom::Rect, corners: [(f64, f64); 4], smoothing: f64) -> BezPath {
    if smoothing > 0.0 && corners.iter().all(|(rx, ry)| rx == ry) {
        return kurbo::Affine::translate((r.x0, r.y0)) * figcraft_geom::rounded_rect(r.width(), r.height(), corners.map(|c| c.0), smoothing);
    }
    elliptical_rect(r, corners)
}

fn non_negative(v: f64) -> f64 {
    if v.is_finite() { v.max(0.0) } else { 0.0 }
}

/// `outer` with `hole` taken out: cut out of it when the hole may leave it, else as an even-odd
/// ring whose hole runs against it.
fn ring(outer: BezPath, hole: &BezPath, cut: bool) -> BezPath {
    if cut && let Ok(p) = boolean_step(&outer, hole, BoolOp::Subtract) {
        return p;
    }
    let mut p = outer;
    p.extend(hole.reverse_subpaths().elements().iter().copied());
    p
}

/// Corner radii `(rx, ry)` for the inner box `inner`, square where either is 0, all shrunk by one
/// factor (returned too) where two on an edge add up to more than it, as CSS shrinks radii.
fn fit_corners(inner: figcraft_geom::Rect, corners: [(f64, f64); 4]) -> ([(f64, f64); 4], f64) {
    let c = corners.map(|(rx, ry)| if rx > 0.0 && ry > 0.0 { (rx, ry) } else { (0.0, 0.0) });
    let [tl, tr, br, bl] = c;
    let edges = [(inner.width(), tl.0 + tr.0), (inner.width(), bl.0 + br.0), (inner.height(), tl.1 + bl.1), (inner.height(), tr.1 + br.1)];
    let fit = edges.iter().filter(|(_, sum)| *sum > 0.0).fold(1.0_f64, |m, (len, sum)| m.min(len / sum));
    (c.map(|(rx, ry)| (rx * fit, ry * fit)), fit)
}

/// Rectangle `r` with an `(rx, ry)` quarter ellipse at each corner `[tl, tr, br, bl]`, drawn like
/// `figcraft_geom::rounded_rect` (circular corners come out the same).
fn elliptical_rect(r: figcraft_geom::Rect, [tl, tr, br, bl]: [(f64, f64); 4]) -> BezPath {
    use std::f64::consts::{FRAC_PI_2, PI};
    // Each corner, the direction of its ellipse centre from it, and the angle its quarter starts at.
    let quarters = [
        (Point::new(r.x1, r.y0), (-1.0, 1.0), tr, -FRAC_PI_2),
        (Point::new(r.x1, r.y1), (-1.0, -1.0), br, 0.0),
        (Point::new(r.x0, r.y1), (1.0, -1.0), bl, FRAC_PI_2),
        (Point::new(r.x0, r.y0), (1.0, 1.0), tl, PI),
    ];
    let mut p = BezPath::new();
    p.move_to((r.x0 + tl.0, r.y0));
    for (corner, (sx, sy), (rx, ry), start) in quarters {
        if rx > 0.0 && ry > 0.0 {
            let c = Point::new(corner.x + sx * rx, corner.y + sy * ry);
            p.line_to((c.x + rx * start.cos(), c.y + ry * start.sin()));
            kurbo::Arc::new(c, (rx, ry), start, FRAC_PI_2, 0.0).to_cubic_beziers(0.01, |a, b, e| p.curve_to(a, b, e));
        } else {
            p.line_to(corner);
        }
    }
    p.close_path();
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use figcraft_geom::Rect;

    fn sq(x: f64, y: f64, s: f64) -> BezPath {
        Rect::new(x, y, x + s, y + s).to_path(0.1)
    }

    fn area_eo(p: &BezPath) -> f64 {
        // Sum of absolute contour areas with even-odd nesting approximated by signed area sum.
        p.area().abs()
    }

    #[test]
    fn a_panic_in_the_sweep_is_bad_input() {
        let r: Result<(), PathOpsError> = sweep(|| panic!("sweep bug"));
        assert!(matches!(r, Err(PathOpsError::BadInput)));
        assert!(matches!(sweep(|| Ok(1)), Ok(1)));
    }

    #[test]
    fn union_intersect_subtract_exclude() {
        let a = sq(0.0, 0.0, 10.0);
        let b = sq(5.0, 5.0, 10.0);
        let u = boolean(&[a.clone(), b.clone()], BoolOp::Union).unwrap();
        assert!((area_eo(&u) - 175.0).abs() < 0.5, "{}", area_eo(&u));
        let i = boolean(&[a.clone(), b.clone()], BoolOp::Intersect).unwrap();
        assert!((area_eo(&i) - 25.0).abs() < 0.5);
        let s = boolean(&[a.clone(), b.clone()], BoolOp::Subtract).unwrap();
        assert!((area_eo(&s) - 75.0).abs() < 0.5);
        let x = boolean(&[a, b], BoolOp::Exclude).unwrap();
        let bb = x.bounding_box();
        assert_eq!((bb.x0, bb.y0, bb.x1, bb.y1), (0.0, 0.0, 15.0, 15.0));
    }

    #[test]
    fn arrangement_faces() {
        // Two overlapping squares: A only, B only, both.
        let f = faces(&[(sq(0.0, 0.0, 10.0), false), (sq(5.0, 5.0, 10.0), false)]).unwrap();
        let mut got: Vec<(Vec<usize>, i64)> = f.iter().map(|x| (x.cover.clone(), x.path.area().abs().round() as i64)).collect();
        got.sort();
        assert_eq!(got, vec![(vec![0], 75), (vec![0, 1], 25), (vec![1], 75)]);
        // Uniting faces dissolves their shared edges.
        let u = union_faces(f.iter().map(|x| &x.path)).unwrap();
        assert!((u.area().abs() - 175.0).abs() < 1e-6);
        assert_eq!(u.elements().iter().filter(|e| matches!(e, PathEl::MoveTo(_))).count(), 1);
        // A square inside a ring (even-odd): the ring, the square, and the gap between them is
        // not covered; the square's area under the ring's hole is a face of its own.
        let mut ring = sq(0.0, 0.0, 30.0);
        ring.extend(sq(10.0, 10.0, 10.0).elements().iter().copied());
        let f = faces(&[(ring, true), (sq(12.0, 12.0, 4.0), false)]).unwrap();
        let mut got: Vec<(Vec<usize>, i64)> = f.iter().map(|x| (x.cover.clone(), x.path.area().abs().round() as i64)).collect();
        got.sort();
        assert_eq!(got, vec![(vec![0], 800), (vec![1], 16)]);
        // Two overlapping circles: three faces with curved edges.
        let c = |x: f64| kurbo::Circle::new((x, 0.0), 10.0).to_path(0.01);
        let f = faces(&[(c(0.0), false), (c(10.0), false)]).unwrap();
        assert_eq!(f.len(), 3);
        assert!(f.iter().all(|x| x.path.elements().iter().any(|e| matches!(e, PathEl::CurveTo(..)))));
        // Hostile and empty input.
        let mut bad = BezPath::new();
        bad.move_to((f64::NAN, 0.0));
        bad.line_to((1.0, 1.0));
        assert!(faces(&[(bad, false)]).is_err());
        assert!(faces(&[]).unwrap().is_empty());
        assert!(faces(&vec![(sq(0.0, 0.0, 1.0), false); MAX_ARRANGEMENT_INPUTS + 1]).is_err());
    }

    #[test]
    fn hostile_inputs() {
        let mut bad = BezPath::new();
        bad.move_to((f64::NAN, 0.0));
        bad.line_to((1.0, 1.0));
        assert!(boolean(&[bad, sq(0.0, 0.0, 1.0)], BoolOp::Union).is_err());
        assert!(boolean(&[], BoolOp::Union).unwrap().elements().is_empty());
        let _ = stroke_outline(&sq(0.0, 0.0, 10.0), &StrokeProps { weight: f64::INFINITY, ..Default::default() }, true);
    }

    #[test]
    fn stroke_alignment_bounds() {
        let r = sq(0.0, 0.0, 100.0);
        let props = StrokeProps { weight: 10.0, align: StrokeAlign::Inside, ..Default::default() };
        let inside = stroke_outline(&r, &props, true);
        let b = inside.bounding_box();
        assert!(b.x0 >= -0.01 && b.x1 <= 100.01, "{b:?}");
        let outside = stroke_outline(&r, &StrokeProps { align: StrokeAlign::Outside, ..props.clone() }, true);
        let b = outside.bounding_box();
        assert!((b.x0 + 10.0).abs() < 0.01 && (b.x1 - 110.0).abs() < 0.01, "{b:?}");
        let center = stroke_outline(&r, &StrokeProps { align: StrokeAlign::Center, ..props }, true);
        let b = center.bounding_box();
        assert!((b.x0 + 5.0).abs() < 0.01);
    }

    #[test]
    fn arrows_on_open_paths() {
        let mut line = BezPath::new();
        line.move_to((0.0, 0.0));
        line.line_to((100.0, 0.0));
        let props = StrokeProps { weight: 2.0, cap_end: StrokeCap::TriangleArrow, ..Default::default() };
        let s = stroke_outline(&line, &props, false);
        assert!(s.bounding_box().height() > 4.0);
        assert!(arrowhead(&BezPath::new(), StrokeCap::LineArrow, 1.0, true).is_none());
    }

    #[test]
    fn equal_point_joins_keep_the_layer_stroke_and_differing_ones_apply() {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((10.0, 10.0));
        path.line_to((20.0, 0.0));
        let net = VectorNetwork {
            vertices: vec![
                figcraft_doc::Vertex { cap: Some(StrokeCap::Square), ..figcraft_doc::Vertex::new(0.0, 0.0) },
                figcraft_doc::Vertex { x: 10.0, y: 10.0, radius: 2.0, join: Some(StrokeJoin::Miter), ..figcraft_doc::Vertex::new(10.0, 10.0) },
                figcraft_doc::Vertex { cap: Some(StrokeCap::Square), ..figcraft_doc::Vertex::new(20.0, 0.0) },
            ],
            segments: vec![figcraft_doc::Segment::line(0, 1), figcraft_doc::Segment::line(1, 2)],
            ..Default::default()
        };
        let props = StrokeProps { weight: 4.0, ..Default::default() };
        let inherited = stroke_outline_of(&path, &props, false, Some(&net));
        let mut without_join = net.clone();
        without_join.vertices[1].join = None;
        let baseline = stroke_outline_of(&path, &props, false, Some(&without_join));
        assert_eq!(inherited, baseline);

        let different_layer = StrokeProps { join: StrokeJoin::Bevel, ..props };
        let explicit = stroke_outline_of(&path, &different_layer, false, Some(&net));
        let layer_only = stroke_outline_of(&path, &different_layer, false, Some(&without_join));
        assert_ne!(explicit, layer_only);
    }

    #[test]
    fn equal_joins_do_not_break_closed_stroke_runs() {
        let path = sq(0.0, 0.0, 20.0);
        let mut net = VectorNetwork::from_points(&[Point::new(0.0, 0.0), Point::new(20.0, 0.0), Point::new(20.0, 20.0), Point::new(0.0, 20.0)], true);
        for vertex in &mut net.vertices {
            vertex.join = Some(StrokeJoin::Round);
        }
        let props = StrokeProps { join: StrokeJoin::Round, weight: 4.0, ..Default::default() };
        let styled = stroke_outline_of(&path, &props, true, Some(&net));
        for vertex in &mut net.vertices {
            vertex.join = None;
        }
        let inherited = stroke_outline_of(&path, &props, true, Some(&net));
        assert_eq!(styled, inherited);
    }

    #[test]
    fn dashes() {
        let mut line = BezPath::new();
        line.move_to((0.0, 0.0));
        line.line_to((100.0, 0.0));
        let props = StrokeProps { weight: 2.0, dashes: vec![10.0, 10.0], align: StrokeAlign::Center, ..Default::default() };
        let s = stroke_center(&line, 2.0, &props);
        assert!((s.area().abs() - 100.0).abs() < 2.0, "{}", s.area());
    }
}
