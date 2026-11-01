//! Variable width strokes: a path measured by length, and the outline of a stroke whose width
//! follows a [`WidthProfile`] along each subpath.
//!
//! The path is flattened into polylines (sampled densely enough for the width to vary smoothly),
//! each point is offset along the polyline's normals by the interpolated half-width of that side
//! (the two sides can differ: one-sided widths), the sides are joined (round, miter or bevel on the
//! outer side, through the centre on the inner side) and capped. The result may overlap itself;
//! fill it with the non-zero rule.

use figcraft_doc::{StrokeCap, StrokeJoin, StrokeProps, WidthProfile};
use figcraft_geom::kurbo::{self, BezPath, PathEl, Point, Shape as _, Vec2};
use figcraft_geom::measure::FLATTEN_TOL;
/// The most points the outline works with; longer paths fall back to a uniform stroke.
pub use figcraft_geom::measure::MAX_SAMPLES as MAX_WIDTH_SAMPLES;
pub use figcraft_geom::measure::{Polyline, polylines};

/// Every subpath gets at least this many width samples, so the width varies smoothly along long
/// straight runs.
const SAMPLES_PER_SUBPATH: f64 = 128.0;
/// The most dashes a dashed stroke makes ([`dashes_fit`]).
const MAX_DASHES: usize = 20_000;

/// The nearest point of the stroke path to `p`: (subpath index, t by length, distance).
pub fn nearest(lines: &[Polyline], p: Point) -> Option<(usize, f64, f64)> {
    lines.iter().enumerate().filter_map(|(i, l)| l.nearest(p).map(|(t, d)| (i, t, d))).min_by(|a, b| a.2.total_cmp(&b.2))
}

/// Half-widths at a sample: `[along the normal, against it]`. The normal is the direction turned
/// clockwise on screen (y down), so the first is the right side looking along the path.
type Half = [f64; 2];

/// The half-width of `h` on side `sigma` (+1 = along the normal).
fn on(h: Half, sigma: f64) -> f64 {
    if sigma > 0.0 { h[0] } else { h[1] }
}

/// One straight piece of an offset polyline: its ends, the half-widths there, its direction and
/// normal.
struct Seg {
    a: Point,
    b: Point,
    ha: Half,
    hb: Half,
    d: Vec2,
    n: Vec2,
}

#[derive(Clone, Copy)]
struct Style {
    join: StrokeJoin,
    miter_limit: f64,
    cap_start: StrokeCap,
    cap_end: StrokeCap,
}

/// Points of an arc around `c` from the direction of `from` turning by `sweep` radians, with the
/// radius going from `r0` to `r1` (ends excluded).
fn arc(out: &mut Vec<Point>, c: Point, from: Vec2, sweep: f64, r0: f64, r1: f64) {
    let r = r0.max(r1);
    if r <= FLATTEN_TOL || !sweep.is_finite() {
        return;
    }
    let step = 2.0 * (1.0 - FLATTEN_TOL / r).clamp(-1.0, 1.0).acos();
    let n = ((sweep.abs() / step.max(1e-3)).ceil() as usize).clamp(2, 64);
    let a0 = from.atan2();
    for k in 1..n {
        let f = k as f64 / n as f64;
        out.push(c + Vec2::from_angle(a0 + sweep * f) * (r0 + (r1 - r0) * f));
    }
}

/// The corner between segments `p` and `c` on side `sigma` (+1 = the normal's side).
fn join(out: &mut Vec<Point>, p: &Seg, c: &Seg, sigma: f64, style: Style) {
    let (phb, cha) = (on(p.hb, sigma), on(c.ha, sigma));
    let pe = p.b + p.n * (sigma * phb);
    let cs = c.a + c.n * (sigma * cha);
    out.push(pe);
    let (cross, dot) = (p.d.cross(c.d), p.d.dot(c.d));
    if cross.abs() < 1e-12 && dot > 0.0 {
        out.push(cs);
        return;
    }
    // Turning towards the normal puts that side inside the corner.
    let outer = if cross.abs() < 1e-12 { sigma < 0.0 } else { cross * sigma < 0.0 };
    let v = c.a;
    if outer {
        let theta = if cross.abs() < 1e-12 { std::f64::consts::PI } else { cross.atan2(dot) };
        match style.join {
            StrokeJoin::Round => arc(out, v, pe - v, theta, phb, cha),
            StrokeJoin::Miter => {
                let half = (theta / 2.0).cos();
                let bis = p.n + c.n;
                if half > 1e-6 && 1.0 / half <= style.miter_limit && bis.hypot() > 1e-9 {
                    out.push(v + bis * (sigma * (phb + cha) / 2.0 / half / bis.hypot()));
                }
            }
            StrokeJoin::Bevel => {}
        }
    } else {
        // Through the centre: keeps the winding right when segments are shorter than the width.
        out.push(v);
    }
    out.push(cs);
}

/// One side of the offset polyline (`sigma` = +1 along the normals, -1 against).
fn side(segs: &[Seg], sigma: f64, closed: bool, style: Style) -> Vec<Point> {
    let mut out = Vec::with_capacity(segs.len() * 3 + 2);
    for (j, s) in segs.iter().enumerate() {
        let prev = if j == 0 { closed.then(|| segs.last()).flatten() } else { segs.get(j - 1) };
        match prev {
            Some(p) => join(&mut out, p, s, sigma, style),
            None => out.push(s.a + s.n * (sigma * on(s.ha, sigma))),
        }
    }
    if !closed && let Some(l) = segs.last() {
        out.push(l.b + l.n * (sigma * on(l.hb, sigma)));
    }
    out
}

/// A cap at `c` facing `dir` (unit, outwards) from the `dir.turn_90()` side, `from` wide, to the
/// other side, `to` wide. Lopsided ends are capped around the middle of the end.
fn cap(out: &mut Vec<Point>, c: Point, dir: Vec2, from: f64, to: f64, kind: StrokeCap) {
    let n = dir.turn_90();
    let (m, r) = (c + n * ((from - to) / 2.0), (from + to) / 2.0);
    match kind.shape() {
        StrokeCap::Square => {
            out.push(m + n * r + dir * r);
            out.push(m - n * r + dir * r);
        }
        StrokeCap::Round => arc(out, m, n, -std::f64::consts::PI, r, r),
        _ => {}
    }
}

fn push_contour(path: &mut BezPath, pts: impl IntoIterator<Item = Point>) {
    let mut it = pts.into_iter();
    let Some(first) = it.next() else { return };
    path.move_to(first);
    for p in it {
        path.line_to(p);
    }
    path.close_path();
}

/// Outline one piece (a subpath or a dash) with its half-widths.
fn outline_piece(path: &mut BezPath, pts: &[Point], hw: &[Half], closed: bool, style: Style) {
    let segs: Vec<Seg> = pts
        .windows(2)
        .zip(hw.windows(2))
        .filter_map(|(p, h)| {
            let (a, b) = (p[0], p[1]);
            let len = (b - a).hypot();
            (len > 1e-9).then(|| {
                let d = (b - a) / len;
                Seg { a, b, ha: h[0], hb: h[1], d, n: d.turn_90() }
            })
        })
        .collect();
    let (Some(first), Some(last)) = (segs.first(), segs.last()) else {
        // A zero-length piece is a dot with round caps.
        let r = hw.first().map_or(0.0, |h| (h[0] + h[1]) / 2.0);
        if let Some(p) = pts.first()
            && style.cap_start.shape() == StrokeCap::Round
            && r > 0.0
        {
            path.extend(kurbo::Shape::path_elements(&kurbo::Circle::new(*p, r), FLATTEN_TOL));
        }
        return;
    };
    let left = side(&segs, 1.0, closed, style);
    let right = side(&segs, -1.0, closed, style);
    if closed {
        push_contour(path, left);
        push_contour(path, right.into_iter().rev());
        return;
    }
    let mut ring = left;
    cap(&mut ring, last.b, last.d, last.hb[0], last.hb[1], style.cap_end);
    ring.extend(right.into_iter().rev());
    cap(&mut ring, first.a, -first.d, first.ha[1], first.ha[0], style.cap_start);
    push_contour(path, ring);
}

/// Split a sampled polyline into dashes (`pattern` alternates dash and gap lengths), `offset` into
/// the pattern at its start.
fn dashes(pts: &[Point], hw: &[Half], pattern: &[f64], offset: f64) -> Vec<(Vec<Point>, Vec<Half>)> {
    let mut out = Vec::new();
    let Some(&first) = pattern.first() else { return out };
    let (mut idx, mut left, mut on) = (0usize, (first - offset).max(0.0), true);
    let mut cur: (Vec<Point>, Vec<Half>) = (pts.iter().take(1).copied().collect(), hw.iter().take(1).copied().collect());
    for (p, h) in pts.windows(2).zip(hw.windows(2)) {
        let (a, b, ha, hb) = (p[0], p[1], h[0], h[1]);
        let len = (b - a).hypot();
        let mut used = 0.0;
        while len - used > left {
            used += left;
            let f = if len > 0.0 { used / len } else { 0.0 };
            let (q, hq) = (a.lerp(b, f), [ha[0] + (hb[0] - ha[0]) * f, ha[1] + (hb[1] - ha[1]) * f]);
            if on {
                cur.0.push(q);
                cur.1.push(hq);
                out.push(std::mem::take(&mut cur));
                if out.len() >= MAX_DASHES {
                    return out;
                }
            } else {
                cur = (vec![q], vec![hq]);
            }
            on = !on;
            idx = (idx + 1) % pattern.len();
            left = pattern.get(idx).copied().unwrap_or(first);
        }
        left -= len - used;
        if on {
            cur.0.push(b);
            cur.1.push(hb);
        }
    }
    if on && !cur.0.is_empty() {
        out.push(cur);
    }
    out
}

/// The dash pattern a stroke uses (odd lengths repeat twice), or `None` for a solid stroke.
pub fn dash_pattern(props: &StrokeProps) -> Option<Vec<f64>> {
    let dashes: Vec<f64> = props.dashes.iter().copied().filter(|d| d.is_finite() && *d >= 0.0).collect();
    if dashes.is_empty() || dashes.iter().sum::<f64>() <= 0.01 {
        return None;
    }
    Some(if dashes.len() % 2 == 1 { [dashes.clone(), dashes].concat() } else { dashes })
}

/// How a line `len` long draws `pattern` (from [`dash_pattern`]): the pattern and how far into it
/// the line starts. An open line starts and ends with a half-length dash, so a whole number of
/// patterns fits it exactly, dashes and gaps scaled together, and it starts halfway through the
/// first dash. A closed line has no ends: it runs the pattern as it is.
pub fn line_dashes(len: f64, closed: bool, pattern: &[f64]) -> (Vec<f64>, f64) {
    let total: f64 = pattern.iter().sum();
    if closed || !(len > 1e-9 && total > 0.0) {
        return (pattern.to_vec(), 0.0);
    }
    // A hair under a whole number is that number: a pattern already fitted stays as it is, also
    // after a trip through a file that rounds it.
    let scale = len / ((len / total + 0.01).floor().max(1.0) * total);
    let fitted: Vec<f64> = pattern.iter().map(|d| d * scale).collect();
    let offset = fitted.first().map_or(0.0, |d| d / 2.0);
    (fitted, offset)
}

/// [`line_dashes`] for a subpath.
pub fn subpath_dashes(sub: &BezPath, pattern: &[f64]) -> (Vec<f64>, f64) {
    let closed = matches!(sub.elements().last(), Some(PathEl::ClosePath));
    line_dashes(sub.perimeter(DASH_ACCURACY), closed, pattern)
}

/// How exactly a subpath is measured to fit its dashes (the stroke's own tolerance).
const DASH_ACCURACY: f64 = 0.05;

/// Whether `pattern` (from [`dash_pattern`]) makes at most [`MAX_DASHES`] dashes along `len`;
/// past that a stroke draws solid instead of one outline per dash (#1060).
pub(crate) fn dashes_fit(len: f64, pattern: &[f64]) -> bool {
    len / pattern.iter().sum::<f64>() * (pattern.len() as f64) < MAX_DASHES as f64
}

/// Resample a polyline so no step is longer than `max_step`; returns the points and the length at
/// each, or `None` past the sample budget.
fn resample(line: &Polyline, max_step: f64, budget: &mut usize) -> Option<(Vec<Point>, Vec<f64>)> {
    let mut pts: Vec<Point> = Vec::with_capacity(line.points.len());
    let mut lens: Vec<f64> = Vec::with_capacity(line.points.len());
    for (i, p) in line.points.iter().enumerate() {
        let s = line.lengths.get(i).copied().unwrap_or(0.0);
        if let (Some(&prev), Some(&ps)) = (pts.last(), lens.last()) {
            let n = ((s - ps) / max_step).ceil().clamp(1.0, MAX_WIDTH_SAMPLES as f64) as usize;
            for k in 1..n {
                let f = k as f64 / n as f64;
                pts.push(Point::lerp(prev, *p, f));
                lens.push(ps + (s - ps) * f);
            }
        }
        pts.push(*p);
        lens.push(s);
        if pts.len() > *budget {
            return None;
        }
    }
    *budget -= pts.len();
    Some((pts, lens))
}

/// The filled outline of a stroke of `width` along `path` whose width follows `profile` (its
/// scales multiply `width`). `None` when the path is invalid or too long (the caller strokes it
/// uniformly instead).
pub fn variable_outline(path: &BezPath, width: f64, props: &StrokeProps, profile: &WidthProfile) -> Option<BezPath> {
    if !(width.is_finite() && width > 0.0) {
        return Some(BezPath::new());
    }
    let profile = profile.sanitized();
    let lines = polylines(path)?;
    let [cap_start, cap_end] = props.band_caps();
    let style = Style { join: props.join, miter_limit: props.miter_limit.clamp(1.0, 100.0), cap_start, cap_end };
    let pattern = dash_pattern(props);
    let mut budget = MAX_WIDTH_SAMPLES;
    let mut out = BezPath::new();
    for line in &lines {
        let len = line.length();
        if len <= 1e-9 {
            continue;
        }
        let (pts, lens) = resample(line, len / SAMPLES_PER_SUBPATH, &mut budget)?;
        let hw: Vec<Half> = lens
            .iter()
            .map(|s| {
                let (left, right) = profile.sides_at(s / len);
                [0.5 * width * right, 0.5 * width * left]
            })
            .collect();
        match &pattern {
            Some(pat) if dashes_fit(len, pat) => {
                let (pat, offset) = line_dashes(len, line.closed, pat);
                for (p, h) in dashes(&pts, &hw, &pat, offset) {
                    outline_piece(&mut out, &p, &h, false, style);
                }
            }
            _ => outline_piece(&mut out, &pts, &hw, line.closed, style),
        }
    }
    Some(out)
}
