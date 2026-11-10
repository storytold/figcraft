//! A corner radius on a closed path: every vertex that is still sharp rounds with a circular arc,
//! convex and concave alike, the way a boolean group draws its result.
//!
//! What a sweep hands back is noisy: straight edges split at collinear points, pieces a hair long,
//! one real corner cut into two half turns. So each subpath is cleaned first ([`clean`]), then each
//! vertex is judged by the turn it makes, and a corner that does not fit shrinks to what the edges
//! beside it can hold ([`Ctx::corner`]).

use figcraft_geom::kurbo::{BezPath, CubicBez, ParamCurve, ParamCurveArclen, ParamCurveDeriv, PathEl, PathSeg, Point, Shape, Vec2};
use std::f64::consts::PI;

/// The smallest turn (radians) that counts as a corner. The sweep moves end points by up to its
/// own 1e-6 accuracy, which can tilt a short piece by more than 1e-6 rad.
const SHARP_TURN: f64 = 1e-3;
/// Accuracy of arc length measurements.
const ACC: f64 = 1e-6;

#[derive(Clone, Copy)]
enum Seg {
    Line(Point, Point),
    Cubic(CubicBez),
}

impl Seg {
    fn start(&self) -> Point {
        match self {
            Self::Line(a, _) => *a,
            Self::Cubic(c) => c.p0,
        }
    }

    fn end(&self) -> Point {
        match self {
            Self::Line(_, b) => *b,
            Self::Cubic(c) => c.p3,
        }
    }

    fn set_start(&mut self, p: Point) {
        match self {
            Self::Line(a, _) => *a = p,
            Self::Cubic(c) => c.p0 = p,
        }
    }

    fn is_line(&self) -> bool {
        matches!(self, Self::Line(..))
    }

    /// As a cubic (a line gets its handles at the thirds).
    fn cubic(&self) -> CubicBez {
        match self {
            Self::Line(a, b) => CubicBez::new(*a, a.lerp(*b, 1.0 / 3.0), a.lerp(*b, 2.0 / 3.0), *b),
            Self::Cubic(c) => *c,
        }
    }

    fn len(&self) -> f64 {
        match self {
            Self::Line(a, b) => (*b - *a).hypot(),
            Self::Cubic(c) => c.arclen(ACC),
        }
    }

    /// The length of the control polygon: cheap, and zero only for a point.
    fn span(&self) -> f64 {
        let c = self.cubic();
        (c.p1 - c.p0).hypot() + (c.p2 - c.p1).hypot() + (c.p3 - c.p2).hypot()
    }

    /// The cubic leaving the start of the segment, or (`from_start` false) leaving its end.
    fn outward(&self, from_start: bool) -> CubicBez {
        let c = self.cubic();
        if from_start { c } else { CubicBez::new(c.p3, c.p2, c.p1, c.p0) }
    }
}

/// Unit direction in which `c` leaves its start (skipping coincident handles).
fn start_dir(c: &CubicBez) -> Option<Vec2> {
    [c.p1, c.p2, c.p3].into_iter().map(|p| p - c.p0).find(|d| d.hypot() > 1e-9).map(|d| d / d.hypot())
}

/// The point `t` along `c` from its start, the unit direction back toward the start there, and the
/// parameter reached.
fn trim(c: &CubicBez, t: f64, line: bool, len: f64) -> Option<(Point, Vec2, f64)> {
    if line {
        if !len.is_finite() || len <= 0.0 {
            return None;
        }
        let u = (c.p3 - c.p0) / len;
        return Some((c.p0 + u * t, -u, t / len));
    }
    let k = c.inv_arclen(t, ACC).clamp(0.0, 1.0);
    let d = c.deriv().eval(k).to_vec2();
    let back = if d.hypot() > 1e-9 { -d / d.hypot() } else { -start_dir(c)? };
    Some((c.eval(k), back, k))
}

/// One rounded corner: the curves from the cut on the incoming segment to the cut on the outgoing
/// one, and where those cuts sit on their segments (each segment's own parameter).
struct Corner {
    curves: Vec<CubicBez>,
    t_in: f64,
    t_out: f64,
}

/// `path` with every corner still sharp rounded by `radius` (and corner `smoothing`, 0..=1, between
/// straight edges). A corner shrinks to what its edges can hold: two corners wanting more than
/// half an edge share it half and half, a corner whose far end is smooth may use all of it. A
/// closed subpath with no sharp corner, an open one, and a path with a bad radius or coordinates
/// come back as they are.
pub fn round_corners(path: &BezPath, radius: f64, smoothing: f64) -> BezPath {
    if !(radius.is_finite() && radius > 0.0) || path.elements().is_empty() || !crate::finite(path) {
        return path.clone();
    }
    let smoothing = if smoothing.is_finite() { smoothing.clamp(0.0, 1.0) } else { 0.0 };
    let bb = path.bounding_box();
    let weld = (bb.width().max(bb.height()) * 1e-6).max(1e-9);
    let mut out = BezPath::new();
    for sub in figcraft_geom::split_subpaths(path) {
        let closed = matches!(sub.elements().last(), Some(PathEl::ClosePath));
        match closed.then(|| round_subpath(&sub, radius, smoothing, weld)).flatten() {
            Some(p) => out.extend(p.elements().iter().copied()),
            None => out.extend(sub.elements().iter().copied()),
        }
    }
    out
}

/// The segments of a closed subpath without the sweep's noise: pieces shorter than `weld` dropped,
/// each segment starting where the one before ends, and straight runs merged into one line.
fn clean(sub: &BezPath, weld: f64) -> Vec<Seg> {
    let mut segs: Vec<Seg> = sub
        .segments()
        .map(|s| match s {
            PathSeg::Line(l) => Seg::Line(l.p0, l.p1),
            other => Seg::Cubic(other.to_cubic()),
        })
        .filter(|s| s.span() >= weld)
        .collect();
    let n = segs.len();
    for i in 0..n {
        let prev_end = segs.get((i + n - 1) % n).map(Seg::end);
        if let (Some(p), Some(s)) = (prev_end, segs.get_mut(i)) {
            s.set_start(p);
        }
    }
    merge_runs(segs, weld)
}

/// Two lines A-B and B-C become A-C when B lies within `weld` of that line and between A and C
/// (a distance test, so a long edge with a real slight kink is not straightened). Cyclic.
fn merge_runs(segs: Vec<Seg>, weld: f64) -> Vec<Seg> {
    let mergeable = |a: &Seg, b: &Seg| match (a, b) {
        (Seg::Line(p, q), Seg::Line(_, r)) => {
            let (d, e) = (*r - *p, *q - *p);
            let l = d.hypot();
            l > 1e-12 && (d.cross(e) / l).abs() <= weld && e.dot(*r - *q) > 0.0
        }
        _ => false,
    };
    let mut merged: Vec<Seg> = Vec::with_capacity(segs.len());
    for s in segs {
        match merged.last_mut() {
            Some(last) if mergeable(last, &s) => *last = Seg::Line(last.start(), s.end()),
            _ => merged.push(s),
        }
    }
    while merged.len() > 2 {
        let (Some(first), Some(last)) = (merged.first().copied(), merged.last().copied()) else { break };
        if !mergeable(&last, &first) {
            break;
        }
        if let Some(f) = merged.first_mut() {
            *f = Seg::Line(last.start(), first.end());
        }
        merged.pop();
    }
    merged
}

/// Everything the corner of one vertex needs. Vertex `i` is where segment `i - 1` ends and
/// segment `i` starts.
struct Ctx<'a> {
    segs: &'a [Seg],
    /// Segment lengths, measured only for segments with a corner at one end.
    lens: &'a [Option<f64>],
    /// Per vertex: the turn between its segments and the length it wants along each of them.
    wants: &'a [Option<(f64, f64)>],
    radius: f64,
    smoothing: f64,
    weld: f64,
}

/// The angle between the segments at each vertex (pi is straight on, 0 folded back) and the
/// length a corner there wants along each; `None` where the vertex is not a corner.
fn wants(segs: &[Seg], radius: f64, smoothing: f64) -> Vec<Option<(f64, f64)>> {
    let n = segs.len();
    (0..n)
        .map(|i| {
            let (a, b) = (segs.get((i + n - 1) % n)?, segs.get(i)?);
            let (u1, u2) = (start_dir(&a.outward(false))?, start_dir(&b.outward(true))?);
            let angle = u1.dot(u2).clamp(-1.0, 1.0).acos();
            if angle < 1e-6 || PI - angle < SHARP_TURN {
                return None;
            }
            let t = radius / (angle / 2.0).tan();
            let want = if a.is_line() && b.is_line() { (1.0 + smoothing) * t } else { t };
            want.is_finite().then_some((angle, want))
        })
        .collect()
}

impl Ctx<'_> {
    fn want(&self, i: usize) -> f64 {
        self.wants.get(i % self.segs.len().max(1)).copied().flatten().map_or(0.0, |(_, w)| w)
    }

    /// What a corner may use of a segment of `len`: half of it, or what the corner at its other
    /// end leaves, whichever is more.
    fn room(len: f64, other: f64) -> f64 {
        (len / 2.0).max(len - other)
    }

    fn corner(&self, i: usize) -> Option<Corner> {
        let n = self.segs.len();
        let (angle, _) = self.wants.get(i).copied().flatten()?;
        let prev = (i + n - 1) % n;
        let (a, b) = (self.segs.get(prev)?, self.segs.get(i)?);
        let (la, lb) = (self.lens.get(prev).copied().flatten()?, self.lens.get(i).copied().flatten()?);
        let room = Self::room(la, self.want(prev)).min(Self::room(lb, self.want(i + 1)));
        if !room.is_finite() || room <= self.weld {
            return None;
        }
        let (c1, c2) = (a.outward(false), b.outward(true));
        if a.is_line() && b.is_line() {
            self.between_lines(&c1, &c2, room, (la, lb))
        } else {
            self.beside_curve((&c1, a.is_line(), la), (&c2, b.is_line(), lb), room, angle)
        }
    }

    fn between_lines(&self, c1: &CubicBez, c2: &CubicBez, room: f64, (la, lb): (f64, f64)) -> Option<Corner> {
        let (u1, u2) = (start_dir(c1)?, start_dir(c2)?);
        let curves = figcraft_geom::smoothed_corner(c1.p0, u1, u2, room, self.radius, self.smoothing)?;
        let (first, last) = (curves.first()?, curves.last()?);
        let (t_in, t_out) = (1.0 - (first.p0 - c1.p0).hypot() / la, (last.p3 - c1.p0).hypot() / lb);
        (t_in.is_finite() && t_out.is_finite() && curves.iter().all(|c| [c.p0, c.p1, c.p2, c.p3].iter().all(|p| p.x.is_finite() && p.y.is_finite())))
            .then_some(Corner { curves, t_in, t_out })
    }

    /// One cubic between the cuts on both sides, its handles along each side's direction at its cut.
    fn beside_curve(
        &self,
        (c1, line1, la): (&CubicBez, bool, f64),
        (c2, line2, lb): (&CubicBez, bool, f64),
        room: f64,
        angle: f64,
    ) -> Option<Corner> {
        let t = (self.radius / (angle / 2.0).tan()).min(room);
        let ((p, d1, k1), (q, d2, k2)) = (trim(c1, t, line1, la)?, trim(c2, t, line2, lb)?);
        let turn = (-d1.dot(d2)).clamp(-1.0, 1.0).acos();
        let chord = (q - p).hypot();
        let k = if turn < 1e-9 { chord / 3.0 } else { 4.0 / 3.0 * (turn / 4.0).tan() * chord / (2.0 * (turn / 2.0).sin()) };
        let ok = k.is_finite() && [p, q].iter().all(|x| x.x.is_finite() && x.y.is_finite());
        ok.then(|| Corner { curves: vec![CubicBez::new(p, p + d1 * k, q + d2 * k, q)], t_in: 1.0 - k1, t_out: k2 })
    }
}

/// The subpath with its corners rounded, or `None` when no vertex of it is a corner that rounds
/// (the caller keeps the subpath exactly as it came in).
fn round_subpath(sub: &BezPath, radius: f64, smoothing: f64, weld: f64) -> Option<BezPath> {
    let segs = clean(sub, weld);
    let n = segs.len();
    if n < 2 {
        return None;
    }
    let wants = wants(&segs, radius, smoothing);
    let touches = |i: usize| wants.get(i).is_some_and(Option::is_some) || wants.get((i + 1) % n).is_some_and(Option::is_some);
    let lens: Vec<Option<f64>> = segs.iter().enumerate().map(|(i, s)| touches(i).then(|| s.len())).collect();
    let ctx = Ctx { segs: &segs, lens: &lens, wants: &wants, radius, smoothing, weld };
    let corners: Vec<Option<Corner>> = (0..n).map(|i| ctx.corner(i)).collect();
    corners.iter().any(Option::is_some).then(|| emit(&segs, &corners))?
}

/// Walk the segments: what is left of each between the corner at its start and the one at its
/// end, then the next corner's curves.
fn emit(segs: &[Seg], corners: &[Option<Corner>]) -> Option<BezPath> {
    let n = segs.len();
    let mut out = BezPath::new();
    let first = segs.first()?;
    out.move_to(corners.first().and_then(Option::as_ref).and_then(|c| c.curves.last()).map_or(first.start(), |c| c.p3));
    for (i, seg) in segs.iter().enumerate() {
        let next = corners.get((i + 1) % n).and_then(Option::as_ref);
        let t0 = corners.get(i).and_then(Option::as_ref).map_or(0.0, |c| c.t_out);
        let t1 = next.map_or(1.0, |c| c.t_in);
        if t1 - t0 > 1e-9 {
            match seg {
                Seg::Line(a, b) => out.line_to(a.lerp(*b, t1)),
                Seg::Cubic(c) => {
                    let q = c.subsegment(t0..t1);
                    out.curve_to(q.p1, q.p2, q.p3);
                }
            }
        }
        for c in next.iter().flat_map(|c| c.curves.iter()) {
            out.curve_to(c.p1, c.p2, c.p3);
        }
    }
    out.close_path();
    crate::finite(&out).then_some(out)
}
