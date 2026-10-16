//! Vector network edits in node-local coordinates: points, handles and mirroring, bending, adding,
//! deleting, cutting and joining. Pure functions on a [`VectorNetwork`]; the session wrappers in
//! `vectoredit.rs` re-fit the node afterwards.

use figcraft_doc::*;
use figcraft_geom::kurbo::{CubicBez, ParamCurve, ParamCurveNearest};
use figcraft_geom::{Affine, Point, Vec2};

fn tvec(t: (f64, f64)) -> Vec2 {
    Vec2::new(t.0, t.1)
}

fn finite(v: Vec2) -> bool {
    v.x.is_finite() && v.y.is_finite()
}

/// Move vertices by `d`; their handles move with them.
pub fn move_vertices(net: &mut VectorNetwork, idx: &[usize], d: Vec2) {
    if !finite(d) {
        return;
    }
    let mut seen = std::collections::BTreeSet::new();
    for i in idx {
        if seen.insert(*i)
            && let Some(v) = net.vertices.get_mut(*i)
        {
            v.x += d.x;
            v.y += d.y;
        }
    }
}

/// Map vertices `idx` through `m` (scale, rotate, flip…); the handles at them follow its linear
/// part. Nothing changes when any result wouldn't be finite.
pub fn transform_vertices(net: &mut VectorNetwork, idx: &[usize], m: Affine) {
    let set: std::collections::BTreeSet<usize> = idx.iter().copied().filter(|i| *i < net.vertices.len()).collect();
    let [a, b, c, d, _, _] = m.as_coeffs();
    let map_t = |t: (f64, f64)| (a * t.0 + c * t.1, b * t.0 + d * t.1);
    let points: Vec<(usize, Point)> = set.iter().filter_map(|i| Some((*i, m * net.vertices.get(*i)?.point()))).collect();
    // (segment, at its start, the mapped handle).
    let handles: Vec<(usize, bool, (f64, f64))> = net
        .segments
        .iter()
        .enumerate()
        .flat_map(|(si, s)| [(si, true, s.a, s.ta), (si, false, s.b, s.tb)])
        .filter(|(_, _, v, _)| set.contains(v))
        .map(|(si, start, _, t)| (si, start, map_t(t)))
        .collect();
    let ok = |x: f64, y: f64| x.is_finite() && y.is_finite();
    if !points.iter().all(|(_, p)| ok(p.x, p.y)) || !handles.iter().all(|(_, _, t)| ok(t.0, t.1)) {
        return;
    }
    for (i, p) in points {
        if let Some(v) = net.vertices.get_mut(i) {
            v.x = p.x;
            v.y = p.y;
        }
    }
    for (si, start, t) in handles {
        if let Some(s) = net.segments.get_mut(si) {
            if start {
                s.ta = t;
            } else {
                s.tb = t;
            }
        }
    }
}

/// Set a handle to `rel` (relative to its vertex). The vertex's other handle follows its mirroring
/// (none, angle, or angle and length); `independent` (Alt-drag) breaks the mirroring.
pub fn set_handle(net: &mut VectorNetwork, seg: usize, start: bool, rel: (f64, f64), independent: bool) {
    if !(rel.0.is_finite() && rel.1.is_finite()) {
        return;
    }
    let Some(s) = net.segments.get(seg).copied() else { return };
    let vi = if start { s.a } else { s.b };
    let mode = if independent { HandleMirroring::None } else { net.mirroring(vi) };
    net.set_handle(seg, start, rel);
    if independent {
        if let Some(v) = net.vertices.get_mut(vi) {
            v.mirror = Some(HandleMirroring::None);
        }
        return;
    }
    let others: Vec<(usize, bool)> = net.incident(vi).into_iter().filter(|(i, st)| !(*i == seg && *st == start)).collect();
    let [(oi, ostart)] = others.as_slice() else { return };
    let r = tvec(rel);
    let mirrored = match mode {
        HandleMirroring::None => return,
        HandleMirroring::AngleAndLength => -r,
        HandleMirroring::Angle => {
            let len = net.handle(*oi, *ostart).map(|h| tvec(h).hypot()).unwrap_or(0.0);
            if r.hypot() < 1e-9 {
                return;
            }
            -r.normalize() * if len > 1e-9 { len } else { r.hypot() }
        }
    };
    net.set_handle(*oi, *ostart, (mirrored.x, mirrored.y));
}

/// Move handles `hs` (segment, the handle at its `a` end) by `d` from where they are now, each as
/// [`set_handle`] would.
pub fn move_handles(net: &mut VectorNetwork, hs: &[(usize, bool)], d: Vec2, independent: bool) {
    let from: Vec<Option<(f64, f64)>> = hs.iter().map(|(s, st)| net.handle(*s, *st)).collect();
    for (&(seg, start), h) in hs.iter().zip(from) {
        if let Some(h) = h {
            set_handle(net, seg, start, (h.0 + d.x, h.1 + d.y), independent);
        }
    }
}

/// Set the mirroring of vertices and make their handles agree with it.
pub fn set_mirroring(net: &mut VectorNetwork, verts: &[usize], mode: HandleMirroring) {
    for &vi in verts {
        let inc = net.incident(vi);
        if let [(s1, a1), (s2, a2)] = inc.as_slice()
            && let (Some(h1), Some(h2)) = (net.handle(*s1, *a1), net.handle(*s2, *a2))
        {
            let (h1, h2) = (tvec(h1), tvec(h2));
            match mode {
                HandleMirroring::None => {}
                HandleMirroring::Angle | HandleMirroring::AngleAndLength => {
                    // Keep the longer handle's direction; the other turns opposite.
                    let (lead, lead_is_first) = if h1.hypot() >= h2.hypot() { (h1, true) } else { (h2, false) };
                    if lead.hypot() > 1e-9 {
                        let other_len = if mode == HandleMirroring::AngleAndLength {
                            (h1.hypot() + h2.hypot()) / 2.0
                        } else if lead_is_first {
                            h2.hypot()
                        } else {
                            h1.hypot()
                        };
                        let dir = lead.normalize();
                        let lead_len = if mode == HandleMirroring::AngleAndLength { other_len } else { lead.hypot() };
                        let (a, b) = (dir * lead_len, -dir * if other_len > 1e-9 { other_len } else { lead_len });
                        let (n1, n2) = if lead_is_first { (a, b) } else { (b, a) };
                        net.set_handle(*s1, *a1, (n1.x, n1.y));
                        net.set_handle(*s2, *a2, (n2.x, n2.y));
                    }
                }
            }
        }
        if let Some(v) = net.vertices.get_mut(vi) {
            v.mirror = Some(mode);
        }
    }
}

/// Direction from vertex `vi` along segment `seg` (towards its other end).
fn away(net: &VectorNetwork, seg: usize, at_start: bool) -> Option<Vec2> {
    let c = net.cubic(seg)?;
    let d = if at_start { c.p3 - c.p0 } else { c.p0 - c.p3 };
    (d.hypot() > 1e-9).then(|| d.normalize())
}

/// Pull symmetric handles out of a point (the Bend tool dragged on a point): `d` is the handle on
/// the side the drag points towards; the opposite side gets `-d`. Mirroring becomes "angle".
pub fn make_smooth(net: &mut VectorNetwork, vi: usize, d: Vec2) {
    if !finite(d) || d.hypot() < 1e-9 {
        return;
    }
    let inc = net.incident(vi);
    let scored: Vec<(usize, bool, f64)> = inc.iter().map(|(s, st)| (*s, *st, away(net, *s, *st).map(|a| a.dot(d)).unwrap_or(0.0))).collect();
    match scored.as_slice() {
        [] => return,
        [(s, st, dot)] => {
            let h = if *dot >= 0.0 { d } else { -d };
            net.set_handle(*s, *st, (h.x, h.y));
        }
        _ => {
            let fwd = scored.iter().copied().max_by(|a, b| a.2.total_cmp(&b.2));
            let back = scored.iter().copied().filter(|x| Some(*x) != fwd).min_by(|a, b| a.2.total_cmp(&b.2));
            if let (Some((s1, a1, _)), Some((s2, a2, _))) = (fwd, back) {
                net.set_handle(s1, a1, (d.x, d.y));
                net.set_handle(s2, a2, (-d.x, -d.y));
            }
        }
    }
    if let Some(v) = net.vertices.get_mut(vi) {
        v.mirror = Some(HandleMirroring::Angle);
    }
}

/// Smooth points automatically: handles along the line between the neighbours, a third of the way
/// to each (endpoints stay as they are).
pub fn auto_smooth(net: &mut VectorNetwork, vi: usize) {
    let inc = net.incident(vi);
    let [(s1, a1), (s2, a2)] = inc.as_slice() else { return };
    let other = |s: usize, st: bool| net.segments.get(s).and_then(|x| net.vertices.get(if st { x.b } else { x.a })).map(|v| v.point());
    let (Some(v), Some(p1), Some(p2)) = (net.vertices.get(vi).map(|v| v.point()), other(*s1, *a1), other(*s2, *a2)) else { return };
    let dir = p1 - p2;
    if dir.hypot() < 1e-9 {
        return;
    }
    let dir = dir.normalize();
    let (l1, l2) = ((p1 - v).hypot() / 3.0, (p2 - v).hypot() / 3.0);
    let (h1, h2) = (dir * l1, -dir * l2);
    net.set_handle(*s1, *a1, (h1.x, h1.y));
    net.set_handle(*s2, *a2, (h2.x, h2.y));
    if let Some(x) = net.vertices.get_mut(vi) {
        x.mirror = Some(HandleMirroring::Angle);
    }
}

/// Remove a point's handles (a sharp corner).
pub fn make_sharp(net: &mut VectorNetwork, vi: usize) {
    for (s, st) in net.incident(vi) {
        net.set_handle(s, st, (0.0, 0.0));
    }
    if let Some(v) = net.vertices.get_mut(vi) {
        v.mirror = None;
    }
}

/// Bend a segment so its curve passes through `p` at parameter `t` (the Bend tool).
pub fn bend(net: &mut VectorNetwork, seg: usize, t: f64, p: Point) {
    let Some(s) = net.segments.get(seg).copied() else { return };
    let (Some(a), Some(b)) = (net.vertices.get(s.a).copied(), net.vertices.get(s.b).copied()) else { return };
    if !(t.is_finite() && p.x.is_finite() && p.y.is_finite()) {
        return;
    }
    let t = t.clamp(0.05, 0.95);
    // Controls at the chord's thirds plus a shared offset: B(t) = L(t) + 3t(1-t)·off.
    let (a, b) = (a.point(), b.point());
    let k = 3.0 * t * (1.0 - t);
    let off = (p - a.lerp(b, t)) / k;
    let c1 = a + (b - a) / 3.0 + off;
    let c2 = b + (a - b) / 3.0 + off;
    net.set_handle(seg, true, (c1.x - a.x, c1.y - a.y));
    net.set_handle(seg, false, (c2.x - b.x, c2.y - b.y));
}

/// Split segment `seg` at `t`, inserting a vertex; returns its index.
pub fn split_segment(net: &mut VectorNetwork, seg: usize, t: f64) -> Option<usize> {
    let s = net.segments.get(seg).copied()?;
    let c: CubicBez = net.cubic(seg)?;
    // Which way each region loop walks `seg`, read before the split changes its endpoints.
    let reversed: Vec<Vec<bool>> = net
        .regions
        .iter()
        .map(|reg| reg.loops.iter().map(|lp| net.loop_walk(lp).iter().find(|(i, _)| *i == seg).is_some_and(|(_, rev)| *rev)).collect())
        .collect();
    let t = if t.is_finite() { t.clamp(0.01, 0.99) } else { 0.5 };
    let l = c.subsegment(0.0..t);
    let r = c.subsegment(t..1.0);
    net.vertices.push(Vertex::new(l.p3.x, l.p3.y));
    let mid = net.vertices.len() - 1;
    let straight = s.is_straight();
    if let Some(sm) = net.segments.get_mut(seg) {
        sm.b = mid;
        if straight {
            sm.ta = (0.0, 0.0);
            sm.tb = (0.0, 0.0);
        } else {
            sm.ta = (l.p1.x - l.p0.x, l.p1.y - l.p0.y);
            sm.tb = (l.p2.x - l.p3.x, l.p2.y - l.p3.y);
        }
    }
    let new_seg = if straight {
        Segment::line(mid, s.b)
    } else {
        // A point added on a curve keeps it smooth.
        if let Some(v) = net.vertices.get_mut(mid) {
            v.mirror = Some(HandleMirroring::Angle);
        }
        Segment { a: mid, b: s.b, ta: (r.p1.x - r.p0.x, r.p1.y - r.p0.y), tb: (r.p2.x - r.p3.x, r.p2.y - r.p3.y) }
    };
    net.segments.push(new_seg);
    let ni = net.segments.len() - 1;
    for (r, reg) in net.regions.iter_mut().enumerate() {
        for (l, lp) in reg.loops.iter_mut().enumerate() {
            if let Some(pos) = lp.iter().position(|x| *x == seg) {
                // Walked backwards, the new half (mid→b) comes before `seg` (a→mid).
                let rev = reversed.get(r).and_then(|v| v.get(l)).copied().unwrap_or(false);
                lp.insert(if rev { pos } else { pos + 1 }, ni);
            }
        }
    }
    Some(mid)
}

/// Remove vertex `vi` (no segment may use it) and shift higher indices down.
fn remove_vertex_index(net: &mut VectorNetwork, vi: usize) {
    if vi >= net.vertices.len() {
        return;
    }
    net.vertices.remove(vi);
    for s in &mut net.segments {
        if s.a > vi {
            s.a -= 1;
        }
        if s.b > vi {
            s.b -= 1;
        }
    }
}

/// Remove segments (fixing region loops); points left without any segment go too.
pub fn remove_segments(net: &mut VectorNetwork, segs: &[usize]) {
    let mut touched: Vec<usize> = segs.iter().filter_map(|s| net.segments.get(*s)).flat_map(|s| [s.a, s.b]).collect();
    remove_segments_only(net, segs);
    touched.sort_unstable();
    touched.dedup();
    for &vi in touched.iter().rev() {
        if net.incident(vi).is_empty() {
            remove_vertex_index(net, vi);
        }
    }
    reopen_loops(net);
}

fn remove_segments_only(net: &mut VectorNetwork, segs: &[usize]) {
    let mut segs: Vec<usize> = segs.iter().copied().filter(|s| *s < net.segments.len()).collect();
    segs.sort_unstable();
    segs.dedup();
    let mut map: Vec<Option<usize>> = (0..net.segments.len()).map(Some).collect();
    for &si in segs.iter().rev() {
        net.segments.remove(si);
        if let Some(m) = map.get_mut(si) {
            *m = None;
        }
    }
    for (k, m) in map.iter_mut().flatten().enumerate() {
        *m = k;
    }
    for r in &mut net.regions {
        for lp in &mut r.loops {
            *lp = lp.iter().filter_map(|i| map.get(*i).copied().flatten()).collect();
        }
        r.loops.retain(|l| l.len() >= 2);
    }
    net.regions.retain(|r| !r.loops.is_empty());
}

/// Delete vertices, joining their neighbours where a vertex had exactly two segments (`heal`).
pub fn delete_vertices(net: &mut VectorNetwork, idx: &[usize], heal: bool) {
    let mut idx: Vec<usize> = idx.to_vec();
    idx.sort_unstable();
    idx.dedup();
    for &vi in idx.iter().rev() {
        if vi >= net.vertices.len() {
            continue;
        }
        let touching: Vec<usize> = net.incident(vi).into_iter().map(|(i, _)| i).collect();
        let mut healed = None;
        if heal && let [s1, s2] = touching.as_slice() {
            let (Some(x), Some(y)) = (net.segments.get(*s1).copied(), net.segments.get(*s2).copied()) else { continue };
            let o1 = if x.a == vi { (x.b, x.tb) } else { (x.a, x.ta) };
            let o2 = if y.a == vi { (y.b, y.tb) } else { (y.a, y.ta) };
            if o1.0 != o2.0 {
                let curved = !(x.is_straight() && y.is_straight());
                let (ta, tb) = if curved { heal_handles(net, vi, o1, o2).unwrap_or((o1.1, o2.1)) } else { (o1.1, o2.1) };
                healed = Some(Segment { a: o1.0, b: o2.0, ta, tb });
            }
        }
        // Remove touching segments (descending) and fix region loops (the healed segment takes
        // the removed ones' place).
        let mut map: Vec<Option<usize>> = (0..net.segments.len()).map(Some).collect();
        for &si in touching.iter().rev() {
            net.segments.remove(si);
            if let Some(m) = map.get_mut(si) {
                *m = None;
            }
        }
        for (k, m) in map.iter_mut().flatten().enumerate() {
            *m = k;
        }
        let healed_index = healed.map(|s| {
            net.segments.push(s);
            net.segments.len() - 1
        });
        for r in &mut net.regions {
            for lp in &mut r.loops {
                let mut out = Vec::new();
                let mut inserted = false;
                for si in lp.iter() {
                    match map.get(*si).copied().flatten() {
                        Some(n) => out.push(n),
                        None => {
                            if let (Some(h), false) = (healed_index, inserted) {
                                out.push(h);
                                inserted = true;
                            }
                        }
                    }
                }
                *lp = out;
            }
            r.loops.retain(|l| l.len() >= 2);
        }
        net.regions.retain(|r| !r.loops.is_empty());
        remove_vertex_index(net, vi);
    }
    net.sanitize();
    reopen_loops(net);
}

/// Handles for the curve that heals over deleted point `vi` between `(vertex, handle)` ends `o1`
/// and `o2`: each end keeps its handle's direction (towards the deleted point when it had none),
/// and the lengths are chosen so the curve still passes through the deleted point.
fn heal_handles(net: &VectorNetwork, vi: usize, o1: (usize, (f64, f64)), o2: (usize, (f64, f64))) -> Option<((f64, f64), (f64, f64))> {
    let p = net.vertices.get(vi)?.point();
    let (a, b) = (net.vertices.get(o1.0)?.point(), net.vertices.get(o2.0)?.point());
    let dir = |h: (f64, f64), from: Point| {
        let h = tvec(h);
        let d = if h.hypot() > 1e-9 { h } else { p - from };
        (d.hypot() > 1e-9).then(|| d.normalize())
    };
    let (d1, d2) = (dir(o1.1, a)?, dir(o2.1, b)?);
    let (la, lb) = ((p - a).hypot(), (p - b).hypot());
    let t = la / (la + lb).max(1e-12);
    // B(t) with both handles zero, and how much each handle length moves it.
    let (u, k1, k2) = (1.0 - t, 3.0 * (1.0 - t).powi(2) * t, 3.0 * (1.0 - t) * t * t);
    let base = (a.to_vec2() * (u.powi(3) + 3.0 * u * u * t) + b.to_vec2() * (3.0 * u * t * t + t.powi(3))).to_point();
    let r = p - base;
    let (c1, c2) = (d1 * k1, d2 * k2);
    let det = c1.cross(c2);
    let (l1, l2) = if det.abs() > 1e-9 * c1.hypot() * c2.hypot() {
        (r.cross(c2) / det, c1.cross(r) / det)
    } else {
        // Parallel handles: one shared length, the best fit.
        let c = c1 + c2;
        let l = r.dot(c) / c.hypot2().max(1e-12);
        (l, l)
    };
    let ok = |l: f64| l.is_finite() && l > 0.0;
    (ok(l1) && ok(l2)).then_some(((d1.x * l1, d1.y * l1), (d2.x * l2, d2.y * l2)))
}

/// After a loop was broken open, start it where the break is so it's one continuous chain (the fill
/// then closes it with a straight line, like an open path's fill).
fn reopen_loops(net: &mut VectorNetwork) {
    let segs = net.segments.clone();
    let shares = |x: usize, y: usize| -> bool {
        match (segs.get(x), segs.get(y)) {
            (Some(a), Some(b)) => a.a == b.a || a.a == b.b || a.b == b.a || a.b == b.b,
            _ => false,
        }
    };
    for r in &mut net.regions {
        for lp in &mut r.loops {
            let n = lp.len();
            if n < 2 {
                continue;
            }
            let brk = (0..n).find(|k| {
                let prev = lp.get((k + n - 1) % n).copied().unwrap_or(0);
                let cur = lp.get(*k).copied().unwrap_or(0);
                !shares(prev, cur)
            });
            if let Some(k) = brk {
                lp.rotate_left(k);
            }
        }
    }
}

/// Cut a segment at `t` into two open ends at the same spot. Returns the two end vertices.
pub fn cut_segment(net: &mut VectorNetwork, seg: usize, t: f64) -> Option<(usize, usize)> {
    let mid = split_segment(net, seg, t)?;
    let ni = net.segments.len().checked_sub(1)?;
    let v = *net.vertices.get(mid)?;
    net.vertices.push(Vertex { mirror: None, ..v });
    let mid2 = net.vertices.len() - 1;
    if let Some(s) = net.segments.get_mut(ni) {
        s.a = mid2;
    }
    if let Some(x) = net.vertices.get_mut(mid) {
        x.mirror = None;
    }
    reopen_loops(net);
    Some((mid, mid2))
}

/// Cut the path at a point: every segment after the first gets its own copy of the point.
pub fn cut_vertex(net: &mut VectorNetwork, vi: usize) -> Vec<usize> {
    let inc = net.incident(vi);
    let Some(v) = net.vertices.get(vi).copied() else { return vec![] };
    let mut out = vec![vi];
    for (s, st) in inc.into_iter().skip(1) {
        net.vertices.push(Vertex { mirror: None, ..v });
        let nv = net.vertices.len() - 1;
        if let Some(seg) = net.segments.get_mut(s) {
            if st {
                seg.a = nv;
            } else {
                seg.b = nv;
            }
        }
        out.push(nv);
    }
    if let Some(x) = net.vertices.get_mut(vi) {
        x.mirror = None;
    }
    reopen_loops(net);
    out
}

/// The Cut tool dragged from `a` to `b`: every segment the line crosses is cut where it crosses.
/// Returns the new ends.
/// shortcut: crossings within 1% of a segment's ends are skipped; cut those at the point instead.
pub fn cut_across(net: &mut VectorNetwork, a: Point, b: Point) -> Vec<usize> {
    use figcraft_geom::kurbo::{Line, PathSeg};
    if !(a.x.is_finite() && a.y.is_finite() && b.x.is_finite() && b.y.is_finite()) || (b - a).hypot() < 1e-9 {
        return vec![];
    }
    let line = Line::new(a, b);
    let crossings: Vec<(usize, Vec<f64>)> = (0..net.segments.len())
        .filter_map(|si| {
            let mut ts: Vec<f64> = PathSeg::Cubic(net.cubic(si)?)
                .intersect_line(line)
                .iter()
                .filter(|h| (0.0..=1.0).contains(&h.line_t) && (0.01..=0.99).contains(&h.segment_t))
                .map(|h| h.segment_t)
                .collect();
            ts.sort_by(f64::total_cmp);
            ts.dedup_by(|x, y| (*x - *y).abs() < 1e-6);
            (!ts.is_empty()).then_some((si, ts))
        })
        .collect();
    let mut ends = Vec::new();
    for (si, ts) in crossings {
        // From the last crossing back: the segment keeps the part before each cut.
        let mut end = 1.0;
        for t in ts.into_iter().rev() {
            if let Some((x, y)) = cut_segment(net, si, t / end) {
                ends.extend([x, y]);
            }
            end = t;
        }
    }
    ends
}

/// The connected parts of the network: each part's points, the part holding point 0 first.
pub fn parts(net: &VectorNetwork) -> Vec<Vec<usize>> {
    let n = net.vertices.len();
    let mut root: Vec<usize> = (0..n).collect();
    fn find(root: &mut [usize], mut v: usize) -> usize {
        while let Some(&r) = root.get(v) {
            if r == v {
                break;
            }
            v = r;
        }
        v
    }
    for s in &net.segments {
        let (x, y) = (find(&mut root, s.a), find(&mut root, s.b));
        if let Some(r) = root.get_mut(x) {
            *r = y;
        }
    }
    let mut out: Vec<(usize, Vec<usize>)> = Vec::new();
    for v in 0..n {
        let r = find(&mut root, v);
        match out.iter_mut().find(|(k, _)| *k == r) {
            Some((_, part)) => part.push(v),
            None => out.push((r, vec![v])),
        }
    }
    out.into_iter().map(|(_, p)| p).collect()
}

/// The part of the network made of points `verts`: their segments, and the pieces of fill loops
/// running through them (a loop that was cut through stays filled on each side).
pub fn part(net: &VectorNetwork, verts: &[usize]) -> VectorNetwork {
    let mut vmap = vec![None; net.vertices.len()];
    let mut out = VectorNetwork::default();
    for &v in verts {
        if let (Some(x), Some(slot)) = (net.vertices.get(v), vmap.get_mut(v)) {
            *slot = Some(out.vertices.len());
            out.vertices.push(*x);
        }
    }
    let mut smap = vec![None; net.segments.len()];
    for (i, s) in net.segments.iter().enumerate() {
        if let (Some(Some(a)), Some(Some(b)), Some(slot)) = (vmap.get(s.a), vmap.get(s.b), smap.get_mut(i)) {
            *slot = Some(out.segments.len());
            out.segments.push(Segment { a: *a, b: *b, ..*s });
        }
    }
    for r in &net.regions {
        let loops: Vec<Vec<usize>> =
            r.loops.iter().map(|l| l.iter().filter_map(|i| smap.get(*i).copied().flatten()).collect::<Vec<_>>()).filter(|l| !l.is_empty()).collect();
        if !loops.is_empty() {
            out.regions.push(Region { loops, winding: r.winding });
        }
    }
    reopen_loops(&mut out);
    out
}

/// Shortest chain of segments from `from` to `to` (breadth first), skipping segment `skip`.
fn path_between(net: &VectorNetwork, from: usize, to: usize, skip: usize) -> Option<Vec<usize>> {
    let n = net.vertices.len();
    let mut prev: Vec<Option<(usize, usize)>> = vec![None; n];
    let mut seen = vec![false; n];
    let mut queue = std::collections::VecDeque::from([from]);
    if let Some(s) = seen.get_mut(from) {
        *s = true;
    }
    while let Some(v) = queue.pop_front() {
        if v == to {
            let mut segs = Vec::new();
            let mut cur = to;
            while cur != from {
                let (pv, ps) = prev.get(cur).copied().flatten()?;
                segs.push(ps);
                cur = pv;
            }
            segs.reverse();
            return Some(segs);
        }
        for (s, st) in net.incident(v) {
            if s == skip {
                continue;
            }
            let Some(seg) = net.segments.get(s) else { continue };
            let w = if st { seg.b } else { seg.a };
            if !seen.get(w).copied().unwrap_or(true) {
                if let Some(x) = seen.get_mut(w) {
                    *x = true;
                }
                if let Some(x) = prev.get_mut(w) {
                    *x = Some((v, s));
                }
                queue.push_back(w);
            }
        }
    }
    None
}

/// Connect two points with a new segment (handles relative to each end). Closing an open path
/// (both points were its ends) makes the loop a fill region. Returns the new segment.
pub fn connect(net: &mut VectorNetwork, a: usize, b: usize, ta: (f64, f64), tb: (f64, f64)) -> Option<usize> {
    if a == b || a >= net.vertices.len() || b >= net.vertices.len() {
        return None;
    }
    let ends = net.incident(a).len() == 1 && net.incident(b).len() == 1;
    let finite_t = |t: (f64, f64)| if t.0.is_finite() && t.1.is_finite() { t } else { (0.0, 0.0) };
    net.segments.push(Segment { a, b, ta: finite_t(ta), tb: finite_t(tb) });
    let si = net.segments.len() - 1;
    if ends && let Some(mut lp) = path_between(net, b, a, si) {
        let unfilled = !net.regions.iter().any(|r| r.loops.iter().flatten().any(|s| lp.contains(s)));
        if unfilled {
            lp.insert(0, si);
            net.regions.push(Region { loops: vec![lp], winding: WindingRule::NonZero });
        }
    }
    Some(si)
}

/// Merge vertex `drop` into `keep` (segments re-attach; ones that collapse go).
pub fn merge_vertices(net: &mut VectorNetwork, keep: usize, drop: usize) {
    if keep == drop || keep >= net.vertices.len() || drop >= net.vertices.len() {
        return;
    }
    for s in &mut net.segments {
        if s.a == drop {
            s.a = keep;
        }
        if s.b == drop {
            s.b = keep;
        }
    }
    let collapsed: Vec<usize> = net.segments.iter().enumerate().filter(|(_, s)| s.a == s.b && s.is_straight()).map(|(i, _)| i).collect();
    remove_segments_only(net, &collapsed);
    remove_vertex_index(net, drop);
    reopen_loops(net);
}

/// Join points (⌘J): two points that overlap merge; otherwise open ends are connected in pairs
/// (nearest first). `smooth` continues each end's curve. Returns the number of joins.
pub fn join(net: &mut VectorNetwork, verts: &[usize], smooth: bool) -> usize {
    let mut verts: Vec<usize> = verts.iter().copied().filter(|v| *v < net.vertices.len()).collect();
    verts.sort_unstable();
    verts.dedup();
    if let [a, b] = verts.as_slice() {
        let (pa, pb) = (net.vertices.get(*a).map(|v| v.point()), net.vertices.get(*b).map(|v| v.point()));
        if let (Some(pa), Some(pb)) = (pa, pb)
            && (pa - pb).hypot() < 0.5
        {
            merge_vertices(net, *a, *b);
            return 1;
        }
    }
    let mut ends: Vec<usize> = if verts.len() == 2 { verts.clone() } else { verts.iter().copied().filter(|v| net.incident(*v).len() <= 1).collect() };
    let mut joins = 0;
    while ends.len() >= 2 {
        let a = ends.remove(0);
        let Some(pa) = net.vertices.get(a).map(|v| v.point()) else { continue };
        let Some((k, _)) = ends
            .iter()
            .enumerate()
            .filter_map(|(k, b)| net.vertices.get(*b).map(|v| (k, (v.point() - pa).hypot())))
            .min_by(|x, y| x.1.total_cmp(&y.1))
        else {
            break;
        };
        let b = ends.remove(k);
        let Some(pb) = net.vertices.get(b).map(|v| v.point()) else { continue };
        let (ta, tb) = if smooth {
            let len = (pb - pa).hypot() / 3.0;
            let out = |v: usize, toward: Vec2| -> Vec2 {
                // Continue the existing curve out of the end; fall back to the chord.
                let inc = net.incident(v);
                let dir = match inc.as_slice() {
                    [(s, st)] => {
                        net.handle(*s, *st).map(tvec).filter(|h| h.hypot() > 1e-9).map(|h| -h.normalize()).or_else(|| away(net, *s, *st).map(|a| -a))
                    }
                    _ => None,
                };
                dir.unwrap_or(toward) * len
            };
            let chord = if (pb - pa).hypot() > 1e-9 { (pb - pa).normalize() } else { Vec2::ZERO };
            let (oa, ob) = (out(a, chord), out(b, -chord));
            ((oa.x, oa.y), (ob.x, ob.y))
        } else {
            ((0.0, 0.0), (0.0, 0.0))
        };
        if connect(net, a, b, ta, tb).is_some() {
            joins += 1;
        }
    }
    joins
}

/// Vertices inside a closed polygon (even-odd), all in the same coordinates.
pub fn in_polygon(points: &[Point], poly: &[Point]) -> Vec<usize> {
    if poly.len() < 3 {
        return vec![];
    }
    points
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            let mut inside = false;
            let n = poly.len();
            for k in 0..n {
                let (Some(a), Some(b)) = (poly.get(k), poly.get((k + 1) % n)) else { continue };
                if (a.y > p.y) != (b.y > p.y) {
                    let x = a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y);
                    if p.x < x {
                        inside = !inside;
                    }
                }
            }
            inside
        })
        .map(|(i, _)| i)
        .collect()
}

/// The parameter ranges of segment `seg` inside a circle (sampled, edges refined).
fn inside_ranges(net: &VectorNetwork, seg: usize, c: Point, r: f64) -> Vec<(f64, f64)> {
    let Some(cb) = net.cubic(seg) else { return vec![] };
    // Strictly inside, so points exactly on the circle (earlier cuts) count as outside.
    let inside = |t: f64| (cb.eval(t) - c).hypot() < r * (1.0 - 1e-9) - 1e-9;
    // Bisect the boundary between an inside and an outside parameter; return the outside end.
    let edge = |mut a: f64, mut b: f64| {
        let ia = inside(a);
        for _ in 0..30 {
            let m = (a + b) / 2.0;
            if inside(m) == ia {
                a = m;
            } else {
                b = m;
            }
        }
        if ia { b } else { a }
    };
    let n = 64;
    // Upper bound on the curve's speed: no point of a sample interval is farther than the whole
    // interval length from either end, so only intervals that come that close need a closer look.
    let speed = 3.0 * [(cb.p1 - cb.p0).hypot(), (cb.p2 - cb.p1).hypot(), (cb.p3 - cb.p2).hypot()].into_iter().fold(0.0, f64::max);
    let reach = speed / f64::from(n);
    // A crossing narrower than an interval whose ends are both outside: the closest point.
    let dip = |a: f64, b: f64| -> Option<f64> {
        if (cb.eval(a) - c).hypot().min((cb.eval(b) - c).hypot()) > r + reach {
            return None;
        }
        let near = cb.subsegment(a..b).nearest(c, 1e-12);
        let m = a + (b - a) * near.t;
        (m > a && m < b && inside(m)).then_some(m)
    };
    let mut out = Vec::new();
    let mut start: Option<f64> = if inside(0.0) { Some(0.0) } else { None };
    let mut prev = 0.0;
    for i in 1..=n {
        let t = f64::from(i) / f64::from(n);
        match (start, inside(t)) {
            (None, true) => start = Some(edge(prev, t)),
            (None, false) => {
                if let Some(m) = dip(prev, t) {
                    out.push((edge(prev, m), edge(m, t)));
                }
            }
            (Some(s0), false) => {
                out.push((s0, edge(prev, t)));
                start = None;
            }
            _ => {}
        }
        prev = t;
    }
    if let Some(s0) = start {
        out.push((s0, 1.0));
    }
    out
}

/// The Eraser: remove every part of the network inside the circle at `c` with radius `r`,
/// splitting segments where the circle crosses them. Returns whether anything changed.
pub fn erase(net: &mut VectorNetwork, c: Point, r: f64) -> bool {
    if !(c.x.is_finite() && c.y.is_finite() && r.is_finite() && r > 0.0) {
        return false;
    }
    let mut changed = false;
    // One cut at a time: indices shift as pieces are added and removed, so re-scan after each.
    // (`split_segment` clamps to 1%..99%, so ranges within 1% of an end count as reaching it.)
    const EPS: f64 = 0.01;
    for _ in 0..10_000 {
        let hit = (0..net.segments.len()).find_map(|si| inside_ranges(net, si, c, r).first().copied().map(|rg| (si, rg)));
        let Some((si, (t0, t1))) = hit else { break };
        changed = true;
        if t0 <= EPS && t1 >= 1.0 - EPS {
            remove_segments(net, &[si]);
        } else if t0 <= EPS {
            // The start is inside: cut where the circle ends and drop the first piece.
            if split_segment(net, si, t1).is_none() {
                break;
            }
            remove_segments(net, &[si]);
        } else if t1 >= 1.0 - EPS {
            // The end is inside: cut where the circle starts and drop the second piece.
            if split_segment(net, si, t0).is_none() {
                break;
            }
            let last = net.segments.len() - 1;
            remove_segments(net, &[last]);
        } else if split_segment(net, si, t0).is_none() {
            // The middle is inside: cut where it starts; the second piece then starts inside.
            break;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> VectorNetwork {
        VectorNetwork::from_points(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 100.0), Point::new(0.0, 100.0)], true)
    }

    fn open_line() -> VectorNetwork {
        VectorNetwork::from_points(&[Point::new(0.0, 0.0), Point::new(50.0, 0.0), Point::new(100.0, 0.0)], false)
    }

    #[test]
    fn handles_follow_mirroring() {
        let mut n = open_line();
        set_mirroring(&mut n, &[1], HandleMirroring::AngleAndLength);
        // Seg 0 ends at v1 (tb), seg 1 starts there (ta).
        set_handle(&mut n, 1, true, (20.0, 10.0), false);
        assert_eq!(n.segments[0].tb, (-20.0, -10.0));
        set_mirroring(&mut n, &[1], HandleMirroring::Angle);
        set_handle(&mut n, 1, true, (0.0, 30.0), false);
        let other = n.segments[0].tb;
        assert!(other.0.abs() < 1e-9 && (other.1 + 22.360679).abs() < 1e-4, "angle mirrors, length kept: {other:?}");
        assert_eq!(n.mirroring(1), HandleMirroring::Angle);
        set_handle(&mut n, 1, true, (10.0, 0.0), true);
        assert_eq!(n.mirroring(1), HandleMirroring::None);
        assert!(n.segments[0].tb.1 < -20.0, "independent drag leaves the other handle");
        set_mirroring(&mut n, &[1], HandleMirroring::AngleAndLength);
        let (a, b) = (tvec(n.segments[1].ta), tvec(n.segments[0].tb));
        assert!((a + b).hypot() < 1e-9, "made symmetric: {a:?} {b:?}");
    }

    fn loop_is_continuous(n: &VectorNetwork, lp: &[usize]) -> bool {
        let ends: Vec<(usize, usize)> = n
            .loop_walk(lp)
            .iter()
            .map(|&(i, rev)| {
                let s = n.segments[i];
                if rev { (s.b, s.a) } else { (s.a, s.b) }
            })
            .collect();
        (0..ends.len()).all(|k| ends[k].1 == ends[(k + 1) % ends.len()].0)
    }

    #[test]
    fn split_keeps_region_loop_in_ring_order() {
        // Triangle closed 0→2: its loop [2, 1, 0] walks segment 1 backwards.
        let mut n = VectorNetwork::from_points(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 100.0)], false);
        n.segments.push(Segment::line(0, 2));
        n.regions.push(Region { loops: vec![vec![2, 1, 0]], winding: WindingRule::NonZero });
        let p = Point::new(25.0, 10.0);
        assert!(n.fill_contains(p));
        for (seg, lp) in [(1, vec![2, 1, 0]), (2, vec![2, 1, 0]), (1, vec![0, 1, 2])] {
            let mut m = n.clone();
            m.regions[0].loops = vec![lp];
            let mid = split_segment(&mut m, seg, 0.5).unwrap();
            assert!(loop_is_continuous(&m, &m.regions[0].loops[0]), "loop {:?}", m.regions[0].loops);
            assert!(m.fill_contains(p), "split of {seg} removed {p:?} from the fill");
            if seg == 1 {
                assert_eq!(m.vertices[mid].point(), Point::new(100.0, 50.0));
            }
        }
    }

    #[test]
    fn inferred_mirroring() {
        let mut n = open_line();
        assert_eq!(n.mirroring(1), HandleMirroring::None);
        n.segments[0].tb = (-10.0, 0.0);
        n.segments[1].ta = (10.0, 0.0);
        assert_eq!(n.mirroring(1), HandleMirroring::AngleAndLength);
        n.segments[1].ta = (20.0, 0.0);
        assert_eq!(n.mirroring(1), HandleMirroring::Angle);
        n.segments[1].ta = (20.0, 5.0);
        assert_eq!(n.mirroring(1), HandleMirroring::None);
    }

    #[test]
    fn smooth_and_sharp() {
        let mut n = square();
        make_smooth(&mut n, 0, Vec2::new(30.0, 0.0));
        // Seg 0 goes 0→1 (towards +x): gets +d; seg 3 (3→0) gets -d at its b end.
        assert_eq!(n.segments[0].ta, (30.0, 0.0));
        assert_eq!(n.segments[3].tb, (-30.0, 0.0));
        assert_eq!(n.mirroring(0), HandleMirroring::Angle);
        make_sharp(&mut n, 0);
        assert!(n.segments[0].is_straight() && n.segments[3].is_straight());
        auto_smooth(&mut n, 1);
        assert_eq!(n.mirroring(1), HandleMirroring::Angle);
        assert!(!n.segments[0].is_straight());
    }

    #[test]
    fn cut_opens_a_closed_shape_but_keeps_its_fill() {
        let mut n = square();
        let (a, b) = cut_segment(&mut n, 0, 0.5).unwrap();
        assert_ne!(a, b);
        assert_eq!(n.vertices[a].point(), n.vertices[b].point());
        assert_eq!(n.incident(a).len(), 1);
        assert_eq!(n.incident(b).len(), 1);
        assert!(n.fill_contains(Point::new(50.0, 50.0)), "fill kept");
        // Cutting at a corner detaches it.
        let mut m = square();
        let vs = cut_vertex(&mut m, 2);
        assert_eq!(vs.len(), 2);
        assert!(vs.iter().all(|v| m.incident(*v).len() == 1));
        assert!(m.fill_contains(Point::new(50.0, 50.0)));
    }

    #[test]
    fn connecting_ends_closes_and_fills() {
        let mut n = open_line();
        n.vertices[2].y = 100.0;
        assert!(n.regions.is_empty());
        let s = connect(&mut n, 2, 0, (0.0, 0.0), (0.0, 0.0)).unwrap();
        assert_eq!(n.regions.len(), 1);
        assert!(n.regions[0].loops[0].contains(&s));
        assert!(n.fill_contains(Point::new(50.0, 20.0)));
        // Branching from the middle point doesn't make a region.
        n.vertices.push(Vertex::new(50.0, -50.0));
        connect(&mut n, 1, 3, (0.0, 0.0), (0.0, 0.0)).unwrap();
        assert_eq!(n.regions.len(), 1);
        assert!(connect(&mut n, 1, 1, (0.0, 0.0), (0.0, 0.0)).is_none());
        assert!(connect(&mut n, 1, 99, (0.0, 0.0), (0.0, 0.0)).is_none());
    }

    #[test]
    fn join_merges_overlapping_points_and_connects_ends() {
        let mut n = open_line();
        n.vertices.push(Vertex::new(100.2, 0.0));
        n.vertices.push(Vertex::new(150.0, 0.0));
        n.segments.push(Segment::line(3, 4));
        assert_eq!(join(&mut n, &[2, 3], false), 1);
        assert_eq!(n.vertices.len(), 4, "merged");
        assert_eq!(n.incident(2).len(), 2);
        // Two separate open ends far apart: connected (smoothly).
        let mut m = open_line();
        m.vertices.push(Vertex::new(200.0, 50.0));
        m.vertices.push(Vertex::new(300.0, 50.0));
        m.segments.push(Segment::line(3, 4));
        assert_eq!(join(&mut m, &[0, 2, 3, 4], true), 2);
        assert!(m.segments.iter().skip(3).any(|s| !s.is_straight()));
    }

    #[test]
    fn deleting_segments_and_points() {
        let mut n = square();
        remove_segments(&mut n, &[0]);
        assert_eq!(n.segments.len(), 3);
        assert_eq!(n.vertices.len(), 4, "points with other segments stay");
        let mut m = open_line();
        remove_segments(&mut m, &[1]);
        assert_eq!(m.vertices.len(), 2, "the orphaned end goes");
        let mut h = square();
        delete_vertices(&mut h, &[1], true);
        assert_eq!((h.vertices.len(), h.segments.len(), h.regions.len()), (3, 3, 1));
        let mut d = square();
        delete_vertices(&mut d, &[1], false);
        assert_eq!((d.vertices.len(), d.segments.len()), (3, 2));
        // Junk indices.
        remove_segments(&mut d, &[99]);
        delete_vertices(&mut d, &[99], true);
        assert!(split_segment(&mut d, 99, 0.5).is_none());
        assert!(cut_segment(&mut d, 99, f64::NAN).is_none());
        assert!(cut_vertex(&mut d, 99).is_empty());
        set_handle(&mut d, 99, true, (1.0, 1.0), false);
        set_handle(&mut d, 0, true, (f64::NAN, 1.0), false);
        make_smooth(&mut d, 99, Vec2::new(1.0, 1.0));
        bend(&mut d, 0, f64::NAN, Point::new(1.0, 1.0));
        move_vertices(&mut d, &[0], Vec2::new(f64::INFINITY, 0.0));
        assert!(d.vertices.iter().all(|v| v.x.is_finite() && v.y.is_finite()));
    }

    #[test]
    fn lasso_polygon() {
        let pts = [Point::new(5.0, 5.0), Point::new(50.0, 50.0), Point::new(15.0, 5.0)];
        let poly = [Point::new(0.0, 0.0), Point::new(20.0, 0.0), Point::new(20.0, 20.0), Point::new(0.0, 20.0)];
        assert_eq!(in_polygon(&pts, &poly), vec![0, 2]);
        assert!(in_polygon(&pts, &poly[..2]).is_empty());
    }

    #[test]
    fn eraser_cuts_through_and_removes() {
        // A straight line from 0 to 100: erasing around 50 leaves two pieces with a gap.
        let mut n = VectorNetwork::from_points(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0)], false);
        assert!(erase(&mut n, Point::new(50.0, 0.0), 10.0));
        assert_eq!(n.segments.len(), 2);
        let ends: Vec<f64> = n.vertices.iter().map(|v| v.x).collect();
        assert!(ends.iter().any(|x| (x - 40.0).abs() < 0.01) && ends.iter().any(|x| (x - 60.0).abs() < 0.01), "{ends:?}");
        // Erasing an end shortens it; erasing everything removes it all.
        assert!(erase(&mut n, Point::new(0.0, 0.0), 5.0));
        assert!(n.vertices.iter().all(|v| v.x >= 4.99), "start shortened");
        assert!(erase(&mut n, Point::new(50.0, 0.0), 200.0));
        assert!(n.segments.is_empty() && n.vertices.is_empty());
        // Missing everything changes nothing.
        let mut m = square();
        let before = m.clone();
        assert!(!erase(&mut m, Point::new(500.0, 500.0), 5.0));
        assert_eq!(m, before);
        // Through a closed square's corner: the loop opens, the fill stays implied.
        assert!(erase(&mut m, Point::new(100.0, 0.0), 10.0));
        assert!(m.segments.len() >= 4);
        assert!(m.fill_contains(Point::new(50.0, 50.0)));
        // Junk.
        assert!(!erase(&mut m, Point::new(f64::NAN, 0.0), 5.0));
        assert!(!erase(&mut m, Point::new(0.0, 0.0), -1.0));
    }

    #[test]
    fn eraser_catches_narrow_crossing_between_samples() {
        // A radius-1 brush on a long line: the crossing fits between the sampled parameters.
        let mut n = VectorNetwork::from_points(&[Point::new(0.0, 0.0), Point::new(6400.0, 0.0)], false);
        assert!(erase(&mut n, Point::new(5050.0, 0.0), 1.0));
        assert_eq!(n.segments.len(), 2);
        let ends: Vec<f64> = n.vertices.iter().map(|v| v.x).collect();
        assert!(ends.iter().any(|x| (x - 5049.0).abs() < 0.01) && ends.iter().any(|x| (x - 5051.0).abs() < 0.01), "{ends:?}");
        assert!(ends.iter().any(|x| x.abs() < 1e-9) && ends.iter().any(|x| (x - 6400.0).abs() < 1e-9), "{ends:?}");
        // A curved segment that only grazes the brush between samples.
        let mut c = VectorNetwork::from_points(&[Point::new(0.0, 0.0), Point::new(6400.0, 0.0)], false);
        c.segments[0].ta = (0.0, 3000.0);
        c.segments[0].tb = (0.0, 3000.0);
        let cb = c.cubic(0).unwrap();
        let t = 0.7039;
        let p = cb.eval(t);
        let d = figcraft_geom::kurbo::ParamCurveDeriv::deriv(&cb).eval(t).to_vec2().normalize();
        let normal = Vec2::new(-d.y, d.x);
        assert!(erase(&mut c, p + normal * 0.5, 1.0));
        assert_eq!(c.segments.len(), 2);
    }

    #[test]
    fn eraser_catches_crossing_at_full_speed_between_samples() {
        // The line moves fastest at t = 0.5; centre a tiny brush halfway between two samples there.
        let mut n = VectorNetwork::from_points(&[Point::new(0.0, 0.0), Point::new(6400.0, 0.0)], false);
        let centre = n.cubic(0).unwrap().eval(32.5 / 64.0);
        assert!(erase(&mut n, centre, 0.5));
        assert_eq!(n.segments.len(), 2);
        let ends: Vec<f64> = n.vertices.iter().map(|v| v.x).collect();
        assert!(ends.iter().any(|x| (x - (centre.x - 0.5)).abs() < 0.01), "{ends:?}");
        assert!(ends.iter().any(|x| (x - (centre.x + 0.5)).abs() < 0.01), "{ends:?}");
    }
}
