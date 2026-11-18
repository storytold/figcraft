//! Measuring: red distance lines with value labels between bounding boxes in page space (rotated
//! layers by their bounds, several selected layers by their union).
//!
//! - Alt-hover: from the selection to the layer under the pointer (Cmd+Alt: the nested layer),
//!   to its parent (over the selection itself or empty space inside the parent) or to a ruler
//!   guide. One box inside the other measures to each edge of the outer one; boxes apart measure
//!   the gaps, with dashed extension lines to the other box when the line passes beside it; boxes
//!   that overlap measure between their matching edges.
//! - Alt while dragging layers: live distances to the nearest neighbours on each side within the
//!   parent (to the parent's edges where there is none).
//! - Shift+Alt over an auto layout frame: its padding and the gaps between its children.
//! - Alt while dragging a ruler guide across a frame: from the guide to the nearest edges inside.
//!
//! The canvas draws the lines and places the labels so they never overlap (see the UI crate).

use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_geom::{Point, Rect};
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::params::{id, targets};
use crate::snap::Guide;
use crate::{Result, Session, bad, spec};

/// Lines shorter than this (page units) aren't drawn.
const EPS: f64 = 1e-6;
/// The most layers looked at for neighbours and gaps.
const MAX_BOXES: usize = 512;

/// A distance as shown on a label: up to two decimals, no trailing zeros.
pub fn label(d: f64) -> String {
    let v = (d.abs() * 100.0).round() / 100.0;
    if v.fract() == 0.0 && v < 1e15 { format!("{}", v as i64) } else { format!("{v}") }
}

/// A labelled distance line, unless it has no length.
pub(crate) fn measured(out: &mut Vec<Guide>, a: Point, b: Point) {
    let d = (b - a).hypot();
    if d > EPS && d.is_finite() {
        out.push(Guide { label: Some(label(d)), ..Guide::new(a, b) });
    }
}

/// A dashed extension line (no label), unless it has no length.
fn extension(out: &mut Vec<Guide>, a: Point, b: Point) {
    if (b - a).hypot() > EPS {
        out.push(Guide { dashed: true, ..Guide::new(a, b) });
    }
}

fn contains(outer: Rect, inner: Rect) -> bool {
    outer.x0 <= inner.x0 + EPS && outer.y0 <= inner.y0 + EPS && outer.x1 >= inner.x1 - EPS && outer.y1 >= inner.y1 - EPS
}

/// The middle of the overlap of two spans, if they overlap.
fn overlap_mid(a0: f64, a1: f64, b0: f64, b1: f64) -> Option<f64> {
    let (lo, hi) = (a0.max(b0), a1.min(b1));
    (lo <= hi).then_some((lo + hi) / 2.0)
}

/// Distance lines from box `a` (the selection) to box `b`.
pub fn between(a: Rect, b: Rect) -> Vec<Guide> {
    let mut out = Vec::new();
    if !(a.is_finite() && b.is_finite()) {
        return out;
    }
    if contains(b, a) {
        inside(&mut out, a, b);
        return out;
    }
    if contains(a, b) {
        inside(&mut out, b, a);
        return out;
    }
    let ox = overlap_mid(a.x0, a.x1, b.x0, b.x1);
    let oy = overlap_mid(a.y0, a.y1, b.y0, b.y1);
    let overlapping = a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1;
    if let (true, Some(mx), Some(my)) = (overlapping, ox, oy) {
        // Overlapping: between matching edges.
        measured(&mut out, Point::new(a.x0, my), Point::new(b.x0, my));
        measured(&mut out, Point::new(a.x1, my), Point::new(b.x1, my));
        measured(&mut out, Point::new(mx, a.y0), Point::new(mx, b.y0));
        measured(&mut out, Point::new(mx, a.y1), Point::new(mx, b.y1));
        return out;
    }
    let c = a.center();
    // Horizontal gap: along the shared rows, or from the middle of `a` with an extension to `b`.
    let gap_x = if b.x0 >= a.x1 {
        Some((a.x1, b.x0))
    } else if b.x1 <= a.x0 {
        Some((a.x0, b.x1))
    } else {
        None
    };
    if let Some((from, to)) = gap_x {
        let y = oy.unwrap_or(c.y);
        measured(&mut out, Point::new(from, y), Point::new(to, y));
        if oy.is_none() {
            extension(&mut out, Point::new(to, y), Point::new(to, if y < b.y0 { b.y0 } else { b.y1 }));
        }
    }
    let gap_y = if b.y0 >= a.y1 {
        Some((a.y1, b.y0))
    } else if b.y1 <= a.y0 {
        Some((a.y0, b.y1))
    } else {
        None
    };
    if let Some((from, to)) = gap_y {
        let x = ox.unwrap_or(c.x);
        measured(&mut out, Point::new(x, from), Point::new(x, to));
        if ox.is_none() {
            extension(&mut out, Point::new(x, to), Point::new(if x < b.x0 { b.x0 } else { b.x1 }, to));
        }
    }
    out
}

/// From `inner` to each edge of `outer`, along `inner`'s centre lines.
fn inside(out: &mut Vec<Guide>, inner: Rect, outer: Rect) {
    let c = inner.center();
    measured(out, Point::new(outer.x0, c.y), Point::new(inner.x0, c.y));
    measured(out, Point::new(inner.x1, c.y), Point::new(outer.x1, c.y));
    measured(out, Point::new(c.x, outer.y0), Point::new(c.x, inner.y0));
    measured(out, Point::new(c.x, inner.y1), Point::new(c.x, outer.y1));
}

/// Distance lines from box `a` to a ruler guide (`vertical` at x = `at`, else at y = `at`): to
/// the nearer side, or both sides when the guide crosses the box.
pub fn to_guide(a: Rect, vertical: bool, at: f64) -> Vec<Guide> {
    let mut out = Vec::new();
    if !(a.is_finite() && at.is_finite()) {
        return out;
    }
    let c = a.center();
    let (lo, hi) = if vertical { (a.x0, a.x1) } else { (a.y0, a.y1) };
    let pt = |v: f64| if vertical { Point::new(v, c.y) } else { Point::new(c.x, v) };
    if at >= hi {
        measured(&mut out, pt(hi), pt(at));
    } else if at <= lo {
        measured(&mut out, pt(at), pt(lo));
    } else {
        measured(&mut out, pt(lo), pt(at));
        measured(&mut out, pt(at), pt(hi));
    }
    out
}

/// Alt while dragging a ruler guide (`vertical` at page x = `at`, else at y = `at`) across frame
/// `frame`: the distances from it to the nearest edge on each side, of the frame's visible layers
/// or the frame itself, across the middle of what each measures to. Nothing when it misses the frame.
pub fn from_guide(doc: &Document, frame: NodeId, vertical: bool, at: f64) -> Vec<Guide> {
    let mut out = Vec::new();
    let Some(f) = doc.world_bounds(frame).filter(|f| f.is_finite() && at.is_finite()) else { return out };
    // Along the guide's normal: (start, end, middle across) of a box.
    let span = |b: Rect| if vertical { (b.x0, b.x1, b.center().y) } else { (b.y0, b.y1, b.center().x) };
    let (lo, hi, mid) = span(f);
    if !(at > lo && at < hi) {
        return out;
    }
    let (mut before, mut after) = ((lo, mid), (hi, mid));
    for b in sibling_boxes(doc, frame, &[]) {
        let (b0, b1, m) = span(b);
        for e in [b0, b1] {
            if e <= at && e > before.0 {
                before = (e, m);
            }
            if e >= at && e < after.0 {
                after = (e, m);
            }
        }
    }
    let pt = |along: f64, across: f64| if vertical { Point::new(along, across) } else { Point::new(across, along) };
    measured(&mut out, pt(before.0, before.1), pt(at, before.1));
    measured(&mut out, pt(at, after.1), pt(after.0, after.1));
    out
}

/// The ruler guides near page point `p` (within `tol`): the page's and those of the top-level
/// frames around `around` (those that run along the page's axes), as (vertical, page offset).
pub fn guide_at(doc: &Document, page: NodeId, around: &[NodeId], p: Point, tol: f64) -> Option<(bool, f64)> {
    let mut lines: Vec<(bool, f64)> = Vec::new();
    if let Some(n) = doc.node(page) {
        lines.extend(n.guides().iter().map(|g| (g.vertical, g.offset)));
    }
    let mut frames: Vec<NodeId> = Vec::new();
    for f in around.iter().filter_map(|a| doc.top_level_frame(*a)) {
        if !frames.contains(&f) {
            frames.push(f);
        }
    }
    for f in frames {
        let Some(n) = doc.node(f).filter(|n| !n.guides.is_empty()) else { continue };
        let w = doc.world_transform(f);
        lines.extend(n.guides.iter().filter_map(|g| crate::guides::line_in(w, g.vertical, g.offset)));
    }
    lines
        .into_iter()
        .filter(|(v, at)| at.is_finite() && (if *v { p.x } else { p.y } - at).abs() <= tol)
        .min_by(|a, b| (if a.0 { p.x } else { p.y } - a.1).abs().total_cmp(&(if b.0 { p.x } else { p.y } - b.1).abs()))
}

/// The visible layers in `parent` other than `moving` (and not inside them), by their bounds.
fn sibling_boxes(doc: &Document, parent: NodeId, moving: &[NodeId]) -> Vec<Rect> {
    let Some(p) = doc.node(parent) else { return Vec::new() };
    p.children
        .iter()
        .filter(|c| !moving.contains(c) && doc.node(**c).is_some_and(|n| n.visible))
        .take(MAX_BOXES)
        .filter_map(|c| doc.world_bounds(*c))
        .filter(|b| b.is_finite())
        .collect()
}

/// Live distances while dragging `moving` (bounds `a`): to the nearest sibling on each side whose
/// span faces `a`, or to the parent's edge on sides without one (the page has no edges).
pub fn neighbours(doc: &Document, page: NodeId, moving: &[NodeId], a: Rect) -> Vec<Guide> {
    let mut out = Vec::new();
    if !a.is_finite() {
        return out;
    }
    let parent = moving.first().and_then(|m| doc.node(*m)).and_then(|n| n.parent).unwrap_or(page);
    let boxes = sibling_boxes(doc, parent, moving);
    let frame = (parent != page).then(|| doc.world_bounds(parent)).flatten().filter(|f| f.is_finite());
    let c = a.center();
    // For each side: the nearest facing box edge (its position and the shared span's middle).
    let nearest = |side: u8| -> Option<(f64, f64)> {
        boxes
            .iter()
            .filter_map(|b| {
                let (edge, mid, dist) = match side {
                    0 => (b.x1, overlap_mid(a.y0, a.y1, b.y0, b.y1)?, a.x0 - b.x1),
                    1 => (b.x0, overlap_mid(a.y0, a.y1, b.y0, b.y1)?, b.x0 - a.x1),
                    2 => (b.y1, overlap_mid(a.x0, a.x1, b.x0, b.x1)?, a.y0 - b.y1),
                    _ => (b.y0, overlap_mid(a.x0, a.x1, b.x0, b.x1)?, b.y0 - a.y1),
                };
                (dist >= -EPS).then_some((edge, mid, dist))
            })
            .min_by(|x, y| x.2.total_cmp(&y.2))
            .map(|(edge, mid, _)| (edge, mid))
    };
    for side in 0..4u8 {
        let hit = nearest(side).or_else(|| {
            let f = frame.filter(|f| contains(*f, a))?;
            Some(match side {
                0 => (f.x0, c.y),
                1 => (f.x1, c.y),
                2 => (f.y0, c.x),
                _ => (f.y1, c.x),
            })
        });
        let Some((edge, mid)) = hit else { continue };
        let (p, q) = match side {
            0 => (Point::new(edge, mid), Point::new(a.x0, mid)),
            1 => (Point::new(a.x1, mid), Point::new(edge, mid)),
            2 => (Point::new(mid, edge), Point::new(mid, a.y0)),
            _ => (Point::new(mid, a.y1), Point::new(mid, edge)),
        };
        measured(&mut out, p, q);
    }
    out
}

/// The auto layout frame Shift+Alt reads out for `target`: itself, or its parent.
pub fn padding_frame(doc: &Document, target: NodeId) -> Option<NodeId> {
    let has_layout = |i: NodeId| doc.node(i).is_some_and(|n| n.layout.is_some());
    if has_layout(target) {
        return Some(target);
    }
    doc.node(target).and_then(|n| n.parent).filter(|p| has_layout(*p))
}

/// An auto layout frame's padding (from each edge, across the middle of the content) and the
/// gaps between neighbouring children in its flow, in page space (rotated frames too).
pub fn padding(doc: &Document, frame: NodeId) -> Vec<Guide> {
    let mut out = Vec::new();
    let Some(n) = doc.node(frame) else { return out };
    let Some(l) = n.layout.as_ref() else { return out };
    let w = doc.world_transform(frame);
    // Flow children in the frame's own space.
    let kids: Vec<Rect> = n
        .children
        .iter()
        .filter_map(|c| doc.node(*c))
        .filter(|c| c.visible && !c.child_layout.absolute && !matches!(c.kind, NodeKind::Slice))
        .take(MAX_BOXES)
        .map(|c| figcraft_geom::transform_rect_bbox(c.transform, c.local_rect()))
        .filter(|r| r.is_finite())
        .collect();
    let content = kids.iter().copied().reduce(|a, b| a.union(b));
    let (cx, cy) = content.map_or((n.width / 2.0, n.height / 2.0), |r| (r.center().x, r.center().y));
    let [top, right, bottom, left] = l.padding.map(|v| if v.is_finite() { v.max(0.0) } else { 0.0 });
    let mut push = |a: Point, b: Point| measured(&mut out, w * a, w * b);
    push(Point::new(0.0, cy), Point::new(left, cy));
    push(Point::new(n.width - right, cy), Point::new(n.width, cy));
    push(Point::new(cx, 0.0), Point::new(cx, top));
    push(Point::new(cx, n.height - bottom), Point::new(cx, n.height));
    for (i, a) in kids.iter().enumerate() {
        // The nearest child to the right sharing rows, and below sharing columns.
        let right = kids
            .iter()
            .enumerate()
            .filter(|(j, b)| *j != i && b.x0 >= a.x1 - EPS && overlap_mid(a.y0, a.y1, b.y0, b.y1).is_some())
            .min_by(|x, y| x.1.x0.total_cmp(&y.1.x0));
        if let Some((_, b)) = right
            && let Some(my) = overlap_mid(a.y0, a.y1, b.y0, b.y1)
        {
            push(Point::new(a.x1, my), Point::new(b.x0, my));
        }
        let below = kids
            .iter()
            .enumerate()
            .filter(|(j, b)| *j != i && b.y0 >= a.y1 - EPS && overlap_mid(a.x0, a.x1, b.x0, b.x1).is_some())
            .min_by(|x, y| x.1.y0.total_cmp(&y.1.y0));
        if let Some((_, b)) = below
            && let Some(mx) = overlap_mid(a.x0, a.x1, b.x0, b.x1)
        {
            push(Point::new(mx, a.y1), Point::new(mx, b.y0));
        }
    }
    out
}

/// What Alt-hover measures to from a selection: a layer, a ruler guide, or an auto layout frame's
/// padding (Shift+Alt).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hover {
    Layer(NodeId),
    Guide { vertical: bool, at: f64 },
    Padding(NodeId),
}

/// The measurement lines for `hover` from the selection `sel` and the box to outline (the
/// measured-to layer or frame).
pub fn hover_lines(doc: &Document, page: NodeId, sel: &[NodeId], hover: Hover) -> (Vec<Guide>, Option<Rect>) {
    match hover {
        Hover::Padding(f) => (padding(doc, f), doc.world_bounds(f)),
        Hover::Guide { vertical, at } => (doc.bounds_of(sel).map(|a| to_guide(a, vertical, at)).unwrap_or_default(), None),
        Hover::Layer(t) => {
            let Some(a) = doc.bounds_of(sel) else { return (Vec::new(), None) };
            // Over the selection itself: to its parent.
            let t = if sel.contains(&t) { parent_of(doc, page, sel).unwrap_or(t) } else { t };
            if sel.contains(&t) {
                return (Vec::new(), None);
            }
            match doc.world_bounds(t) {
                Some(b) => (between(a, b), Some(b)),
                None => (Vec::new(), None),
            }
        }
    }
}

/// The selection's parent when it isn't the page.
pub fn parent_of(doc: &Document, page: NodeId, sel: &[NodeId]) -> Option<NodeId> {
    sel.first().and_then(|i| doc.node(*i)).and_then(|n| n.parent).filter(|p| *p != page && !doc.node(*p).is_some_and(|n| n.is_page()))
}

fn lines_json(lines: &[Guide]) -> Value {
    json!({"lines": lines})
}

fn measure_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let doc = s.doc()?;
    let page = s.page()?;
    let sel = targets(s, p);
    for i in &sel {
        if !doc.contains(*i) {
            return Err(crate::EngineError::NoNode(*i));
        }
    }
    if sel.is_empty() {
        return Err(bad("measure.distances", "nothing is selected (pass `ids`)"));
    }
    let to = match id(p, "to") {
        Some(t) if doc.contains(t) => t,
        Some(t) => return Err(crate::EngineError::NoNode(t)),
        None => parent_of(doc, page, &sel).ok_or_else(|| bad("measure.distances", "pass `to` (the selection is at the top level)"))?,
    };
    let (lines, _) = hover_lines(doc, page, &sel, Hover::Layer(to));
    Ok(lines_json(&lines))
}

fn padding_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let doc = s.doc()?;
    let target = targets(s, p).first().copied().ok_or_else(|| bad("measure.padding", "pass `id` (an auto layout frame or one of its children)"))?;
    let frame = padding_frame(doc, target).ok_or_else(|| bad("measure.padding", "not in an auto layout frame"))?;
    Ok(json!({"frame": frame, "lines": padding(doc, frame)}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "measure.distances",
            "Measure distances",
            "{ids? (default selection), to? (layer id; default the parent)} → {lines: [{x0, y0, x1, y1, label, dashed}]} (page space; what Alt-hover shows)",
            false,
            measure_cmd
        ),
        spec!(
            "measure.padding",
            "Measure auto layout padding",
            "{id? (an auto layout frame or a child; default the selection)} → {frame, lines} (padding and gaps; what Shift+Alt-hover shows)",
            false,
            padding_cmd
        ),
    ]
}
