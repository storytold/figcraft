//! Path simplification that keeps to its input where kurbo's fit would shoot past it.

use kurbo::{BezPath, ParamCurve, ParamCurveNearest, PathEl, PathSeg, Point, Vec2};

/// How far, as a multiple of the accuracy asked for, a fitted run may stray from the run it
/// replaces before the run is kept as it was. kurbo's own fits stay within about the accuracy; its
/// failures stray by tens to hundreds of units, so twice the accuracy tells them apart without
/// undoing fits that are merely a little over.
const STRAY: f64 = 2.0;

/// `path` simplified to within `accuracy` by kurbo's [`simplify_bezpath`], kept from straying
/// far from `path` where kurbo's fit does.
///
/// Two inputs made kurbo's fit shoot far past the path it simplified (#1741, a Pencil stroke with
/// a spike off it), and both are common in pointer samples:
/// - A join where the path turns straight back (samples going down and back up the same line):
///   kurbo keeps two segments in one smooth fit when their tangents are parallel whichever way
///   they point (`|cross| <= |dot| * angle_thresh`), so it fits a smooth curve through the cusp.
///   The path is cut where it turns back by more than a right angle, each run between the cuts is
///   simplified on its own, and the runs are joined again, so the turn stays a sharp corner. Any
///   other join that isn't straight within kurbo's threshold is a corner to kurbo already, so the
///   cut changes nothing there.
/// - A run of nearly collinear segments: kurbo measures its error across the path, not along it, so
///   a fitted cubic whose handle runs hundreds of units on along the line passes its check. Each
///   fitted run is measured against the run it replaces, and one that strays more than
///   twice `accuracy` from it is kept as it was instead.
///
/// Subpaths and closes are kept, and a closing segment stays implicit, as kurbo leaves it.
///
/// [`simplify_bezpath`]: kurbo::simplify::simplify_bezpath
pub fn simplify_path(path: &BezPath, accuracy: f64) -> BezPath {
    let mut out = BezPath::new();
    let mut run = BezPath::new();
    // Where the current subpath started, where the pen is, and the last segment's end tangent.
    let mut start = Point::ZERO;
    let mut at = Point::ZERO;
    let mut last_tan: Option<Vec2> = None;
    // Whether the next run that draws anything has to start the subpath with its move.
    let mut fresh = true;
    for el in path.elements() {
        let seg = match *el {
            PathEl::MoveTo(p) => {
                flush(&mut out, &mut run, accuracy, &mut fresh);
                (start, at, last_tan, fresh) = (p, p, None, true);
                run.move_to(p);
                continue;
            }
            PathEl::ClosePath => {
                flush(&mut out, &mut run, accuracy, &mut fresh);
                if !fresh {
                    out.close_path();
                }
                // A segment after a close starts a new subpath at the closed one's start.
                (at, last_tan, fresh) = (start, None, true);
                run.move_to(start);
                continue;
            }
            PathEl::LineTo(p) => PathSeg::Line(kurbo::Line::new(at, p)),
            PathEl::QuadTo(p1, p2) => PathSeg::Quad(kurbo::QuadBez::new(at, p1, p2)),
            PathEl::CurveTo(p1, p2, p3) => PathSeg::Cubic(kurbo::CubicBez::new(at, p1, p2, p3)),
        };
        if run.elements().is_empty() {
            run.move_to(at);
        }
        let Some((t0, t1)) = tangents(seg) else { continue };
        if last_tan.is_some_and(|last| last.dot(t0) < 0.0) {
            flush(&mut out, &mut run, accuracy, &mut fresh);
            run.move_to(at);
        }
        run.push(*el);
        at = seg.end();
        last_tan = Some(t1);
    }
    flush(&mut out, &mut run, accuracy, &mut fresh);
    out
}

/// Simplify `run` (one move and the segments after it) with kurbo and append it to `out`; the
/// run's move is kept only when it starts a subpath (`fresh`), else the run carries on from the
/// last point drawn. `run` is left empty.
fn flush(out: &mut BezPath, run: &mut BezPath, accuracy: f64, fresh: &mut bool) {
    if run.elements().len() > 1 {
        let fitted = kurbo::simplify::simplify_bezpath(run.iter(), accuracy, &Default::default());
        let fitted = if keeps_to(&fitted, run, STRAY * accuracy) { fitted } else { std::mem::take(run) };
        for el in fitted.elements() {
            match el {
                PathEl::MoveTo(_) if !*fresh => {}
                _ => {
                    out.push(*el);
                    *fresh = false;
                }
            }
        }
    }
    *run = BezPath::new();
}

/// Whether every point of `fitted` lies within `tol` of `source`: one subpath each, from the same
/// start to the same end, `fitted` being kurbo's simplification of `source`.
///
/// kurbo ends each fitted segment on the source, so the two are walked together: for each fitted
/// segment, the source segments from where it starts to the first one its end lies on are the only
/// ones its sample points are measured against. The work is linear in the two lengths. A fitted end
/// found on no source segment counts as straying.
fn keeps_to(fitted: &BezPath, source: &BezPath, tol: f64) -> bool {
    /// Parts each fitted segment is cut into: the points between them are the ones measured.
    const SAMPLES: u32 = 16;
    let src: Vec<PathSeg> = source.segments().collect();
    // kurbo puts each end on the source by evaluating it: off by rounding at most.
    let on2 = (tol * 1e-3).powi(2);
    let (tol2, mut cur) = (tol * tol, 0);
    for seg in fitted.segments() {
        let Some(last) = (cur..src.len()).find(|&i| src.get(i).is_some_and(|s| s.nearest(seg.end(), 1e-9).distance_sq <= on2)) else {
            return false;
        };
        let near = src.get(cur..=last).unwrap_or_default();
        for i in 1..SAMPLES {
            let p = seg.eval(f64::from(i) / f64::from(SAMPLES));
            if !near.iter().any(|s| s.nearest(p, 1e-6).distance_sq <= tol2) {
                return false;
            }
        }
        cur = last;
    }
    true
}

/// The directions `seg` leaves its start and reaches its end, skipping control points that sit on
/// an end; `None` for a segment of no length, which kurbo skips too.
fn tangents(seg: PathSeg) -> Option<(Vec2, Vec2)> {
    let pts: Vec<Point> = match seg {
        PathSeg::Line(l) => vec![l.p0, l.p1],
        PathSeg::Quad(q) => vec![q.p0, q.p1, q.p2],
        PathSeg::Cubic(c) => vec![c.p0, c.p1, c.p2, c.p3],
    };
    let (first, last) = (*pts.first()?, *pts.last()?);
    let t0 = pts.iter().map(|p| *p - first).find(|d| d.hypot2() > 0.0)?;
    let t1 = pts.iter().rev().map(|p| last - *p).find(|d| d.hypot2() > 0.0)?;
    Some((t0, t1))
}
