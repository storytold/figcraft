//! Convert the simple vector network used by text-on-path layers in `.fig` files into a native text path.

use figcraft_doc::{
    TextPath,
    geom::{
        BezPath,
        kurbo::{CubicBez, Line, ParamCurve, ParamCurveArclen, Point},
    },
};

use super::network::{Network, Segment};

const START_SEGMENT_STYLE: u32 = 0x01ff_ffff;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Use {
    segment: usize,
    forward: bool,
}

/// Build a native path from a validated connected chain or cycle. The marked source segment is
/// oriented a→b; `t` is local to that line/cubic. Coordinates are scaled into layer space.
pub(super) fn from_network(net: &Network, sx: f64, sy: f64, t: f64, forward: bool) -> Result<TextPath, &'static str> {
    if !t.is_finite() || !(0.0..=1.0).contains(&t) {
        return Err("invalid start parameter");
    }
    if !sx.is_finite() || !sy.is_finite() || sx <= 0.0 || sy <= 0.0 {
        return Err("invalid path scale");
    }
    let markers: Vec<usize> = net.segments.iter().enumerate().filter_map(|(i, s)| (s.style == START_SEGMENT_STYLE).then_some(i)).collect();
    let [marked] = markers.as_slice() else {
        return Err("missing or ambiguous start segment marker");
    };

    let (uses, closed) = ordered_chain(net, *marked)?;
    let mut d = BezPath::new();
    let mut total = 0.0;
    let mut prefix = 0.0;
    let mut start_segment_length = 0.0;
    for use_ in &uses {
        let (p0, p1, p2, p3) = cubic(net, *use_, sx, sy)?;
        if d.elements().is_empty() {
            d.move_to((p0.0, p0.1));
        }
        let line = p1 == p0 && p2 == p3;
        let (length, partial) = if line {
            d.line_to((p3.0, p3.1));
            let line = Line::new(Point::new(p0.0, p0.1), Point::new(p3.0, p3.1));
            (line.arclen(1e-9), line.arclen(1e-9) * t)
        } else {
            d.curve_to((p1.0, p1.1), (p2.0, p2.1), (p3.0, p3.1));
            let curve = CubicBez::new((p0.0, p0.1), (p1.0, p1.1), (p2.0, p2.1), (p3.0, p3.1));
            let partial = if t <= 0.0 { 0.0 } else { curve.subsegment(0.0..t).arclen(1e-9) };
            (curve.arclen(1e-9), partial)
        };
        if !length.is_finite() || length <= 1e-10 {
            return Err("zero-length path segment");
        }
        if use_.segment == *marked {
            start_segment_length = partial;
            if !start_segment_length.is_finite() {
                return Err("invalid path length");
            }
            prefix = total;
        }
        total += length;
    }
    if closed {
        d.close_path();
    }
    if !total.is_finite() || total <= 1e-10 {
        return Err("zero-length path");
    }
    let mut start = ((prefix + start_segment_length) / total).clamp(0.0, 1.0);
    let flip = !forward;
    if flip {
        start = 1.0 - start;
    }
    let path = TextPath { d: d.to_svg(), start, flip };
    let lines = figcraft_doc::geom::measure::polylines(&path.path()).ok_or("cannot be measured by text renderer")?;
    if lines.first().is_none_or(|line| line.length() <= 1e-9) {
        return Err("cannot be measured by text renderer");
    }
    Ok(path)
}

/// Return every segment in chain order. Open chains are oriented so the marked segment runs a→b;
/// cycles start at the marked segment's a vertex and traverse it first.
fn ordered_chain(net: &Network, marked: usize) -> Result<(Vec<Use>, bool), &'static str> {
    let marker = net.segments.get(marked).ok_or("invalid start segment index")?;
    let mut adjacency = vec![Vec::<usize>::new(); net.vertices.len()];
    for (i, s) in net.segments.iter().enumerate() {
        if s.a >= adjacency.len() || s.b >= adjacency.len() {
            return Err("invalid path vertex index");
        }
        if s.a == s.b {
            return Err("path contains a segment that starts and ends at the same vertex");
        }
        adjacency[s.a].push(i);
        adjacency[s.b].push(i);
    }
    let mut endpoints = Vec::new();
    for edges in &adjacency {
        if edges.len() > 2 {
            return Err("branched path network");
        }
        if edges.len() == 1 {
            endpoints.push(edges[0]);
        }
    }
    if !endpoints.is_empty() && endpoints.len() != 2 {
        return Err("disconnected or malformed path network");
    }
    let cycle = endpoints.is_empty();
    let mut used = vec![false; net.segments.len()];
    let mut out = Vec::with_capacity(net.segments.len());
    if cycle {
        out.push(Use { segment: marked, forward: true });
        used[marked] = true;
        let mut at = marker.b;
        let mut visited = vec![false; net.vertices.len()];
        visited[marker.a] = true;
        visited[marker.b] = true;
        while at != marker.a {
            let next = adjacency[at].iter().copied().find(|i| !used[*i]).ok_or("disconnected path cycle")?;
            used[next] = true;
            let s = &net.segments[next];
            let is_forward = s.a == at;
            at = if is_forward { s.b } else { s.a };
            if at != marker.a && std::mem::replace(&mut visited[at], true) {
                return Err("path cycle is not simple");
            }
            out.push(Use { segment: next, forward: is_forward });
            if out.len() > net.segments.len() {
                return Err("invalid path cycle");
            }
        }
    } else {
        let endpoint = adjacency.iter().position(|e| e.len() == 1).ok_or("missing path endpoint")?;
        let mut at = endpoint;
        while let Some(next) = adjacency[at].iter().copied().find(|i| !used[*i]) {
            used[next] = true;
            let s = &net.segments[next];
            let is_forward = s.a == at;
            at = if is_forward { s.b } else { s.a };
            out.push(Use { segment: next, forward: is_forward });
        }
        let Some(marker_use) = out.iter().find(|u| u.segment == marked) else { return Err("disconnected start segment") };
        if !marker_use.forward {
            out.reverse();
            for u in &mut out {
                u.forward = !u.forward;
            }
        }
    }
    if !used.iter().all(|x| *x) {
        return Err("disconnected path network");
    }
    Ok((out, cycle))
}

type P = (f64, f64);

fn cubic(net: &Network, use_: Use, sx: f64, sy: f64) -> Result<(P, P, P, P), &'static str> {
    let s: &Segment = net.segments.get(use_.segment).ok_or("invalid path segment index")?;
    let a = net.vertices.get(s.a).ok_or("invalid path vertex index")?;
    let b = net.vertices.get(s.b).ok_or("invalid path vertex index")?;
    let p0 = (a.x * sx, a.y * sy);
    let p3 = (b.x * sx, b.y * sy);
    let ta = (s.ta.0 * sx, s.ta.1 * sy);
    let tb = (s.tb.0 * sx, s.tb.1 * sy);
    if [p0.0, p0.1, p3.0, p3.1, ta.0, ta.1, tb.0, tb.1].iter().any(|x| !x.is_finite()) {
        return Err("invalid path coordinates");
    }
    let (p0, p1, p2, p3) =
        if ta == (0.0, 0.0) && tb == (0.0, 0.0) { (p0, p0, p3, p3) } else { (p0, (p0.0 + ta.0, p0.1 + ta.1), (p3.0 + tb.0, p3.1 + tb.1), p3) };
    if use_.forward { Ok((p0, p1, p2, p3)) } else { Ok((p3, p2, p1, p0)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fig::network::{Segment, Vertex};
    use figcraft_doc::geom::kurbo::PathEl;

    fn vertex(x: f64, y: f64) -> Vertex {
        Vertex { style: 0, x, y }
    }

    fn segment(style: u32, a: usize, b: usize, ta: (f64, f64), tb: (f64, f64)) -> Segment {
        Segment { style, a, ta, b, tb }
    }

    fn curved() -> Network {
        Network {
            vertices: vec![vertex(0.0, 82.5), vertex(300.0, 82.5), vertex(600.0, 82.5)],
            segments: vec![segment(START_SEGMENT_STYLE, 0, 1, (70.0, -110.0), (-70.0, -110.0)), segment(0, 1, 2, (70.0, 110.0), (-70.0, 110.0))],
            ..Network::default()
        }
    }

    #[test]
    fn local_cubic_parameter_converts_to_path_distance() {
        let path = from_network(&curved(), 1.0, 1.0, 0.0086954795, true).unwrap();
        assert!((path.start - 0.004744078).abs() < 0.00001);
        // Cubic t is not its fraction of total path length.
        assert!((path.start - 0.0086954795).abs() > 0.003);
        let elements = path.path().elements().to_vec();
        assert!(matches!(elements.first(), Some(PathEl::MoveTo(p)) if p.x == 0.0 && p.y == 82.5));
        assert!(matches!(elements.get(1), Some(PathEl::CurveTo(..))));
    }

    #[test]
    fn second_segment_marker_and_t_are_respected() {
        let mut net = curved();
        net.segments[0].style = 0;
        net.segments[1].style = START_SEGMENT_STYLE;
        let path = from_network(&net, 1.0, 1.0, 0.49800667, true).unwrap();
        assert!((path.start - 0.7490379757).abs() < 0.00002);
    }

    #[test]
    fn reverse_direction_inverts_normalized_start() {
        let net = Network {
            vertices: vec![vertex(0.0, 0.0), vertex(600.0, 0.0)],
            segments: vec![segment(START_SEGMENT_STYLE, 0, 1, (0.0, 0.0), (0.0, 0.0))],
            ..Network::default()
        };
        let path = from_network(&net, 1.0, 1.0, 0.83986014, false).unwrap();
        assert!(path.flip);
        assert!((path.start - 0.16013986).abs() < 1e-8);
        assert!(matches!(path.path().elements().get(1), Some(PathEl::LineTo(p)) if p.x == 600.0 && p.y == 0.0));
    }

    #[test]
    fn rejects_missing_duplicate_and_zero_length_segments() {
        let mut net = curved();
        net.segments[0].style = 0;
        assert_eq!(from_network(&net, 1.0, 1.0, 0.5, true), Err("missing or ambiguous start segment marker"));
        net.segments[1].style = START_SEGMENT_STYLE;
        net.segments[0].style = START_SEGMENT_STYLE;
        assert_eq!(from_network(&net, 1.0, 1.0, 0.5, true), Err("missing or ambiguous start segment marker"));
        let zero = Network {
            vertices: vec![vertex(0.0, 0.0), vertex(0.0, 0.0)],
            segments: vec![segment(START_SEGMENT_STYLE, 0, 1, (0.0, 0.0), (0.0, 0.0))],
            ..Network::default()
        };
        assert_eq!(from_network(&zero, 1.0, 1.0, 0.5, true), Err("zero-length path segment"));
    }

    #[test]
    fn rejects_branching_and_disconnected_segments() {
        let branch = Network {
            vertices: vec![vertex(0.0, 0.0), vertex(1.0, 0.0), vertex(2.0, 0.0), vertex(1.0, 1.0)],
            segments: vec![
                segment(START_SEGMENT_STYLE, 0, 1, (0.0, 0.0), (0.0, 0.0)),
                segment(0, 1, 2, (0.0, 0.0), (0.0, 0.0)),
                segment(0, 1, 3, (0.0, 0.0), (0.0, 0.0)),
            ],
            ..Network::default()
        };
        assert_eq!(from_network(&branch, 1.0, 1.0, 0.5, true), Err("branched path network"));
        let disconnected = Network {
            vertices: vec![vertex(0.0, 0.0), vertex(1.0, 0.0), vertex(2.0, 0.0), vertex(3.0, 0.0)],
            segments: vec![segment(START_SEGMENT_STYLE, 0, 1, (0.0, 0.0), (0.0, 0.0)), segment(0, 2, 3, (0.0, 0.0), (0.0, 0.0))],
            ..Network::default()
        };
        assert_eq!(from_network(&disconnected, 1.0, 1.0, 0.5, true), Err("disconnected or malformed path network"));
    }

    #[test]
    fn scales_geometry_and_accepts_closed_cycles() {
        let cycle = Network {
            vertices: vec![vertex(0.0, 0.0), vertex(100.0, 0.0), vertex(100.0, 100.0)],
            segments: vec![
                segment(START_SEGMENT_STYLE, 0, 1, (0.0, 0.0), (0.0, 0.0)),
                segment(0, 1, 2, (0.0, 0.0), (0.0, 0.0)),
                segment(0, 2, 0, (0.0, 0.0), (0.0, 0.0)),
            ],
            ..Network::default()
        };
        let path = from_network(&cycle, 2.0, 3.0, 0.5, true).unwrap();
        let elements = path.path().elements().to_vec();
        assert!(matches!(elements.first(), Some(PathEl::MoveTo(p)) if p.x == 0.0 && p.y == 0.0));
        assert!(matches!(elements.get(1), Some(PathEl::LineTo(p)) if p.x == 200.0 && p.y == 0.0));
        let total = 200.0 + 300.0 + 130_000.0_f64.sqrt();
        assert!((path.start - 100.0 / total).abs() < 1e-8);
        assert!(matches!(elements.last(), Some(PathEl::ClosePath)));
    }

    #[test]
    fn rejects_paths_the_native_renderer_cannot_measure() {
        let count = 40_000usize;
        let many_lines = Network {
            vertices: (0..=count).map(|i| vertex(i as f64, 0.0)).collect(),
            segments: (0..count).map(|i| segment(if i == 0 { START_SEGMENT_STYLE } else { 0 }, i, i + 1, (0.0, 0.0), (0.0, 0.0))).collect(),
            ..Network::default()
        };
        assert_eq!(from_network(&many_lines, 1.0, 1.0, 0.5, true), Err("cannot be measured by text renderer"));

        let huge = Network {
            vertices: vec![vertex(0.0, 0.0), vertex(1e9, 0.0)],
            segments: vec![segment(START_SEGMENT_STYLE, 0, 1, (0.0, 0.0), (0.0, 0.0))],
            ..Network::default()
        };
        assert_eq!(from_network(&huge, 1.0, 1.0, 0.5, true), Err("cannot be measured by text renderer"));

        let tiny = Network {
            vertices: vec![vertex(0.0, 0.0), vertex(5e-10, 0.0)],
            segments: vec![segment(START_SEGMENT_STYLE, 0, 1, (0.0, 0.0), (0.0, 0.0))],
            ..Network::default()
        };
        assert_eq!(from_network(&tiny, 1.0, 1.0, 0.5, true), Err("cannot be measured by text renderer"));
    }
}
