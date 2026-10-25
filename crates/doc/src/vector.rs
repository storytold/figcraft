//! Vector networks: vertices joined by curved segments, any number of segments per vertex, plus
//! filled regions made of closed loops.

use figcraft_geom::kurbo::{CubicBez, ParamCurve, ParamCurveArclen, ParamCurveDeriv};
use figcraft_geom::{BezPath, Point, Vec2};
use serde::{Deserialize, Serialize};

use crate::{StrokeCap, StrokeJoin};

/// How a vertex's two bezier handles follow each other when one is dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum HandleMirroring {
    /// Handles move independently (a corner).
    #[default]
    None,
    /// The other handle stays opposite but keeps its own length.
    Angle,
    /// The other handle mirrors angle and length (a symmetric smooth point).
    AngleAndLength,
}

impl HandleMirroring {
    pub fn id(self) -> &'static str {
        match self {
            HandleMirroring::None => "none",
            HandleMirroring::Angle => "angle",
            HandleMirroring::AngleAndLength => "angleAndLength",
        }
    }
    pub fn from_id(s: &str) -> Option<Self> {
        [HandleMirroring::None, HandleMirroring::Angle, HandleMirroring::AngleAndLength].into_iter().find(|m| m.id().eq_ignore_ascii_case(s))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Vertex {
    pub x: f64,
    pub y: f64,
    /// Per-vertex corner radius (0 = sharp).
    #[serde(default)]
    pub radius: f64,
    /// Handle mirroring; `None` infers it from the handles (see [`VectorNetwork::mirroring`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror: Option<HandleMirroring>,
    /// The cap where a stroke ends at this point; `None` takes the layer's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cap: Option<StrokeCap>,
    /// The join where the stroke turns at this point; `None` takes the layer's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join: Option<StrokeJoin>,
}

impl Vertex {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y, radius: 0.0, mirror: None, cap: None, join: None }
    }
    pub fn point(&self) -> Point {
        Point::new(self.x, self.y)
    }
}

/// A segment from vertex `a` to `b` with tangent handles relative to each end vertex.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub a: usize,
    pub b: usize,
    #[serde(default)]
    pub ta: (f64, f64),
    #[serde(default)]
    pub tb: (f64, f64),
}

impl Segment {
    pub fn line(a: usize, b: usize) -> Self {
        Self { a, b, ta: (0.0, 0.0), tb: (0.0, 0.0) }
    }
    pub fn is_straight(&self) -> bool {
        self.ta == (0.0, 0.0) && self.tb == (0.0, 0.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum WindingRule {
    #[default]
    NonZero,
    EvenOdd,
}

/// A filled region: loops of segment indices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Region {
    pub loops: Vec<Vec<usize>>,
    #[serde(default)]
    pub winding: WindingRule,
}

/// A rounded point as drawn: point `v` moves to `a` (on segment `s1`), segment `s2` starts again
/// at `b`, and an arc with handles `ta`/`tb` joins them. `cut1`/`cut2`: where each segment is cut,
/// in its own parameter (curved segments keep the rest of their curve).
struct Corner {
    v: usize,
    s1: usize,
    s1_at_a: bool,
    s2: usize,
    s2_at_a: bool,
    a: Point,
    b: Point,
    ta: (f64, f64),
    tb: (f64, f64),
    cut1: f64,
    cut2: f64,
}

/// Accuracy of arc lengths along curved segments (local units).
const ARCLEN_ACC: f64 = 1e-6;

/// A run of the stroke (see [`VectorNetwork::stroke_runs`]): its path and the points it starts and
/// ends at (the same point when it is closed).
#[derive(Clone, Debug, PartialEq)]
pub struct StrokeRun {
    pub path: BezPath,
    pub start: usize,
    pub end: usize,
    pub closed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct VectorNetwork {
    pub vertices: Vec<Vertex>,
    pub segments: Vec<Segment>,
    #[serde(default)]
    pub regions: Vec<Region>,
}

/// Point the subpath's last segment (the only one ending at the fresh vertex `from`) at `to`,
/// so merging a duplicate endpoint doesn't rescan the segments of earlier subpaths.
fn redirect_last_segment(net: &mut VectorNetwork, loop_segs: &[usize], from: usize, to: usize) {
    if let Some(s) = loop_segs.last().and_then(|i| net.segments.get_mut(*i))
        && s.b == from
    {
        s.b = to;
    }
}

impl VectorNetwork {
    /// A network from a polyline / polygon.
    pub fn from_points(points: &[Point], closed: bool) -> Self {
        let vertices: Vec<Vertex> = points.iter().map(|p| Vertex::new(p.x, p.y)).collect();
        let n = vertices.len();
        let mut segments: Vec<Segment> = (1..n).map(|i| Segment::line(i - 1, i)).collect();
        let mut regions = Vec::new();
        if closed && n > 2 {
            segments.push(Segment::line(n - 1, 0));
            regions.push(Region { loops: vec![(0..segments.len()).collect()], winding: WindingRule::NonZero });
        }
        Self { vertices, segments, regions }
    }

    /// A network from a bezier path: each subpath becomes a chain (closed subpaths a loop + region).
    pub fn from_path(path: &BezPath) -> Self {
        use figcraft_geom::kurbo::PathEl;
        let mut net = VectorNetwork::default();
        let mut loop_segs: Vec<usize> = Vec::new();
        let mut first_v: Option<usize> = None;
        let mut cur_v: Option<usize> = None;
        let mut all_loops: Vec<Vec<usize>> = Vec::new();
        let push_v = |net: &mut VectorNetwork, p: Point| -> usize {
            net.vertices.push(Vertex::new(p.x, p.y));
            net.vertices.len() - 1
        };
        // A subpath that ends back on its start point without a ClosePath (e.g. ellipse outlines)
        // is closed too, so it keeps its fill.
        let close_open =
            |net: &mut VectorNetwork, loop_segs: &mut Vec<usize>, all_loops: &mut Vec<Vec<usize>>, first_v: Option<usize>, cur_v: Option<usize>| {
                let (Some(a), Some(f)) = (cur_v, first_v) else { return };
                let (Some(pa), Some(pf)) = (net.vertices.get(a).map(Vertex::point), net.vertices.get(f).map(Vertex::point)) else { return };
                if a == f || loop_segs.len() < 2 || (pa - pf).hypot() > 1e-6 || a + 1 != net.vertices.len() {
                    return;
                }
                redirect_last_segment(net, loop_segs, a, f);
                net.vertices.pop();
                all_loops.push(std::mem::take(loop_segs));
            };
        for el in path.elements() {
            match *el {
                PathEl::MoveTo(p) => {
                    close_open(&mut net, &mut loop_segs, &mut all_loops, first_v, cur_v);
                    loop_segs.clear();
                    let v = push_v(&mut net, p);
                    first_v = Some(v);
                    cur_v = Some(v);
                }
                PathEl::LineTo(p) => {
                    let Some(a) = cur_v else { continue };
                    let b = push_v(&mut net, p);
                    net.segments.push(Segment::line(a, b));
                    loop_segs.push(net.segments.len() - 1);
                    cur_v = Some(b);
                }
                PathEl::QuadTo(c, p) => {
                    let Some(a) = cur_v else { continue };
                    let Some(pa) = net.vertices.get(a).map(Vertex::point) else { continue };
                    let c1 = pa + (c - pa) * (2.0 / 3.0);
                    let c2 = p + (c - p) * (2.0 / 3.0);
                    let b = push_v(&mut net, p);
                    net.segments.push(Segment { a, b, ta: (c1.x - pa.x, c1.y - pa.y), tb: (c2.x - p.x, c2.y - p.y) });
                    loop_segs.push(net.segments.len() - 1);
                    cur_v = Some(b);
                }
                PathEl::CurveTo(c1, c2, p) => {
                    let Some(a) = cur_v else { continue };
                    let Some(pa) = net.vertices.get(a).map(Vertex::point) else { continue };
                    let b = push_v(&mut net, p);
                    net.segments.push(Segment { a, b, ta: (c1.x - pa.x, c1.y - pa.y), tb: (c2.x - p.x, c2.y - p.y) });
                    loop_segs.push(net.segments.len() - 1);
                    cur_v = Some(b);
                }
                PathEl::ClosePath => {
                    let (Some(a), Some(f)) = (cur_v, first_v) else { continue };
                    let close_pt = net.vertices.get(a).map(Vertex::point);
                    let first_pt = net.vertices.get(f).map(Vertex::point);
                    // The end vertex lands on the start (up to float noise): merge them.
                    let same = matches!((close_pt, first_pt), (Some(c), Some(f)) if (c - f).hypot() <= 1e-6);
                    if same && a != f {
                        // Merge the duplicate end vertex into the first one.
                        redirect_last_segment(&mut net, &loop_segs, a, f);
                        if a + 1 == net.vertices.len() {
                            net.vertices.pop();
                        }
                    } else if a != f {
                        net.segments.push(Segment::line(a, f));
                        loop_segs.push(net.segments.len() - 1);
                    }
                    if !loop_segs.is_empty() {
                        all_loops.push(std::mem::take(&mut loop_segs));
                    }
                    cur_v = Some(f);
                }
            }
        }
        close_open(&mut net, &mut loop_segs, &mut all_loops, first_v, cur_v);
        if !all_loops.is_empty() {
            net.regions.push(Region { loops: all_loops, winding: WindingRule::NonZero });
        }
        net
    }

    /// Segments touching vertex `v`: (segment index, `true` when `v` is the segment's `a` end).
    pub fn incident(&self, v: usize) -> Vec<(usize, bool)> {
        self.segments
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                if s.a == v {
                    Some((i, true))
                } else if s.b == v {
                    Some((i, false))
                } else {
                    None
                }
            })
            .collect()
    }

    /// The handle (relative to its vertex) at one end of a segment.
    pub fn handle(&self, seg: usize, start: bool) -> Option<(f64, f64)> {
        let s = self.segments.get(seg)?;
        Some(if start { s.ta } else { s.tb })
    }

    pub fn set_handle(&mut self, seg: usize, start: bool, t: (f64, f64)) {
        if let Some(s) = self.segments.get_mut(seg) {
            if start {
                s.ta = t;
            } else {
                s.tb = t;
            }
        }
    }

    /// The vertex's handle mirroring: its explicit setting, else inferred from its two handles
    /// (opposite and equal → angle and length, opposite → angle, otherwise none).
    pub fn mirroring(&self, v: usize) -> HandleMirroring {
        if let Some(m) = self.vertices.get(v).and_then(|x| x.mirror) {
            return m;
        }
        let inc = self.incident(v);
        let [(s1, a1), (s2, a2)] = inc.as_slice() else { return HandleMirroring::None };
        let (Some(h1), Some(h2)) = (self.handle(*s1, *a1), self.handle(*s2, *a2)) else { return HandleMirroring::None };
        let (l1, l2) = (h1.0.hypot(h1.1), h2.0.hypot(h2.1));
        if l1 < 1e-9 || l2 < 1e-9 {
            return HandleMirroring::None;
        }
        // Opposite directions: the cross product is ~0 and the dot product negative.
        let cross = (h1.0 * h2.1 - h1.1 * h2.0) / (l1 * l2);
        let dot = (h1.0 * h2.0 + h1.1 * h2.1) / (l1 * l2);
        if cross.abs() > 1e-3 || dot > -0.999 {
            HandleMirroring::None
        } else if (l1 - l2).abs() <= 1e-6 * l1.max(l2).max(1.0) {
            HandleMirroring::AngleAndLength
        } else {
            HandleMirroring::Angle
        }
    }

    /// Segment `i` as a cubic bezier.
    pub fn cubic(&self, i: usize) -> Option<figcraft_geom::kurbo::CubicBez> {
        let (a, c1, c2, b) = self.seg_points(self.segments.get(i)?)?;
        Some(figcraft_geom::kurbo::CubicBez::new(a, c1, c2, b))
    }

    /// One closed loop of segments as a path.
    pub fn loop_path(&self, lp: &[usize]) -> BezPath {
        let mut path = BezPath::new();
        self.append_loop(&mut path, lp);
        path
    }

    /// Whether the rendered fill covers `p` (the renderer fills every region with one rule: even-odd
    /// when any region asks for it).
    pub fn fill_contains(&self, p: Point) -> bool {
        use figcraft_geom::Shape;
        let path = self.fill_path();
        let w = path.winding(p);
        if self.regions.iter().any(|r| r.winding == WindingRule::EvenOdd) { w % 2 != 0 } else { w != 0 }
    }

    fn seg_points(&self, s: &Segment) -> Option<(Point, Point, Point, Point)> {
        let a = self.vertices.get(s.a)?.point();
        let b = self.vertices.get(s.b)?.point();
        Some((a, Point::new(a.x + s.ta.0, a.y + s.ta.1), Point::new(b.x + s.tb.0, b.y + s.tb.1), b))
    }

    fn push_seg(&self, path: &mut BezPath, s: &Segment, reversed: bool) {
        let Some((a, c1, c2, b)) = self.seg_points(s) else { return };
        let (c1, c2, b) = if reversed { (c2, c1, a) } else { (c1, c2, b) };
        if s.is_straight() {
            path.line_to(b);
        } else {
            path.curve_to(c1, c2, b);
        }
    }

    /// The network with its rounded points drawn in: each sharp point with a corner radius becomes
    /// an arc between its two segments (the radius shrinks so the arc fits in half of each
    /// segment, like a polygon's corners). A curved segment is cut where the arc meets it and keeps
    /// the rest of its curve, the arc leaving along its direction there. `None` when no point is
    /// rounded.
    pub fn rounded(&self) -> Option<VectorNetwork> {
        if !self.vertices.iter().any(|v| v.radius > 0.0) {
            return None;
        }
        let mut incident: Vec<Vec<(usize, bool)>> = vec![Vec::new(); self.vertices.len()];
        for (i, s) in self.segments.iter().enumerate() {
            for (v, at_a) in [(s.a, true), (s.b, false)] {
                if let Some(l) = incident.get_mut(v) {
                    l.push((i, at_a));
                }
            }
        }
        let corners: Vec<Corner> = (0..self.vertices.len()).filter_map(|v| self.corner(v, incident.get(v)?)).collect();
        let mut out = self.clone();
        for v in &mut out.vertices {
            v.radius = 0.0;
        }
        // Curved segments keep the part of their curve between the cuts at their ends.
        let mut cuts: Vec<(f64, f64)> = vec![(0.0, 1.0); self.segments.len()];
        for c in &corners {
            for (s, at_a, t) in [(c.s1, c.s1_at_a, c.cut1), (c.s2, c.s2_at_a, c.cut2)] {
                if let Some(k) = cuts.get_mut(s) {
                    if at_a {
                        k.0 = t;
                    } else {
                        k.1 = t;
                    }
                }
            }
        }
        for (i, &(t0, t1)) in cuts.iter().enumerate() {
            let (Some(c), Some(s)) = (self.cubic(i), out.segments.get_mut(i)) else { continue };
            if (t0, t1) == (0.0, 1.0) || s.is_straight() {
                continue;
            }
            let q = c.subsegment(t0..t1);
            s.ta = (q.p1.x - q.p0.x, q.p1.y - q.p0.y);
            s.tb = (q.p2.x - q.p3.x, q.p2.y - q.p3.y);
        }
        let mut by_vertex: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
        for c in corners {
            if let Some(v) = out.vertices.get_mut(c.v) {
                (v.x, v.y) = (c.a.x, c.a.y);
            }
            out.vertices.push(Vertex::new(c.b.x, c.b.y));
            let nb = out.vertices.len() - 1;
            if let Some(s) = out.segments.get_mut(c.s2) {
                if c.s2_at_a {
                    s.a = nb;
                } else {
                    s.b = nb;
                }
            }
            out.segments.push(Segment { a: c.v, b: nb, ta: c.ta, tb: c.tb });
            let arc = out.segments.len() - 1;
            by_vertex.insert(c.v, arc);
        }
        // The arc goes between the two segments wherever a loop walks through the point. Each loop
        // is rebuilt in one pass, so a polygon with many rounded corners isn't quadratic.
        for lp in out.regions.iter_mut().flat_map(|r| r.loops.iter_mut()) {
            let Some(&first_idx) = lp.first() else { continue };
            let start_v = match (self.segments.get(first_idx), lp.get(1).and_then(|&i| self.segments.get(i))) {
                (Some(first), Some(second)) if first.a == second.a || first.a == second.b => first.b,
                (Some(first), _) => first.a,
                _ => 0,
            };
            let mut rebuilt = Vec::with_capacity(lp.len() + by_vertex.len().min(lp.len()));
            let mut cur = start_v;
            for &x in lp.iter() {
                rebuilt.push(x);
                let Some(s) = self.segments.get(x) else { continue };
                cur = if s.a == cur { s.b } else { s.a };
                if let Some(&arc) = by_vertex.get(&cur) {
                    rebuilt.push(arc);
                }
            }
            *lp = rebuilt;
        }
        Some(out)
    }

    /// How point `v` (its `incident` segments) is rounded, if it is: not where the two segments
    /// run on smoothly or fold back.
    fn corner(&self, v: usize, incident: &[(usize, bool)]) -> Option<Corner> {
        let r = self.vertices.get(v)?.radius;
        let [(s1, a1), (s2, a2)] = *incident else { return None };
        if !(r > 0.0 && r.is_finite()) || s1 == s2 {
            return None;
        }
        let (c1, c2) = (self.outward(s1, a1)?, self.outward(s2, a2)?);
        let (u1, u2) = (start_dir(&c1)?, start_dir(&c2)?);
        let angle = u1.dot(u2).clamp(-1.0, 1.0).acos();
        if angle < 1e-6 || std::f64::consts::PI - angle < 1e-6 {
            return None;
        }
        let (l1, l2) = (length(&c1), length(&c2));
        if !(l1 > 1e-9 && l2 > 1e-9) {
            return None;
        }
        let t = (r / (angle / 2.0).tan()).min(l1.min(l2) / 2.0);
        let ((a, d1, k1), (b, d2, k2)) = (trim(&c1, t, l1), trim(&c2, t, l2));
        // A cubic arc: handles of 4/3·tan(φ/4)·ρ along each end's direction, φ the turn between
        // them and ρ the radius of the circle through both ends (the corner radius between lines).
        let turn = (-d1.dot(d2)).clamp(-1.0, 1.0).acos();
        let chord = (b - a).hypot();
        let k = if turn < 1e-9 { chord / 3.0 } else { 4.0 / 3.0 * (turn / 4.0).tan() * chord / (2.0 * (turn / 2.0).sin()) };
        if !(k.is_finite() && a.x.is_finite() && a.y.is_finite() && b.x.is_finite() && b.y.is_finite()) {
            return None;
        }
        let own = |at_a: bool, t: f64| if at_a { t } else { 1.0 - t };
        Some(Corner {
            v,
            s1,
            s1_at_a: a1,
            s2,
            s2_at_a: a2,
            a,
            b,
            ta: (d1.x * k, d1.y * k),
            tb: (d2.x * k, d2.y * k),
            cut1: own(a1, k1),
            cut2: own(a2, k2),
        })
    }

    /// Segment `seg` as a cubic running away from its `a` end (`at_a`) or from its `b` end.
    fn outward(&self, seg: usize, at_a: bool) -> Option<CubicBez> {
        let c = self.cubic(seg)?;
        Some(if at_a { c } else { CubicBez::new(c.p3, c.p2, c.p1, c.p0) })
    }

    /// The unit direction segment `seg` leaves its `a` end (`at_a`) or its `b` end in.
    pub fn tangent_out(&self, seg: usize, at_a: bool) -> Option<Vec2> {
        start_dir(&self.outward(seg, at_a)?)
    }

    /// Whether any point has a stroke cap of its own or a join other than the layer's `layer_join`.
    pub fn has_point_styles(&self, layer_join: StrokeJoin) -> bool {
        self.vertices.iter().any(|v| v.cap.is_some() || v.join.is_some_and(|j| j != layer_join))
    }

    /// The fill path: every region's loops.
    pub fn fill_path(&self) -> BezPath {
        if let Some(r) = self.rounded() {
            return r.fill_path();
        }
        let mut path = BezPath::new();
        for region in &self.regions {
            for lp in &region.loops {
                self.append_loop(&mut path, lp);
            }
        }
        path
    }

    fn append_loop(&self, path: &mut BezPath, lp: &[usize]) {
        // Walk the loop, orienting each segment to continue from the previous end vertex.
        let segs: Vec<&Segment> = lp.iter().filter_map(|i| self.segments.get(*i)).collect();
        let Some(first) = segs.first() else { return };
        let start_v = match segs.get(1) {
            Some(second) if first.a == second.a || first.a == second.b => first.b,
            _ => first.a,
        };
        let Some(start) = self.vertices.get(start_v) else { return };
        path.move_to(start.point());
        let mut cur = start_v;
        for s in segs {
            if s.a == cur {
                self.push_seg(path, s, false);
                cur = s.b;
            } else {
                self.push_seg(path, s, true);
                cur = s.a;
            }
        }
        path.close_path();
    }

    /// The stroke path: segments chained into maximal open runs (vertices of degree 2 continue a
    /// run); closed loops come out closed.
    pub fn stroke_path(&self) -> BezPath {
        if let Some(r) = self.rounded() {
            return r.stroke_path();
        }
        let mut path = BezPath::new();
        for run in self.runs(&|_| false) {
            path.extend(run.path.elements().iter().copied());
        }
        path
    }

    /// The stroke as runs for points with caps and joins of their own (rounded points drawn in):
    /// like [`Self::stroke_path`], but a run also breaks at a point with a join of its own (left
    /// open there, so the join can be drawn on its own).
    pub fn stroke_runs(&self) -> Vec<StrokeRun> {
        if let Some(r) = self.rounded() {
            return r.stroke_runs();
        }
        self.runs(&|v| self.vertices.get(v).is_some_and(|x| x.join.is_some()))
    }

    /// Segments chained into runs: a run goes on through points of degree 2 unless `breaks` says
    /// to stop there; a run that comes back to its start without a break is closed.
    fn runs(&self, breaks: &dyn Fn(usize) -> bool) -> Vec<StrokeRun> {
        let mut out = Vec::new();
        let mut used = vec![false; self.segments.len()];
        let mut degrees = vec![0usize; self.vertices.len()];
        for s in &self.segments {
            for v in [s.a, s.b] {
                if let Some(d) = degrees.get_mut(v) {
                    *d += 1;
                }
            }
        }
        let degree = |v: usize| degrees.get(v).copied().unwrap_or(0);
        // Each vertex's segments in segment order, and how far down that list the used ones reach,
        // so finding the next unused segment never rescans the whole network.
        let mut adjacent: Vec<Vec<(usize, bool)>> = vec![Vec::new(); self.vertices.len()];
        for (i, s) in self.segments.iter().enumerate() {
            if let Some(l) = adjacent.get_mut(s.a) {
                l.push((i, false));
            }
            if s.b != s.a
                && let Some(l) = adjacent.get_mut(s.b)
            {
                l.push((i, true));
            }
        }
        let mut cursor = vec![0usize; self.vertices.len()];
        let mut remaining = self.segments.len();
        let mut next_from = |v: usize, used: &[bool]| -> Option<(usize, bool)> {
            let list = adjacent.get(v)?;
            let at = cursor.get_mut(v)?;
            while let Some(&(i, rev)) = list.get(*at) {
                if !used.get(i).copied().unwrap_or(true) {
                    return Some((i, rev));
                }
                *at += 1;
            }
            None
        };
        let stop = |v: usize| degree(v) != 2 || breaks(v);
        // Start runs at endpoints, junctions and breaks first, then whatever remains (pure loops).
        let mut starts: Vec<usize> = (0..self.vertices.len()).filter(|v| stop(*v)).collect();
        starts.extend((0..self.vertices.len()).filter(|v| !stop(*v)));
        for v0 in starts {
            if remaining == 0 {
                break;
            }
            while let Some((si, rev)) = next_from(v0, &used) {
                let Some(start) = self.vertices.get(v0) else { break };
                let mut path = BezPath::new();
                path.move_to(start.point());
                let mut cur = v0;
                let mut closed = false;
                let mut seg = Some((si, rev));
                while let Some((i, r)) = seg {
                    if let Some(u) = used.get_mut(i)
                        && !*u
                    {
                        *u = true;
                        remaining = remaining.saturating_sub(1);
                    }
                    let Some(s) = self.segments.get(i) else { break };
                    self.push_seg(&mut path, s, r);
                    cur = if r { s.a } else { s.b };
                    if cur == v0 && !breaks(v0) {
                        path.close_path();
                        closed = true;
                        break;
                    }
                    seg = if stop(cur) { None } else { next_from(cur, &used) };
                }
                out.push(StrokeRun { path, start: v0, end: cur, closed });
            }
        }
        out
    }

    /// Translate/scale every vertex and handle (used when the node is resized).
    pub fn scale(&mut self, sx: f64, sy: f64) {
        for v in &mut self.vertices {
            v.x *= sx;
            v.y *= sy;
        }
        for s in &mut self.segments {
            s.ta = (s.ta.0 * sx, s.ta.1 * sy);
            s.tb = (s.tb.0 * sx, s.tb.1 * sy);
        }
    }

    pub fn translate(&mut self, dx: f64, dy: f64) {
        for v in &mut self.vertices {
            v.x += dx;
            v.y += dy;
        }
    }

    /// Bounds of the curves.
    pub fn bounds(&self) -> Option<figcraft_geom::Rect> {
        use figcraft_geom::Shape;
        let p = self.stroke_path();
        if p.elements().is_empty() { None } else { Some(p.bounding_box()) }
    }

    /// Remove segments that reference missing vertices, and regions referencing missing segments.
    pub fn sanitize(&mut self) {
        let nv = self.vertices.len();
        let keep: Vec<bool> = self.segments.iter().map(|s| s.a < nv && s.b < nv).collect();
        if keep.iter().all(|k| *k) {
            let ns = self.segments.len();
            for r in &mut self.regions {
                r.loops.retain(|l| l.iter().all(|i| *i < ns));
            }
            return;
        }
        let mut remap = vec![usize::MAX; self.segments.len()];
        let mut out = Vec::new();
        for (i, s) in self.segments.iter().enumerate() {
            if keep.get(i).copied().unwrap_or(false) {
                if let Some(m) = remap.get_mut(i) {
                    *m = out.len();
                }
                out.push(*s);
            }
        }
        self.segments = out;
        for r in &mut self.regions {
            r.loops.retain(|l| l.iter().all(|i| remap.get(*i).is_some_and(|m| *m != usize::MAX)));
            for l in &mut r.loops {
                for i in l.iter_mut() {
                    *i = remap.get(*i).copied().unwrap_or(0);
                }
            }
        }
    }
}

/// The direction a curve leaves its start in: along its first handle, else toward its second
/// control point or its end; `None` for a curve of no length.
fn start_dir(c: &CubicBez) -> Option<Vec2> {
    [c.p1 - c.p0, c.p2 - c.p0, c.p3 - c.p0].into_iter().find(|d| d.hypot() > 1e-9).map(|d| d / d.hypot())
}

/// Whether a cubic is a straight segment (no handles).
fn is_line(c: &CubicBez) -> bool {
    c.p1 == c.p0 && c.p2 == c.p3
}

fn length(c: &CubicBez) -> f64 {
    if is_line(c) { (c.p3 - c.p0).hypot() } else { c.arclen(ARCLEN_ACC) }
}

/// The point `t` along curve `c` (`len` long) from its start, the unit direction back toward the
/// start there, and its parameter.
fn trim(c: &CubicBez, t: f64, len: f64) -> (Point, Vec2, f64) {
    if is_line(c) {
        let u = (c.p3 - c.p0) / len;
        return (c.p0 + u * t, -u, t / len);
    }
    let k = c.inv_arclen(t, ARCLEN_ACC).clamp(0.0, 1.0);
    let d = c.deriv().eval(k).to_vec2();
    let back = if d.hypot() > 1e-9 { -d / d.hypot() } else { start_dir(c).map(|u| -u).unwrap_or_default() };
    (c.eval(k), back, k)
}
