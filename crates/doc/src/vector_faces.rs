//! Faces of a vector network: the closed areas between its segments, which the paint bucket fills
//! and unfills.
//!
//! Faces come from a half-edge walk of the planar graph: at each vertex the walk turns to the next
//! segment in angular order, which traces every bounded area once (positive signed area) and the
//! outside of each connected piece once (negative). Pieces nested inside a face become its holes.
//! Segments are assumed not to cross each other; crossing networks still give well-formed (if
//! approximate) faces.

use figcraft_geom::kurbo::{ParamCurve, ParamCurveDeriv, PathEl};
use figcraft_geom::{BezPath, Point, Rect, Shape, Vec2};

use crate::vector::{Region, VectorNetwork, WindingRule};

/// A bounded area: its outer loop first, then the outer loops of pieces nested inside it.
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    /// Loops of segment indices.
    pub loops: Vec<Vec<usize>>,
    /// Enclosed area (the outer loop's minus the holes').
    pub area: f64,
    /// The face's outline (all loops); fill it even-odd.
    pub path: BezPath,
}

impl Face {
    pub fn contains(&self, p: Point) -> bool {
        self.path.winding(p) % 2 != 0
    }

    /// The fill region for this face.
    pub fn region(&self) -> Region {
        Region { loops: self.loops.clone(), winding: if self.loops.len() > 1 { WindingRule::EvenOdd } else { WindingRule::NonZero } }
    }

    /// Some point strictly inside the face (not in a hole).
    pub fn interior_point(&self) -> Option<Point> {
        interior_point(&self.path)
    }
}

/// A traced cycle of half-edges (segment index, forward?).
struct Cycle {
    half_edges: Vec<(usize, bool)>,
    area: f64,
    component: usize,
}

impl VectorNetwork {
    /// Every bounded face, holes included.
    pub fn faces(&self) -> Vec<Face> {
        let cycles = self.cycles();
        // Each connected piece's outside: its most negative cycle.
        let mut outside: std::collections::BTreeMap<usize, &Cycle> = std::collections::BTreeMap::new();
        for c in cycles.iter().filter(|c| c.area < -AREA_EPS) {
            let e = outside.entry(c.component).or_insert(c);
            if c.area < e.area {
                *e = c;
            }
        }
        // A piece's outer boundary (loops + path), only for pieces that enclose any area.
        let boundaries: Vec<(usize, Vec<Vec<usize>>, BezPath, f64)> = outside
            .iter()
            .map(|(comp, c)| {
                let loops = self.split_loops(&c.half_edges);
                let path = self.loops_path(&loops);
                (*comp, loops, path, -c.area)
            })
            .filter(|(_, loops, _, _)| !loops.is_empty())
            .collect();
        let mut faces = Vec::new();
        for c in cycles.iter().filter(|c| c.area > AREA_EPS) {
            let loops = self.split_loops(&c.half_edges);
            if loops.is_empty() {
                continue;
            }
            let outer = self.loops_path(&loops);
            // Pieces directly inside this face (not inside another piece that is) are its holes.
            let inside: Vec<&(usize, Vec<Vec<usize>>, BezPath, f64)> = boundaries
                .iter()
                .filter(|(comp, ..)| *comp != c.component)
                .filter(|(comp, ..)| self.component_point(&cycles, *comp).is_some_and(|p| outer.winding(p) % 2 != 0))
                .collect();
            let mut face = Face { loops, area: c.area, path: outer };
            for (comp, hl, hp, ha) in &inside {
                let nested = inside
                    .iter()
                    .any(|(other, _, op, _)| other != comp && self.component_point(&cycles, *comp).is_some_and(|p| op.winding(p) % 2 != 0));
                if nested {
                    continue;
                }
                face.loops.extend(hl.iter().cloned());
                face.path.extend(hp.elements().iter().copied());
                face.area -= ha;
            }
            faces.push(face);
        }
        faces
    }

    /// The smallest face containing `p`.
    pub fn face_at(&self, p: Point) -> Option<Face> {
        self.faces().into_iter().filter(|f| f.contains(p)).min_by(|a, b| a.area.total_cmp(&b.area))
    }

    /// The paint bucket: fill the face under `p`, or unfill it when it's filled. Returns the new
    /// state (`Some(true)` = filled), or `None` when `p` isn't inside a closed area.
    ///
    /// Regions that aren't single faces (e.g. from a converted shape or an import) are first
    /// re-expressed as one region per filled face, so any face can be toggled on its own.
    pub fn paint(&mut self, p: Point) -> Option<bool> {
        self.paint_to(p, None)
    }

    /// Like [`Self::paint`], but with `want` the face ends up filled (`Some(true)`) or unfilled
    /// (`Some(false)`) whatever it was: a paint bucket drag keeps doing what its first click did.
    pub fn paint_to(&mut self, p: Point, want: Option<bool>) -> Option<bool> {
        let face = self.face_at(p)?;
        let key = |loops: &[Vec<usize>]| -> Vec<Vec<usize>> {
            let mut k: Vec<Vec<usize>> = loops
                .iter()
                .map(|l| {
                    let mut l = l.clone();
                    l.sort_unstable();
                    l
                })
                .collect();
            k.sort();
            k
        };
        let faces = self.faces();
        let face_keys: Vec<Vec<Vec<usize>>> = faces.iter().map(|f| key(&f.loops)).collect();
        if !self.regions.iter().all(|r| face_keys.contains(&key(&r.loops))) {
            let filled: Vec<Region> = faces.iter().filter(|f| f.interior_point().is_some_and(|q| self.fill_contains(q))).map(Face::region).collect();
            self.regions = filled;
        }
        let k = key(&face.loops);
        match (self.regions.iter().position(|r| key(&r.loops) == k), want) {
            (Some(_), None | Some(false)) => {
                self.regions.retain(|r| key(&r.loops) != k);
                Some(false)
            }
            (Some(_), Some(true)) => Some(true),
            (None, Some(false)) => Some(false),
            (None, _) => {
                self.regions.push(face.region());
                Some(true)
            }
        }
    }

    /// Trace every half-edge cycle.
    fn cycles(&self) -> Vec<Cycle> {
        let n = self.segments.len();
        let nv = self.vertices.len();
        let valid = |i: usize| self.segments.get(i).is_some_and(|s| s.a < nv && s.b < nv);
        // Outgoing half-edges per vertex, sorted by angle.
        let mut out: Vec<Vec<(f64, f64, usize, bool)>> = vec![Vec::new(); nv];
        for i in (0..n).filter(|i| valid(*i)) {
            for fwd in [true, false] {
                let Some((origin, dir, along)) = self.half_edge_dir(i, fwd) else { continue };
                if let Some(list) = out.get_mut(origin) {
                    list.push((dir.y.atan2(dir.x), along.y.atan2(along.x), i, fwd));
                }
            }
        }
        for list in &mut out {
            list.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        }
        let origin = |i: usize, fwd: bool| self.segments.get(i).map(|s| if fwd { s.a } else { s.b });
        let dest = |i: usize, fwd: bool| self.segments.get(i).map(|s| if fwd { s.b } else { s.a });
        // The walk: arrive at v along (i, fwd); leave by the edge before the twin in angular order.
        let next = |i: usize, fwd: bool| -> Option<(usize, bool)> {
            let v = dest(i, fwd)?;
            let list = out.get(v)?;
            let k = list.iter().position(|e| e.2 == i && e.3 != fwd)?;
            let e = list.get((k + list.len() - 1) % list.len())?;
            Some((e.2, e.3))
        };
        let comp = self.components();
        let idx = |i: usize, fwd: bool| i * 2 + usize::from(!fwd);
        let mut seen = vec![false; n * 2];
        let mut cycles = Vec::new();
        for i in (0..n).filter(|i| valid(*i)) {
            for fwd in [true, false] {
                if seen.get(idx(i, fwd)).copied().unwrap_or(true) {
                    continue;
                }
                let mut half_edges = Vec::new();
                let mut cur = (i, fwd);
                let mut closed = false;
                for _ in 0..=n * 2 {
                    if let Some(s) = seen.get_mut(idx(cur.0, cur.1)) {
                        *s = true;
                    }
                    half_edges.push(cur);
                    let Some(nx) = next(cur.0, cur.1) else { break };
                    if nx == (i, fwd) {
                        closed = true;
                        break;
                    }
                    if seen.get(idx(nx.0, nx.1)).copied().unwrap_or(true) {
                        break;
                    }
                    cur = nx;
                }
                if !closed {
                    continue;
                }
                let area = self.half_edge_path(&half_edges).area();
                let component = origin(i, fwd).and_then(|v| comp.get(v).copied()).unwrap_or(0);
                cycles.push(Cycle { half_edges, area: if area.is_finite() { area } else { 0.0 }, component });
            }
        }
        cycles
    }

    /// Where half-edge (i, fwd) starts, its initial direction and a direction a little along it
    /// (tie-break for tangent curves).
    fn half_edge_dir(&self, i: usize, fwd: bool) -> Option<(usize, Vec2, Vec2)> {
        let s = self.segments.get(i)?;
        let c = self.cubic(i)?;
        let (origin, p0, d, along) = if fwd {
            (s.a, c.p0, c.deriv().eval(0.0).to_vec2(), c.eval(0.25) - c.p0)
        } else {
            (s.b, c.p3, -c.deriv().eval(1.0).to_vec2(), c.eval(0.75) - c.p3)
        };
        let fallback = if fwd { [c.p2 - p0, c.p3 - p0] } else { [c.p1 - p0, c.p0 - p0] };
        // A zero-length segment gets a nominal direction; the walk stays a permutation, so its
        // cycles still close (and its angle is irrelevant at the usual two-segment vertex).
        let nominal = if fwd { Vec2::new(1.0, 0.0) } else { Vec2::new(-1.0, 0.0) };
        let dir = std::iter::once(d).chain(fallback).find(|v| v.hypot() > 1e-9).unwrap_or(nominal);
        Some((origin, dir, if along.hypot() > 1e-9 { along } else { dir }))
    }

    fn half_edge_path(&self, half_edges: &[(usize, bool)]) -> BezPath {
        let mut path = BezPath::new();
        for (k, (i, fwd)) in half_edges.iter().enumerate() {
            let Some(c) = self.cubic(*i) else { continue };
            let c = if *fwd { c } else { figcraft_geom::kurbo::CubicBez::new(c.p3, c.p2, c.p1, c.p0) };
            if k == 0 {
                path.move_to(c.p0);
            }
            path.curve_to(c.p1, c.p2, c.p3);
        }
        path.close_path();
        path
    }

    /// A cycle's segments as closed loops: segments walked twice (dangling paths, bridges between
    /// loops) are dropped and what's left is chained into loops.
    fn split_loops(&self, half_edges: &[(usize, bool)]) -> Vec<Vec<usize>> {
        let count = |i: usize| half_edges.iter().filter(|h| h.0 == i).count();
        let kept: Vec<(usize, bool)> = half_edges.iter().copied().filter(|h| count(h.0) == 1).collect();
        let ends = |h: (usize, bool)| self.segments.get(h.0).map(|s| if h.1 { (s.a, s.b) } else { (s.b, s.a) });
        let mut used = vec![false; kept.len()];
        let mut loops = Vec::new();
        for start in 0..kept.len() {
            if used.get(start).copied().unwrap_or(true) {
                continue;
            }
            let Some(first) = kept.get(start).copied() else { continue };
            let Some((v0, mut v)) = ends(first) else { continue };
            if let Some(u) = used.get_mut(start) {
                *u = true;
            }
            let mut lp = vec![first.0];
            let mut pos = start;
            while v != v0 {
                // Prefer the next half-edge in walk order that continues from `v`.
                let found = (1..=kept.len())
                    .map(|d| (pos + d) % kept.len())
                    .find(|j| !used.get(*j).copied().unwrap_or(true) && kept.get(*j).and_then(|h| ends(*h)).is_some_and(|(a, _)| a == v));
                let Some(j) = found else { break };
                if let Some(u) = used.get_mut(j) {
                    *u = true;
                }
                let Some(h) = kept.get(j).copied() else { break };
                lp.push(h.0);
                v = ends(h).map(|e| e.1).unwrap_or(v0);
                pos = j;
            }
            if v == v0 {
                loops.push(lp);
            }
        }
        loops
    }

    fn loops_path(&self, loops: &[Vec<usize>]) -> BezPath {
        let mut path = BezPath::new();
        for lp in loops {
            path.extend(self.loop_path(lp).elements().iter().copied());
        }
        path
    }

    /// Connected piece of every vertex.
    fn components(&self) -> Vec<usize> {
        let nv = self.vertices.len();
        let mut parent: Vec<usize> = (0..nv).collect();
        fn find(parent: &mut [usize], mut x: usize) -> usize {
            for _ in 0..parent.len() {
                let p = parent.get(x).copied().unwrap_or(x);
                if p == x {
                    break;
                }
                let gp = parent.get(p).copied().unwrap_or(p);
                if let Some(px) = parent.get_mut(x) {
                    *px = gp;
                }
                x = gp;
            }
            x
        }
        for s in &self.segments {
            if s.a < nv && s.b < nv {
                let (ra, rb) = (find(&mut parent, s.a), find(&mut parent, s.b));
                if let Some(r) = parent.get_mut(ra) {
                    *r = rb;
                }
            }
        }
        (0..nv).map(|v| find(&mut parent, v)).collect()
    }

    /// A vertex of piece `comp` (for nesting tests).
    fn component_point(&self, cycles: &[Cycle], comp: usize) -> Option<Point> {
        let c = cycles.iter().find(|c| c.component == comp)?;
        let (i, fwd) = c.half_edges.first().copied()?;
        let s = self.segments.get(i)?;
        self.vertices.get(if fwd { s.a } else { s.b }).map(|v| v.point())
    }
}

const AREA_EPS: f64 = 1e-9;

/// A point inside a closed path under the even-odd rule: the middle of the widest span on a few
/// horizontal scanlines.
pub fn interior_point(path: &BezPath) -> Option<Point> {
    let bb: Rect = path.bounding_box();
    if !(bb.width() > 0.0 && bb.height() > 0.0 && bb.width().is_finite() && bb.height().is_finite()) {
        return None;
    }
    let mut lines: Vec<(Point, Point)> = Vec::new();
    let (mut start, mut last) = (Point::ZERO, Point::ZERO);
    figcraft_geom::kurbo::flatten(path.iter(), bb.width().max(bb.height()) * 1e-3, |el| match el {
        PathEl::MoveTo(p) => {
            start = p;
            last = p;
        }
        PathEl::LineTo(p) => {
            lines.push((last, p));
            last = p;
        }
        PathEl::ClosePath => {
            lines.push((last, start));
            last = start;
        }
        _ => {}
    });
    let mut best: Option<(f64, Point)> = None;
    for k in [0.5, 0.3, 0.7, 0.15, 0.85, 0.05, 0.95] {
        // An irrational nudge keeps the scanline off vertices.
        let y = bb.y0 + bb.height() * (k + 0.000_123_456_7);
        let mut xs: Vec<f64> = lines
            .iter()
            .filter(|(p, q)| (p.y <= y && y < q.y) || (q.y <= y && y < p.y))
            .map(|(p, q)| p.x + (y - p.y) * (q.x - p.x) / (q.y - p.y))
            .filter(|x| x.is_finite())
            .collect();
        xs.sort_by(f64::total_cmp);
        for pair in xs.chunks_exact(2) {
            let (Some(a), Some(b)) = (pair.first(), pair.get(1)) else { continue };
            let w = b - a;
            if w > best.map(|b| b.0).unwrap_or(1e-9) {
                best = Some((w, Point::new((a + b) / 2.0, y)));
            }
        }
    }
    best.map(|b| b.1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector::{Segment, Vertex};

    fn square(x: f64, y: f64, s: f64) -> VectorNetwork {
        VectorNetwork::from_points(&[Point::new(x, y), Point::new(x + s, y), Point::new(x + s, y + s), Point::new(x, y + s)], true)
    }

    fn merge(a: &VectorNetwork, b: &VectorNetwork) -> VectorNetwork {
        let mut n = a.clone();
        let (nv, ns) = (n.vertices.len(), n.segments.len());
        n.vertices.extend(b.vertices.iter().copied());
        n.segments.extend(b.segments.iter().map(|s| Segment { a: s.a + nv, b: s.b + nv, ..*s }));
        n.regions
            .extend(b.regions.iter().map(|r| Region { loops: r.loops.iter().map(|l| l.iter().map(|i| i + ns).collect()).collect(), ..r.clone() }));
        n
    }

    #[test]
    fn square_has_one_face_and_the_bucket_toggles_it() {
        let mut n = square(0.0, 0.0, 100.0);
        n.regions.clear();
        let f = n.faces();
        assert_eq!(f.len(), 1);
        assert!((f[0].area - 10000.0).abs() < 1e-6);
        assert_eq!(n.paint(Point::new(50.0, 50.0)), Some(true));
        assert!(n.fill_contains(Point::new(50.0, 50.0)));
        assert_eq!(n.paint(Point::new(50.0, 50.0)), Some(false));
        assert!(n.regions.is_empty());
        assert_eq!(n.paint(Point::new(150.0, 50.0)), None, "outside every face");
    }

    #[test]
    fn a_diagonal_splits_the_square_into_two_faces() {
        let mut n = square(0.0, 0.0, 100.0);
        n.regions.clear();
        n.segments.push(Segment::line(0, 2));
        let f = n.faces();
        assert_eq!(f.len(), 2);
        assert!(f.iter().all(|f| (f.area - 5000.0).abs() < 1e-6));
        assert_eq!(n.paint(Point::new(75.0, 25.0)), Some(true));
        assert!(n.fill_contains(Point::new(75.0, 25.0)));
        assert!(!n.fill_contains(Point::new(25.0, 75.0)));
    }

    #[test]
    fn unfilling_part_of_a_converted_shape_keeps_the_rest() {
        // A filled square (one region over the outline) cut by a diagonal.
        let mut n = square(0.0, 0.0, 100.0);
        n.segments.push(Segment::line(0, 2));
        assert!(n.fill_contains(Point::new(75.0, 25.0)));
        assert_eq!(n.paint(Point::new(75.0, 25.0)), Some(false));
        assert!(!n.fill_contains(Point::new(75.0, 25.0)));
        assert!(n.fill_contains(Point::new(25.0, 75.0)));
    }

    #[test]
    fn nested_pieces_become_holes() {
        let mut outer = square(0.0, 0.0, 100.0);
        outer.regions.clear();
        let mut inner = square(25.0, 25.0, 50.0);
        inner.regions.clear();
        let mut n = merge(&outer, &inner);
        let mut areas: Vec<f64> = n.faces().iter().map(|f| f.area).collect();
        areas.sort_by(f64::total_cmp);
        assert_eq!(areas.len(), 2);
        assert!((areas[0] - 2500.0).abs() < 1e-6 && (areas[1] - 7500.0).abs() < 1e-6, "{areas:?}");
        assert_eq!(n.paint(Point::new(10.0, 10.0)), Some(true));
        assert!(n.fill_contains(Point::new(10.0, 10.0)));
        assert!(!n.fill_contains(Point::new(50.0, 50.0)), "the ring has a hole");
        assert_eq!(n.paint(Point::new(50.0, 50.0)), Some(true));
        assert!(n.fill_contains(Point::new(50.0, 50.0)));
        // A third square inside the inner one is the inner face's hole, not the ring's.
        let mut core = square(45.0, 45.0, 10.0);
        core.regions.clear();
        let n3 = merge(&n, &core);
        let ring = n3.face_at(Point::new(10.0, 10.0)).unwrap();
        assert_eq!(ring.loops.len(), 2);
        assert!((ring.area - 7500.0).abs() < 1e-6);
        let mid = n3.face_at(Point::new(30.0, 30.0)).unwrap();
        assert!((mid.area - 2400.0).abs() < 1e-6);
    }

    #[test]
    fn dangling_paths_and_bridges_dont_break_faces() {
        let mut n = square(0.0, 0.0, 100.0);
        n.regions.clear();
        // A spur into the square.
        n.vertices.push(Vertex::new(50.0, 50.0));
        n.segments.push(Segment::line(0, 4));
        // A second square inside, bridged to the spur's end.
        let inner = square(60.0, 60.0, 20.0);
        let mut m = merge(&n, &inner);
        m.regions.clear();
        m.segments.push(Segment::line(4, 5));
        let f = m.face_at(Point::new(10.0, 90.0)).unwrap();
        assert!((f.area - (10000.0 - 400.0)).abs() < 1e-6, "{}", f.area);
        assert_eq!(f.loops.len(), 2);
        assert_eq!(m.paint(Point::new(10.0, 90.0)), Some(true));
        assert!(m.fill_contains(Point::new(10.0, 90.0)));
        assert!(!m.fill_contains(Point::new(70.0, 70.0)));
    }

    #[test]
    fn curves_and_junk_networks() {
        let mut circle = figcraft_geom::kurbo::Ellipse::new((50.0, 50.0), (50.0, 50.0), 0.0).to_path(0.01);
        circle.close_path();
        let mut n = VectorNetwork::from_path(&circle);
        let close = n.vertices.iter().enumerate().any(|(i, a)| n.vertices.iter().skip(i + 1).any(|b| (a.point() - b.point()).hypot() < 1e-3));
        assert!(!close, "the closing point merges into the first");
        n.regions.clear();
        eprintln!(
            "{:?}
{:?}",
            n.vertices, n.segments
        );
        for c in n.cycles() {
            eprintln!("cycle {:?} area {}", c.half_edges, c.area);
        }
        let f = n.faces();
        assert_eq!(f.len(), 1);
        assert!((f[0].area - std::f64::consts::PI * 2500.0).abs() < 5.0, "{}", f[0].area);
        // Without the ClosePath (kurbo's ellipse outline) it's still one closed, filled loop.
        let open = figcraft_geom::kurbo::Ellipse::new((50.0, 50.0), (50.0, 50.0), 0.0).to_path(0.01);
        let o = VectorNetwork::from_path(&open);
        assert_eq!(o.vertices.len(), n.vertices.len());
        assert_eq!(o.regions.len(), 1);
        assert!(o.fill_contains(Point::new(50.0, 50.0)));
        assert_eq!(n.paint(Point::new(50.0, 50.0)), Some(true));
        // Junk: NaN coordinates, dangling indices, self-loops, coincident points.
        let mut j = square(0.0, 0.0, 10.0);
        j.vertices.push(Vertex::new(f64::NAN, 1.0));
        j.segments.push(Segment::line(0, 4));
        j.segments.push(Segment::line(2, 99));
        j.segments.push(Segment::line(1, 1));
        j.segments.push(Segment { a: 3, b: 3, ta: (5.0, 5.0), tb: (-5.0, 5.0) });
        j.vertices.push(Vertex::new(0.0, 0.0));
        j.segments.push(Segment::line(0, 5));
        let _ = j.faces();
        let _ = j.paint(Point::new(5.0, 5.0));
        let _ = j.paint(Point::new(f64::NAN, 5.0));
        assert!(interior_point(&BezPath::new()).is_none());
    }

    #[test]
    fn the_bucket_unfills_a_face_with_duplicate_regions_in_one_click() {
        let c = Point::new(50.0, 50.0);
        let mut n = square(0.0, 0.0, 100.0);
        n.regions.push(n.regions[0].clone());
        assert_eq!(n.paint(c), Some(false));
        assert!(!n.fill_contains(c));
        assert_eq!(n.region_at(c), None);
        assert_eq!(n.paint(c), Some(true));
        assert!(n.fill_contains(c));

        let mut e = square(0.0, 0.0, 100.0);
        e.regions.push(e.regions[0].clone());
        assert_eq!(e.paint_to(c, Some(false)), Some(false));
        assert!(!e.fill_contains(c));
    }
}
