//! FigCraft path operations.
//!
//! - [`boolean`]: Union, Subtract, Intersect and Exclude over any number of closed paths (robust
//!   sweep-line topology from `linesweeper`); results use the even-odd rule.
//! - [`stroke_outline`]: a stroke as a fill, with Figma's Inside / Center / Outside alignment,
//!   dashes, caps, joins, arrowheads, variable width profiles ([`width`]) and brushes ([`brush`]).
//!
//! Every sweep is capped ([`MAX_SEGMENTS`], [`MAX_CROSSINGS`], and [`MAX_FOLD_SEGMENTS`] over a
//! fold): past the caps an operation is refused with [`PathOpsError::TooComplex`].
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod brush;
#[cfg(test)]
mod brush_tests;
mod budget;
#[cfg(test)]
mod budget_tests;
#[cfg(test)]
mod dash_cap_tests;
#[cfg(test)]
mod dash_ends_tests;
#[cfg(test)]
mod dot_dash_tests;
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

use figcraft_doc::{BoolOp, StrokeAlign, StrokeCap, StrokeJoin, StrokeProps, VectorNetwork, WindingRule};
use figcraft_geom::kurbo::{self, BezPath, Cap, Join, PathEl, PathSeg, Point, Shape, Vec2};

pub use budget::{Complexity, FoldBudget, MAX_CROSSINGS, MAX_FOLD_SEGMENTS, MAX_SEGMENTS, complexity};

#[derive(Debug, PartialEq)]
pub enum PathOpsError {
    /// The inputs contain NaN or infinite coordinates.
    BadInput,
    /// The inputs are past [`MAX_SEGMENTS`] or [`MAX_CROSSINGS`]: the operation is refused rather
    /// than run for unbounded time and memory.
    TooComplex,
}

impl std::fmt::Display for PathOpsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadInput => write!(f, "path contains invalid coordinates"),
            Self::TooComplex => {
                write!(f, "the shapes are too complex to combine (over {MAX_SEGMENTS} segments or {MAX_CROSSINGS} possible crossings)")
            }
        }
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
// shortcut: wasm32 can't catch it (panics abort there): the page stops and its panic hook keeps the
// unsaved files (apps/figcraft-web/src/recovery.rs); drop this once linesweeper no longer panics.
fn sweep<T>(f: impl FnOnce() -> Result<T, PathOpsError>) -> Result<T, PathOpsError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(Err(PathOpsError::BadInput))
}

fn binary(a: &BezPath, b: &BezPath, op: linesweeper::BinaryOp) -> Result<BezPath, PathOpsError> {
    binary_by(a, b, linesweeper::FillRule::NonZero, op)
}

/// [`binary`] with both operands read by fill rule `rule`.
fn binary_by(a: &BezPath, b: &BezPath, rule: linesweeper::FillRule, op: linesweeper::BinaryOp) -> Result<BezPath, PathOpsError> {
    if a.elements().is_empty() && b.elements().is_empty() {
        return Ok(BezPath::new());
    }
    if !finite(a) || !finite(b) {
        return Err(PathOpsError::BadInput);
    }
    let (a, b) = (closed(a), closed(b));
    budget::check([&a, &b])?;
    let contours = sweep(|| linesweeper::binary_op(&a, &b, rule, op).map_err(|_| PathOpsError::BadInput))?;
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

/// Normalize one path using its fill rule. Unlike [`faces`], this emits every contour without a
/// face-count cap, so render silhouettes preserve large imported networks.
pub fn simplify_with_rule(path: &BezPath, even_odd: bool) -> Result<BezPath, PathOpsError> {
    let rule = if even_odd { linesweeper::FillRule::EvenOdd } else { linesweeper::FillRule::NonZero };
    binary_by(path, &BezPath::new(), rule, linesweeper::BinaryOp::Union)
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
/// their own operations paints. A lone operand comes out simplified. Past [`MAX_FOLD_SEGMENTS`]
/// swept over all the steps, the fold is refused ([`FoldBudget`]).
pub fn boolean_ops<'a>(operands: impl IntoIterator<Item = (&'a BezPath, BoolOp)>) -> Result<BezPath, PathOpsError> {
    let mut it = operands.into_iter();
    let Some((first, _)) = it.next() else { return Ok(BezPath::new()) };
    let mut acc: Option<BezPath> = None;
    let mut budget = FoldBudget::default();
    for (p, op) in it {
        let below = acc.as_ref().unwrap_or(first);
        budget.spend(below, p)?;
        acc = Some(boolean_step(below, p, op)?);
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
    budget::check(paths.iter().map(|(p, _)| p))?;
    // Self-intersecting or self-overlapping inputs (a centred stroke outlined around a sharp
    // corner folds over itself and touches the shape's fill) leave the winding on each side of a
    // segment ambiguous where they touch, which made the sweep report overlapping faces. Reducing
    // every input to the simple contours of the area it fills first keeps the faces disjoint.
    let closed_paths: Vec<BezPath> = paths.iter().map(|(p, even_odd)| plain(&closed(p), *even_odd)).collect();
    if closed_paths.iter().all(|p| p.elements().is_empty()) {
        return Ok(Vec::new());
    }
    let bbox = closed_paths.iter().filter(|p| !p.elements().is_empty()).map(Shape::bounding_box).reduce(|a, b| a.union(b));
    let m = bbox.map(|b| b.x0.abs().max(b.x1.abs()).max(b.y0.abs()).max(b.y1.abs())).unwrap_or(0.0);
    let eps = (m * f64::EPSILON * 64.0).max(1e-6);
    // Simplifying split the inputs where they crossed themselves: the arrangement is checked too.
    budget::check(&closed_paths)?;
    sweep(|| arrange(&closed_paths, eps))
}

/// The area `path` fills under `even_odd` as simple contours (self-intersections and overlapping
/// subpaths resolved), which both fill rules then cover the same. A path the sweep can't handle
/// comes back unchanged, so [`faces`] still tries it as it is.
fn plain(path: &BezPath, even_odd: bool) -> BezPath {
    if path.elements().is_empty() {
        return BezPath::new();
    }
    let rule = if even_odd { linesweeper::FillRule::EvenOdd } else { linesweeper::FillRule::NonZero };
    let swept = sweep(|| linesweeper::binary_op(path, &BezPath::new(), rule, linesweeper::BinaryOp::Union).map_err(|_| PathOpsError::BadInput));
    let Ok(contours) = swept else { return path.clone() };
    let mut out = BezPath::new();
    for c in contours.contours() {
        out.extend(c.path.elements().iter().copied());
    }
    out
}

/// [`faces`] of the closed, simple `paths`, swept with tolerance `eps`.
fn arrange(closed_paths: &[BezPath], eps: f64) -> Result<Vec<ArrangementFace>, PathOpsError> {
    let topo = linesweeper::topology::Topology::<Windings>::from_paths(closed_paths.iter().enumerate().map(|(i, p)| (p, i)), eps)
        .map_err(|_| PathOpsError::BadInput)?;
    // Every path was reduced to simple contours, so either rule covers the same area: non-zero.
    let cover_of = |w: &Windings| -> Vec<usize> { w.0.iter().enumerate().filter(|(_, n)| **n != 0).map(|(i, _)| i).collect() };
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
    match c.shape() {
        StrokeCap::Round => Cap::Round,
        StrokeCap::Square => Cap::Square,
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
    let caps = props.band_caps();
    let style = kurbo::Stroke::new(width)
        .with_join(kjoin(props.join))
        .with_miter_limit(props.miter_limit.clamp(1.0, 100.0))
        .with_start_cap(kcap(caps[0]))
        .with_end_cap(kcap(caps[1]));
    // Each subpath restarts the pattern, so it can add up to one more pattern's worth of dashes.
    let subpaths = path.elements().iter().filter(|e| matches!(e, PathEl::MoveTo(_))).count() as f64;
    // A zero-length dash is a dot of its cap, in viewers too; older squares stay undrawn here.
    let dots = caps.into_iter().any(|c| matches!(kcap(c), Cap::Round)) || props.dash_cap == Some(StrokeCap::Square);
    let pattern = width::dash_pattern(props).map(|p| p.into_iter().map(|d| if dots && d == 0.0 { 1e-3 } else { d }).collect::<Vec<_>>());
    let opts = kurbo::StrokeOpts::default();
    let Some(pat) = pattern.filter(|p| width::dashes_fit(path.perimeter(1.0) + subpaths * p.iter().sum::<f64>(), p)) else {
        return kurbo::stroke(path.iter(), &style, &opts, 0.05);
    };
    // Each subpath fits the pattern to its own length.
    let mut out = BezPath::new();
    for sub in figcraft_geom::split_subpaths(path) {
        let (dashes, offset) = width::subpath_dashes(&sub, &pat);
        out.extend(kurbo::stroke(sub.iter(), &style.clone().with_dashes(offset, dashes), &opts, 0.05));
    }
    out
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
    // Inside/Outside clip against the shape's own fill, so an even-odd shape's holes stay holes.
    let even_odd = net.is_some_and(|n| n.regions.iter().any(|r| r.winding == WindingRule::EvenOdd));
    let clip = |s: BezPath, op: linesweeper::BinaryOp| {
        if even_odd {
            // The stroke overlaps itself (non-zero): flatten it to even-odd contours first.
            simplify(&s).and_then(|s| binary_by(&s, path, linesweeper::FillRule::EvenOdd, op)).unwrap_or(s)
        } else {
            binary(&s, path, op).unwrap_or(s)
        }
    };
    let mut out = match align {
        StrokeAlign::Center => centred(w),
        StrokeAlign::Inside => clip(centred(w * 2.0), linesweeper::BinaryOp::Intersection),
        StrokeAlign::Outside => clip(centred(w * 2.0), linesweeper::BinaryOp::Difference),
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
    use kurbo::ParamCurve;
    use kurbo::ParamCurveDeriv;
    let dir_of = |seg: &PathSeg| -> Option<(Point, Vec2)> {
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
        (len >= 1e-12).then(|| (p, d / len))
    };
    // Skip zero-length segments (repeated vertices) and use the nearest one with a direction.
    if at_end { path.segments().collect::<Vec<_>>().iter().rev().find_map(dir_of) } else { path.segments().find_map(|s| dir_of(&s)) }
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

    /// A five-pointed star with sharp tips, and its centred stroke, whose outline folds over
    /// itself at every tip (a self-intersecting non-zero input that touches the star's fill).
    fn star_and_stroke() -> (BezPath, BezPath) {
        let mut star = BezPath::new();
        for i in 0..10 {
            let a = -std::f64::consts::FRAC_PI_2 + i as f64 * std::f64::consts::PI / 5.0;
            let r = if i % 2 == 0 { 100.0 } else { 40.0 };
            let p = (150.0 + r * a.cos(), 150.0 + r * a.sin());
            if i == 0 {
                star.move_to(p);
            } else {
                star.line_to(p);
            }
        }
        star.close_path();
        (star.clone(), stroke_center(&star, 8.0, &StrokeProps::default()))
    }

    #[test]
    fn self_intersecting_inputs_make_disjoint_faces() {
        let (star, ring) = star_and_stroke();
        // The stroke outline really is self-intersecting: its winding is 2 at the tips.
        assert_eq!(ring.winding(Point::new(150.0, 56.0)), 2);
        let f = faces(&[(star.clone(), false), (ring.clone(), false)]).unwrap();
        let mut got: Vec<Vec<usize>> = f.iter().map(|x| x.cover.clone()).collect();
        got.sort();
        assert_eq!(got, vec![vec![0], vec![0, 1], vec![1]], "the star's inside, its stroke's inner half, its outer half");
        // Faces never overlap: no face's interior point is inside another face.
        for (i, a) in f.iter().enumerate() {
            let Some(p) = figcraft_doc::vector_faces::interior_point(&a.path) else { continue };
            for (j, b) in f.iter().enumerate() {
                assert!(i == j || b.path.winding(p) == 0, "faces {i} and {j} overlap at {p:?}");
            }
        }
        // They cover exactly what the two shapes paint together.
        let painted = boolean(&[star, ring], BoolOp::Union).unwrap();
        let all = union_faces(f.iter().map(|x| &x.path)).unwrap();
        assert!((all.area().abs() - painted.area().abs()).abs() < 0.5, "{} vs {}", all.area().abs(), painted.area().abs());
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
    fn layer_end_arrow_is_drawn_when_a_closed_run_follows_the_open_one() {
        let verts = [(0.0, 0.0), (50.0, 0.0), (0.0, 100.0), (50.0, 100.0), (25.0, 150.0)];
        let net = VectorNetwork {
            vertices: verts
                .iter()
                .enumerate()
                .map(|(i, &(x, y))| figcraft_doc::Vertex { cap: (i == 0).then_some(StrokeCap::Round), ..figcraft_doc::Vertex::new(x, y) })
                .collect(),
            segments: vec![
                figcraft_doc::Segment::line(0, 1),
                figcraft_doc::Segment::line(2, 3),
                figcraft_doc::Segment::line(3, 4),
                figcraft_doc::Segment::line(4, 2),
            ],
            ..Default::default()
        };
        let path = BezPath::new();
        let plain = StrokeProps { weight: 2.0, ..Default::default() };
        let arrow = StrokeProps { cap_end: StrokeCap::TriangleArrow, ..plain.clone() };
        assert_ne!(stroke_outline_of(&path, &arrow, false, Some(&net)), stroke_outline_of(&path, &plain, false, Some(&net)));
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

    #[test]
    fn a_dash_pattern_too_fine_for_its_path_strokes_solid() {
        // ~500k dashes on this line (#1060): drawn solid, as a variable width stroke is past
        // `MAX_DASHES`, rather than one outline per dash.
        let mut line = BezPath::new();
        line.move_to((0.0, 0.0));
        line.line_to((10_000.0, 0.0));
        let props = StrokeProps { weight: 2.0, dashes: vec![0.01, 0.01], align: StrokeAlign::Center, ..Default::default() };
        let s = stroke_center(&line, 2.0, &props);
        assert!(s.elements().len() < 100, "{} elements", s.elements().len());
        assert!((s.area().abs() - 20_000.0).abs() < 10.0, "{}", s.area());
    }

    #[test]
    fn each_subpath_restarting_the_dash_pattern_counts_toward_the_limit() {
        // The pattern restarts at every subpath: on 10,000 short lines its 199 tiny dashes come
        // round each time (~1M dashes), though the length alone suggests a few hundred.
        let mut path = BezPath::new();
        for i in 0..10_000 {
            path.move_to((0.0, f64::from(i) * 4.0));
            path.line_to((2.0, f64::from(i) * 4.0));
        }
        let mut dashes = vec![0.01; 199];
        dashes.push(10_000.0);
        let props = StrokeProps { weight: 1.0, dashes, align: StrokeAlign::Center, ..Default::default() };
        let s = stroke_center(&path, 1.0, &props);
        assert!(s.elements().len() < 100_000, "{} elements", s.elements().len());
    }

    #[test]
    fn inside_and_outside_strokes_respect_an_even_odd_hole() {
        // A ring: both loops run the same way, so only the even-odd rule leaves the hole open.
        let mut ring = sq(0.0, 0.0, 100.0);
        ring.extend(sq(30.0, 30.0, 40.0).elements().iter().copied());
        let mut net = VectorNetwork::from_path(&ring);
        for r in &mut net.regions {
            r.winding = figcraft_doc::WindingRule::EvenOdd;
        }
        let (material, hole) = (Point::new(29.0, 50.0), Point::new(31.0, 50.0));
        let inside = StrokeProps { weight: 4.0, align: StrokeAlign::Inside, ..Default::default() };
        let s = stroke_outline_of(&ring, &inside, true, Some(&net));
        assert_ne!(s.winding(material), 0, "inside stroke misses the material");
        assert_eq!(s.winding(hole), 0, "inside stroke leaks into the hole");
        let outside = StrokeProps { align: StrokeAlign::Outside, ..inside.clone() };
        let s = stroke_outline_of(&ring, &outside, true, Some(&net));
        assert_eq!(s.winding(material), 0, "outside stroke covers the material");
        assert_ne!(s.winding(hole), 0, "outside stroke misses the hole");
        // Non-zero: the inner loop doesn't cut a hole, so its inside stroke covers both sides.
        let nz = VectorNetwork::from_path(&ring);
        let s = stroke_outline_of(&ring, &inside, true, Some(&nz));
        assert!(s.winding(material) != 0 && s.winding(hole) != 0);
    }

    #[test]
    fn arrowhead_skips_zero_length_end_segments() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((100.0, 0.0));
        assert!(arrowhead(&p, StrokeCap::TriangleArrow, 2.0, true).is_some());
        p.line_to((100.0, 0.0));
        let (tip, dir) = path_end(&p, true).expect("end direction");
        assert_eq!((tip, dir), (Point::new(100.0, 0.0), Vec2::new(1.0, 0.0)));
        assert!(arrowhead(&p, StrokeCap::TriangleArrow, 2.0, true).is_some());
        let mut q = BezPath::new();
        q.move_to((0.0, 0.0));
        q.line_to((0.0, 0.0));
        q.line_to((100.0, 0.0));
        let (tip, dir) = path_end(&q, false).expect("start direction");
        assert_eq!((tip, dir), (Point::new(0.0, 0.0), Vec2::new(-1.0, 0.0)));
    }
}
