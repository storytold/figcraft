//! Paths measured by length: subpaths flattened into polylines with the length along them, for
//! anything placed along a path (variable width strokes, text on a path).

use kurbo::{BezPath, PathEl, Point, Vec2};

/// Flattening tolerance (local units).
pub const FLATTEN_TOL: f64 = 0.05;
/// The most flattened points a path is measured with; longer paths are refused.
pub const MAX_SAMPLES: usize = 40_000;

/// Every coordinate is finite and of a sane magnitude.
pub fn finite_path(path: &BezPath) -> bool {
    let ok = |p: &Point| p.x.is_finite() && p.y.is_finite() && p.x.abs() < 1e9 && p.y.abs() < 1e9;
    path.elements().iter().all(|el| match el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => ok(p),
        PathEl::QuadTo(a, b) => ok(a) && ok(b),
        PathEl::CurveTo(a, b, c) => ok(a) && ok(b) && ok(c),
        PathEl::ClosePath => true,
    })
}

/// One subpath flattened, with the length along it at each point. A closed subpath ends on its
/// start point.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Polyline {
    pub points: Vec<Point>,
    /// Cumulative length at each point (starts at 0).
    pub lengths: Vec<f64>,
    pub closed: bool,
}

impl Polyline {
    pub fn length(&self) -> f64 {
        self.lengths.last().copied().unwrap_or(0.0)
    }

    fn push(&mut self, p: Point) {
        let Some(last) = self.points.last().copied() else {
            self.points.push(p);
            self.lengths.push(0.0);
            return;
        };
        let d = (p - last).hypot();
        if d > 1e-9 {
            self.points.push(p);
            self.lengths.push(self.length() + d);
        }
    }

    /// The point at `t` (0..1 by length) and the unit normal there (the direction turned 90°).
    pub fn at(&self, t: f64) -> Option<(Point, Vec2)> {
        let len = self.length();
        if len <= 1e-9 || !t.is_finite() {
            return None;
        }
        let s = t.clamp(0.0, 1.0) * len;
        let i = self.lengths.partition_point(|l| *l <= s).clamp(1, self.points.len().saturating_sub(1).max(1));
        let (a, b) = (*self.points.get(i - 1)?, *self.points.get(i)?);
        let (la, lb) = (*self.lengths.get(i - 1)?, *self.lengths.get(i)?);
        let u = if lb - la > 1e-12 { ((s - la) / (lb - la)).clamp(0.0, 1.0) } else { 0.0 };
        let d = b - a;
        let n = d.turn_90() / d.hypot().max(1e-12);
        Some((a.lerp(b, u), n))
    }

    /// The nearest point to `p`: (t by length, distance).
    pub fn nearest(&self, p: Point) -> Option<(f64, f64)> {
        let len = self.length();
        if len <= 1e-9 {
            return None;
        }
        let mut best: Option<(f64, f64)> = None;
        for (i, w) in self.points.windows(2).enumerate() {
            let (a, b) = (w[0], w[1]);
            let d = b - a;
            let l2 = d.hypot2();
            let u = if l2 > 1e-18 { ((p - a).dot(d) / l2).clamp(0.0, 1.0) } else { 0.0 };
            let dist = (a.lerp(b, u) - p).hypot();
            if best.is_none_or(|(_, bd)| dist < bd) {
                let s = self.lengths.get(i).copied().unwrap_or(0.0) + u * l2.sqrt();
                best = Some((s / len, dist));
            }
        }
        best
    }
}

/// Every subpath of `path` flattened. `None` when the path has invalid coordinates or is too
/// long to work with.
pub fn polylines(path: &BezPath) -> Option<Vec<Polyline>> {
    polylines_with_limit(path, MAX_SAMPLES)
}

/// [`polylines`] with an explicit sample budget, so tests can use a small one.
///
/// The budget is enforced *while* flattening, not after it (issue #204): the input element
/// count is checked up front, each curve is flattened on its own, and flattening stops as
/// soon as the budget is exceeded. An over-budget path therefore costs at most one extra
/// segment of flattening work and never materialises more than `limit` retained points,
/// instead of building the complete flattened result only to throw it away.
fn polylines_with_limit(path: &BezPath, limit: usize) -> Option<Vec<Polyline>> {
    if !finite_path(path) {
        return None;
    }
    // Every element contributes at least one flattened sample, so a path with more
    // elements than the budget can be refused without flattening anything at all.
    if path.elements().len() > limit {
        return None;
    }
    fn finish(cur: &mut Polyline, out: &mut Vec<Polyline>) {
        let done = std::mem::take(cur);
        if done.points.len() >= 2 {
            out.push(done);
        }
    }
    let mut out: Vec<Polyline> = Vec::new();
    let mut cur = Polyline::default();
    let mut start = Point::ZERO;
    let mut pen = Point::ZERO;
    let mut count = 0usize;
    for el in path.elements() {
        if count > limit {
            return None;
        }
        match *el {
            PathEl::MoveTo(p) => {
                count += 1;
                finish(&mut cur, &mut out);
                start = p;
                pen = p;
                cur.push(p);
            }
            PathEl::LineTo(p) => {
                count += 1;
                if cur.points.is_empty() {
                    cur.push(start);
                }
                cur.push(p);
                pen = p;
            }
            PathEl::QuadTo(_, end) | PathEl::CurveTo(_, _, end) => {
                // Flatten this one segment on its own, so an over-budget curve aborts
                // here instead of after the whole path has been flattened.
                let seg_start = pen;
                let mut over = false;
                kurbo::flatten([PathEl::MoveTo(seg_start), *el], FLATTEN_TOL, |fel| {
                    if over {
                        return;
                    }
                    if let PathEl::LineTo(p) = fel {
                        count += 1;
                        if count > limit {
                            over = true;
                            return;
                        }
                        if cur.points.is_empty() {
                            cur.push(seg_start);
                        }
                        cur.push(p);
                    }
                });
                if over {
                    return None;
                }
                pen = end;
            }
            PathEl::ClosePath => {
                count += 1;
                cur.push(start);
                cur.closed = cur.points.len() >= 3;
                pen = start;
                finish(&mut cur, &mut out);
            }
        }
    }
    finish(&mut cur, &mut out);
    (count <= limit).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_path(n: usize) -> BezPath {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        for i in 1..=n {
            p.line_to((i as f64, 0.0));
        }
        p
    }

    #[test]
    fn normal_open_and_closed_paths_are_preserved() {
        let open = polylines_with_limit(&line_path(3), 100).expect("under budget");
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].points.len(), 4);
        assert!((open[0].length() - 3.0).abs() < 1e-9);
        assert!(!open[0].closed);

        let mut square = BezPath::new();
        square.move_to((0.0, 0.0));
        square.line_to((10.0, 0.0));
        square.line_to((10.0, 10.0));
        square.line_to((0.0, 10.0));
        square.close_path();
        let closed = polylines_with_limit(&square, 100).expect("under budget");
        assert_eq!(closed.len(), 1);
        assert!(closed[0].closed);
        assert!((closed[0].length() - 40.0).abs() < 1e-9);
    }

    #[test]
    fn over_budget_line_path_is_refused() {
        // 51 elements against a budget of 10: refused by the up-front element check.
        assert!(polylines_with_limit(&line_path(50), 10).is_none());
    }

    #[test]
    fn over_budget_curve_path_is_refused_early() {
        // One big cubic flattens to far more than 8 samples at FLATTEN_TOL; the
        // per-segment flattening must trip the budget and refuse the path.
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.curve_to((0.0, 1_000.0), (1_000.0, 1_000.0), (1_000.0, 0.0));
        assert!(polylines_with_limit(&p, 8).is_none());
        // The same curve is accepted with the production budget.
        assert!(polylines(&p).is_some());
    }

    #[test]
    fn curve_endpoints_and_length_still_measure() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.curve_to((0.0, 100.0), (100.0, 100.0), (100.0, 0.0));
        let lines = polylines(&p).expect("valid curve");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].points.first().copied(), Some(Point::new(0.0, 0.0)));
        assert_eq!(lines[0].points.last().copied(), Some(Point::new(100.0, 0.0)));
        // The flattened curve is longer than its chord and shorter than its control polygon.
        assert!(lines[0].length() > 100.0 && lines[0].length() < 300.0);
    }
}
