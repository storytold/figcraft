//! Connectors: lines whose ends attach to other layers and follow them.
//!
//! A connector keeps its two ends ([`ConnectorEnd`]) and a route (the points its line runs
//! through, in its own space). The route is derived: [`reroute`] recomputes it from where the
//! attached layers are, and the engine calls it after every edit that touches a connector or a
//! layer one of its ends is attached to. The connector's box is the route's bounds, so selection,
//! hit testing, export and rendering treat it like any other open path.

use crate::{Document, Node, NodeId, NodeKind};
use figcraft_geom::{Affine, BezPath, Point, Rect, Vec2};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// How a connector's line runs between its ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectorLineType {
    /// One straight segment.
    Straight,
    /// Horizontal and vertical segments; the bends are rounded ([`ELBOW_RADIUS`]).
    #[default]
    Elbowed,
    /// One smooth curve leaving and entering the attached sides at right angles.
    Curved,
}

impl ConnectorLineType {
    pub const ALL: [ConnectorLineType; 3] = [ConnectorLineType::Straight, ConnectorLineType::Elbowed, ConnectorLineType::Curved];

    pub fn label(self) -> &'static str {
        match self {
            ConnectorLineType::Straight => "Straight",
            ConnectorLineType::Elbowed => "Elbowed",
            ConnectorLineType::Curved => "Curved",
        }
    }

    /// Parse a name as commands and imported files spell it (`straight`, `ELBOWED`, `curved`…).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "straight" | "line" => Some(ConnectorLineType::Straight),
            "elbowed" | "elbow" => Some(ConnectorLineType::Elbowed),
            "curved" | "curve" => Some(ConnectorLineType::Curved),
            _ => None,
        }
    }
}

/// Where on an attached layer an end sits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Magnet {
    /// The middle of whichever side faces the other end.
    #[default]
    Auto,
    Top,
    Right,
    Bottom,
    Left,
    /// The layer's centre.
    Center,
}

impl Magnet {
    pub const ALL: [Magnet; 6] = [Magnet::Auto, Magnet::Top, Magnet::Right, Magnet::Bottom, Magnet::Left, Magnet::Center];

    pub fn label(self) -> &'static str {
        match self {
            Magnet::Auto => "Auto",
            Magnet::Top => "Top",
            Magnet::Right => "Right",
            Magnet::Bottom => "Bottom",
            Magnet::Left => "Left",
            Magnet::Center => "Center",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" | "none" => Some(Magnet::Auto),
            "top" => Some(Magnet::Top),
            "right" => Some(Magnet::Right),
            "bottom" => Some(Magnet::Bottom),
            "left" => Some(Magnet::Left),
            "center" | "centre" => Some(Magnet::Center),
            _ => None,
        }
    }

    /// The outward direction of the side this magnet names (`None` for the centre and auto).
    fn normal(self) -> Option<Vec2> {
        match self {
            Magnet::Top => Some(Vec2::new(0.0, -1.0)),
            Magnet::Right => Some(Vec2::new(1.0, 0.0)),
            Magnet::Bottom => Some(Vec2::new(0.0, 1.0)),
            Magnet::Left => Some(Vec2::new(-1.0, 0.0)),
            Magnet::Auto | Magnet::Center => None,
        }
    }
}

/// One end of a connector: attached to a layer, or free at a point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorEnd {
    /// The layer this end is attached to; `None` when it's free.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<NodeId>,
    #[serde(default, skip_serializing_if = "crate::node::is_default")]
    pub magnet: Magnet,
    /// Where the end is on the page: the free point, or where it last met its layer (kept so a
    /// connector whose layer is deleted stays where it was).
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
}

impl ConnectorEnd {
    /// A free end at page point (`x`, `y`).
    pub fn free(x: f64, y: f64) -> Self {
        Self { node: None, magnet: Magnet::Auto, x, y }
    }

    /// An end attached to `node` at `magnet`.
    pub fn attached(node: NodeId, magnet: Magnet) -> Self {
        Self { node: Some(node), magnet, x: 0.0, y: 0.0 }
    }

    pub fn point(&self) -> Point {
        Point::new(self.x, self.y)
    }
}

/// How far an elbowed route runs straight out of a side before it may turn back.
pub const ELBOW_STUB: f64 = 24.0;

/// The radius an elbowed line's bends are rounded to (less where its segments are shorter).
pub const ELBOW_RADIUS: f64 = 8.0;

/// The stroke weight of a thin connector (new connectors) and of a thick one.
pub const THIN: f64 = 2.0;
pub const THICK: f64 = 4.0;

/// The colour new connectors are drawn in: a mid grey, softer than the shapes they join.
pub const DEFAULT_COLOR: crate::color::Color = crate::color::Color::from_rgb8_const(0x75, 0x75, 0x75);

/// The layer `end` can attach to in `doc`, for a connector on `page`: it must exist on that page,
/// be visible in the tree, and not be a page or another connector.
fn target(doc: &Document, end: &ConnectorEnd, page: Option<NodeId>) -> Option<(NodeId, Rect)> {
    let id = end.node?;
    let n = doc.node(id)?;
    if n.is_page() || matches!(n.kind, NodeKind::Connector { .. }) || doc.page_of(id) != page {
        return None;
    }
    let r = doc.world_bounds(id)?;
    (r.x0.is_finite() && r.y0.is_finite() && r.x1.is_finite() && r.y1.is_finite()).then_some((id, r))
}

/// The side of `r` that faces `toward`.
fn facing(r: Rect, toward: Point) -> Magnet {
    let c = r.center();
    let (hw, hh) = ((r.width() / 2.0).max(1e-9), (r.height() / 2.0).max(1e-9));
    let (dx, dy) = ((toward.x - c.x) / hw, (toward.y - c.y) / hh);
    if dx.abs() >= dy.abs() {
        if dx >= 0.0 { Magnet::Right } else { Magnet::Left }
    } else if dy >= 0.0 {
        Magnet::Bottom
    } else {
        Magnet::Top
    }
}

fn magnet_point(r: Rect, m: Magnet) -> Point {
    let c = r.center();
    match m {
        Magnet::Top => Point::new(c.x, r.y0),
        Magnet::Right => Point::new(r.x1, c.y),
        Magnet::Bottom => Point::new(c.x, r.y1),
        Magnet::Left => Point::new(r.x0, c.y),
        Magnet::Auto | Magnet::Center => c,
    }
}

/// A resolved end: where it is on the page and which way the line leaves it (`None`: free, or a
/// centre, where any way goes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resolved {
    pub point: Point,
    pub normal: Option<Vec2>,
    pub attached: bool,
}

fn resolve(doc: &Document, end: &ConnectorEnd, page: Option<NodeId>, other: Point) -> Resolved {
    match target(doc, end, page) {
        Some((_, r)) => {
            let m = if end.magnet == Magnet::Auto { facing(r, other) } else { end.magnet };
            Resolved { point: magnet_point(r, m), normal: m.normal(), attached: true }
        }
        None => Resolved { point: end.point(), normal: None, attached: false },
    }
}

/// Where an end aims before it's resolved: its layer's centre, or its point.
fn aim(doc: &Document, end: &ConnectorEnd, page: Option<NodeId>) -> Point {
    target(doc, end, page).map(|(_, r)| r.center()).unwrap_or_else(|| end.point())
}

/// Both ends of a connector `n` resolved in `doc` (page coordinates).
pub fn resolve_ends(doc: &Document, n: &Node) -> Option<(Resolved, Resolved)> {
    let NodeKind::Connector { start, end, .. } = &n.kind else { return None };
    let page = doc.page_of(n.id);
    let (a, b) = (aim(doc, start, page), aim(doc, end, page));
    Some((resolve(doc, start, page, b), resolve(doc, end, page, a)))
}

/// The unit axis direction from `from` toward `to` (horizontal when they're level or as far apart
/// both ways).
fn axis_toward(from: Point, to: Point) -> Vec2 {
    let d = to - from;
    if d.x.abs() >= d.y.abs() { Vec2::new(if d.x >= 0.0 { 1.0 } else { -1.0 }, 0.0) } else { Vec2::new(0.0, if d.y >= 0.0 { 1.0 } else { -1.0 }) }
}

/// The route between two resolved ends, in page coordinates: two points for a straight line, the
/// corner points of an elbowed one, or the four points of one cubic curve.
pub fn route(a: Resolved, b: Resolved, line: ConnectorLineType) -> Vec<Point> {
    let (p0, p1) = (a.point, b.point);
    match line {
        ConnectorLineType::Straight => vec![p0, p1],
        ConnectorLineType::Elbowed => {
            let d0 = a.normal.unwrap_or_else(|| axis_toward(p0, p1));
            // A free end is entered on the axis the line leaves on, from the side facing the start.
            let d1 = b.normal.unwrap_or_else(|| {
                let back = |d: f64, fallback: f64| if d.abs() < 1e-9 { fallback } else { d.signum() };
                if d0.y == 0.0 { Vec2::new(back(p0.x - p1.x, -d0.x), 0.0) } else { Vec2::new(0.0, back(p0.y - p1.y, -d0.y)) }
            });
            simplify(elbow(p0, d0, p1, d1))
        }
        ConnectorLineType::Curved => {
            let len = (p1 - p0).hypot();
            let k = (len * 0.4).clamp(ELBOW_STUB, 400.0);
            let d0 = a.normal.unwrap_or_else(|| axis_toward(p0, p1));
            let d1 = b.normal.unwrap_or_else(|| axis_toward(p1, p0));
            vec![p0, p0 + d0 * k, p1 + d1 * k, p1]
        }
    }
}

/// Corner points from `p0` (leaving along `d0`) to `p1` (entered from its `d1` side).
fn elbow(p0: Point, d0: Vec2, p1: Point, d1: Vec2) -> Vec<Point> {
    let s = ELBOW_STUB;
    let (h0, h1) = (d0.y == 0.0, d1.y == 0.0);
    match (h0, h1) {
        (true, true) => {
            if d0.x * (p1.x - p0.x) > 0.0 && d1.x * (p0.x - p1.x) > 0.0 {
                let mx = (p0.x + p1.x) / 2.0;
                vec![p0, Point::new(mx, p0.y), Point::new(mx, p1.y), p1]
            } else if d0.x == d1.x {
                let x = if d0.x > 0.0 { p0.x.max(p1.x) + s } else { p0.x.min(p1.x) - s };
                vec![p0, Point::new(x, p0.y), Point::new(x, p1.y), p1]
            } else {
                let (a0, a1, my) = (p0.x + d0.x * s, p1.x + d1.x * s, (p0.y + p1.y) / 2.0);
                vec![p0, Point::new(a0, p0.y), Point::new(a0, my), Point::new(a1, my), Point::new(a1, p1.y), p1]
            }
        }
        (false, false) => {
            if d0.y * (p1.y - p0.y) > 0.0 && d1.y * (p0.y - p1.y) > 0.0 {
                let my = (p0.y + p1.y) / 2.0;
                vec![p0, Point::new(p0.x, my), Point::new(p1.x, my), p1]
            } else if d0.y == d1.y {
                let y = if d0.y > 0.0 { p0.y.max(p1.y) + s } else { p0.y.min(p1.y) - s };
                vec![p0, Point::new(p0.x, y), Point::new(p1.x, y), p1]
            } else {
                let (a0, a1, mx) = (p0.y + d0.y * s, p1.y + d1.y * s, (p0.x + p1.x) / 2.0);
                vec![p0, Point::new(p0.x, a0), Point::new(mx, a0), Point::new(mx, a1), Point::new(p1.x, a1), p1]
            }
        }
        (true, false) => {
            if d0.x * (p1.x - p0.x) > 0.0 && d1.y * (p0.y - p1.y) > 0.0 {
                vec![p0, Point::new(p1.x, p0.y), p1]
            } else {
                let (a0, a1) = (p0 + d0 * s, p1 + d1 * s);
                vec![p0, a0, Point::new(a0.x, a1.y), a1, p1]
            }
        }
        (false, true) => {
            if d0.y * (p1.y - p0.y) > 0.0 && d1.x * (p0.x - p1.x) > 0.0 {
                vec![p0, Point::new(p0.x, p1.y), p1]
            } else {
                let (a0, a1) = (p0 + d0 * s, p1 + d1 * s);
                vec![p0, a0, Point::new(a1.x, a0.y), a1, p1]
            }
        }
    }
}

/// Drop repeated points and corners that don't turn (an elbow between aligned layers is straight).
fn simplify(pts: Vec<Point>) -> Vec<Point> {
    const EPS: f64 = 1e-6;
    let mut out: Vec<Point> = Vec::with_capacity(pts.len());
    for p in pts {
        if out.last().is_some_and(|q| (p - *q).hypot() < EPS) {
            continue;
        }
        out.push(p);
        while out.len() >= 3 {
            let n = out.len();
            let (Some(a), Some(b), Some(c)) = (out.get(n - 3), out.get(n - 2), out.get(n - 1)) else { break };
            let collinear = ((a.x - b.x).abs() < EPS && (b.x - c.x).abs() < EPS) || ((a.y - b.y).abs() < EPS && (b.y - c.y).abs() < EPS);
            if !collinear {
                break;
            }
            out.remove(n - 2);
        }
    }
    if out.len() == 1
        && let Some(p) = out.first().copied()
    {
        out.push(p);
    }
    out
}

/// The connector's path in its own space, from its stored route.
pub fn path(line: ConnectorLineType, route: &[[f64; 2]]) -> BezPath {
    let mut p = BezPath::new();
    let pt = |i: usize| route.get(i).map(|q| Point::new(q[0], q[1]));
    let Some(first) = pt(0) else { return p };
    p.move_to(first);
    match (line, pt(1), pt(2), pt(3)) {
        (ConnectorLineType::Curved, Some(c0), Some(c1), Some(end)) if route.len() == 4 => p.curve_to(c0, c1, end),
        (ConnectorLineType::Elbowed, ..) => {
            for k in 1..route.len() {
                let (Some(b), Some(prev)) = (pt(k), pt(k - 1)) else { break };
                let Some(next) = pt(k + 1) else {
                    p.line_to(b);
                    break;
                };
                // Round the bend at `b`: in along the incoming segment, out along the next one.
                let (din, dout) = (b - prev, next - b);
                let (lin, lout) = (din.hypot(), dout.hypot());
                let r = ELBOW_RADIUS.min(lin / 2.0).min(lout / 2.0);
                if r < 1e-6 || !r.is_finite() {
                    p.line_to(b);
                    continue;
                }
                let (uin, uout) = (din / lin, dout / lout);
                // A quarter circle as one cubic (the usual 0.5523 handle length).
                let k = r * 0.552_284_75;
                let (a, c) = (b - uin * r, b + uout * r);
                p.line_to(a);
                p.curve_to(a + uin * k, c - uout * k, c);
            }
            if route.len() == 1 {
                p.line_to(first);
            }
        }
        _ => {
            for q in route.iter().skip(1) {
                p.line_to((q[0], q[1]));
            }
            if route.len() == 1 {
                p.line_to(first);
            }
        }
    }
    p
}

/// Recompute connector `id`'s route, box and end points from where its layers are now.
/// Free ends move with the connector when the connector itself was moved. Returns whether
/// anything changed (unchanged connectors aren't touched, so they stay clean).
pub fn reroute(doc: &mut Document, id: NodeId) -> bool {
    let Some(n) = doc.node(id) else { return false };
    let NodeKind::Connector { start, end, line_type, route: old } = &n.kind else { return false };
    let (mut start, mut end, line, old) = (*start, *end, *line_type, old.clone());
    let world = doc.world_transform(id);
    // A connector dragged as a whole carries its free ends along. (An empty route is one whose
    // ends were just set: they stay where they were put.)
    let shift = |i: usize, e: &ConnectorEnd| -> Option<Vec2> {
        let q = old.get(i)?;
        let now = world * Point::new(q[0], q[1]);
        let d = now - e.point();
        (d.hypot() > 1e-9 && d.x.is_finite() && d.y.is_finite()).then_some(d)
    };
    if start.node.is_none()
        && let Some(d) = shift(0, &start)
    {
        (start.x, start.y) = (start.x + d.x, start.y + d.y);
    }
    if end.node.is_none()
        && let Some(d) = old.len().checked_sub(1).and_then(|last| shift(last, &end))
    {
        (end.x, end.y) = (end.x + d.x, end.y + d.y);
    }
    let page = doc.page_of(id);
    // Ends whose layer is gone (or moved to another page) stay where they last were, free.
    for e in [&mut start, &mut end] {
        if e.node.is_some() && target(doc, e, page).is_none() {
            e.node = None;
        }
    }
    let (a_aim, b_aim) = (aim(doc, &start, page), aim(doc, &end, page));
    let (a, b) = (resolve(doc, &start, page, b_aim), resolve(doc, &end, page, a_aim));
    (start.x, start.y) = (a.point.x, a.point.y);
    (end.x, end.y) = (b.point.x, b.point.y);
    let pts = route(a, b, line);
    if pts.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return false;
    }
    let bounds = pts
        .iter()
        .skip(1)
        .fold(Rect::from_points(pts.first().copied().unwrap_or_default(), pts.first().copied().unwrap_or_default()), |r, p| r.union_pt(*p));
    let parent_world = n.parent.map(|p| doc.world_transform(p)).unwrap_or(Affine::IDENTITY);
    let transform = parent_world.inverse() * Affine::translate(bounds.origin().to_vec2());
    let local: Vec<[f64; 2]> = pts.iter().map(|p| [p.x - bounds.x0, p.y - bounds.y0]).collect();
    let kind = NodeKind::Connector { start, end, line_type: line, route: local };
    let same = n.kind == kind
        && (n.width - bounds.width()).abs() < 1e-9
        && (n.height - bounds.height()).abs() < 1e-9
        && n.transform.as_coeffs().iter().zip(transform.as_coeffs()).all(|(x, y)| (x - y).abs() < 1e-9);
    if same {
        return false;
    }
    let Some(m) = doc.node_mut(id) else { return false };
    m.kind = kind;
    m.transform = transform;
    m.width = bounds.width();
    m.height = bounds.height();
    true
}

/// Connectors in `doc` that must be re-routed after `touched` changed: connectors that were edited,
/// and connectors with an end on a touched layer, on a layer inside one, or on one that's gone.
pub fn affected(doc: &Document, touched: &BTreeSet<NodeId>) -> Vec<NodeId> {
    if touched.is_empty() {
        return Vec::new();
    }
    let hit = |e: &ConnectorEnd| -> bool {
        let Some(t) = e.node else { return false };
        !doc.contains(t) || touched.contains(&t) || doc.ancestors(t).iter().any(|a| touched.contains(a))
    };
    doc.nodes()
        .filter_map(|n| match &n.kind {
            NodeKind::Connector { start, end, .. } => {
                let own = touched.contains(&n.id) || doc.ancestors(n.id).iter().any(|a| touched.contains(a));
                (own || hit(start) || hit(end)).then_some(n.id)
            }
            _ => None,
        })
        .collect()
}

/// Re-route every connector [`affected`] by `touched`; returns the ones that changed.
pub fn follow(doc: &mut Document, touched: &BTreeSet<NodeId>) -> Vec<NodeId> {
    affected(doc, touched).into_iter().filter(|id| reroute(doc, *id)).collect()
}
