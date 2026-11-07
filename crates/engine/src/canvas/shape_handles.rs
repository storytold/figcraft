//! Shape handles: the on-canvas handles that round a selected rectangle's, frame's, polygon's or
//! star's corners and set a star's ratio.
//!
//! - **Radius**: a rectangle or frame has one inside each corner; a polygon or star one inside its
//!   top point. Each sits on the corner's bisector at the centre of the corner's arc (at least
//!   [`INSET_PX`] in, so it can be grabbed at radius 0). Dragging it along the bisector sets the
//!   radius of every corner; with Alt, only that corner of a rectangle or frame (the documentation
//!   is silent on Alt).
//! - **Ratio**: a star's sits on its first inner point; dragging it towards or away from the
//!   centre sets the inner radius as a share of the outer one.
//!
//! Positions are in page coordinates and work for rotated and flipped layers at any zoom; the
//! handles hide on layers smaller than [`MIN_PX`] on screen. The Move tool's gesture (`tools.rs`)
//! writes the result through `node.set` (`cornerRadius`, `topLeftRadius`…, `innerRadius`), one
//! undo step per drag.

use figcraft_doc::{Document, Node, NodeId, NodeKind};
use figcraft_geom::{Affine, Point, Vec2};
use serde::Serialize;
use serde_json::{Map, Value, json};

/// One shape handle: a corner's radius (corner index: tl, tr, br, bl; 0 for a polygon's or
/// star's top point) or a star's ratio.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ShapeHandle {
    Radius { corner: usize },
    Ratio,
}

/// Layers smaller than this on screen (either side, in pixels) show no shape handles.
pub const MIN_PX: f64 = 48.0;
/// How far in from its corner a handle sits at least (screen pixels).
pub const INSET_PX: f64 = 10.0;
/// How near the pointer must be to pick a handle (screen pixels).
const HIT_PX: f64 = 6.0;

/// A corner as its handle sees it (local coordinates): the point, the unit bisector into the
/// shape, the half-angle there, and the largest radius that fits.
struct Corner {
    at: Point,
    into: Vec2,
    half: f64,
    max_r: f64,
}

impl Corner {
    /// Its handle for radius `r` at `inset` local units in at least.
    fn handle(&self, r: f64, inset: f64) -> Point {
        self.at + self.into * (r.clamp(0.0, self.max_r) / self.half.sin()).max(inset)
    }

    /// The radius a handle dragged to local point `q` sets.
    fn radius(&self, q: Point) -> f64 {
        ((q - self.at).dot(self.into) * self.half.sin()).clamp(0.0, self.max_r)
    }
}

/// The corner at `cur` between `prev` and `next` (a polygon's), `None` when degenerate.
fn corner(prev: Point, cur: Point, next: Point) -> Option<Corner> {
    let (v1, v2) = (prev - cur, next - cur);
    let (l1, l2) = (v1.hypot(), v2.hypot());
    if l1 < 1e-9 || l2 < 1e-9 {
        return None;
    }
    let into = v1 / l1 + v2 / l2;
    let half = (v1 / l1).dot(v2 / l2).clamp(-1.0, 1.0).acos() / 2.0;
    (into.hypot() > 1e-9 && half > 1e-6).then(|| Corner { at: cur, into: into.normalize(), half, max_r: l1.min(l2) / 2.0 * half.tan() })
}

/// The corners of `n` that have handles, with their current radii.
fn corners(n: &Node) -> Vec<(Corner, f64)> {
    let (w, h) = (n.width, n.height);
    let r = n.corners.radii;
    match n.kind {
        NodeKind::Polygon { count } => top(&figcraft_geom::polygon_points(w, h, count), r[0]),
        NodeKind::Star { count, inner_ratio } => top(&figcraft_geom::star_points(w, h, count, inner_ratio), r[0]),
        _ if n.kind.has_independent_corners() => {
            let max_r = w.min(h) / 2.0;
            let quarter = std::f64::consts::FRAC_PI_4;
            [(0.0, 0.0, 1.0, 1.0), (w, 0.0, -1.0, 1.0), (w, h, -1.0, -1.0), (0.0, h, 1.0, -1.0)]
                .iter()
                .zip(r)
                .map(|(&(x, y, dx, dy), r)| (Corner { at: Point::new(x, y), into: Vec2::new(dx, dy).normalize(), half: quarter, max_r }, r))
                .collect()
        }
        _ => vec![],
    }
}

/// A polygon's or star's top point (its first) as a corner.
fn top(pts: &[Point], r: f64) -> Vec<(Corner, f64)> {
    match (pts.last(), pts.first(), pts.get(1)) {
        (Some(p), Some(c), Some(n)) => corner(*p, *c, *n).map(|c| (c, r)).into_iter().collect(),
        _ => vec![],
    }
}

/// The node whose handles show: the only selected layer, unlocked, visible and at least
/// [`MIN_PX`] on screen; with its local → page transform and the local inset.
fn target<'a>(doc: &'a Document, sel: &[NodeId], zoom: f64) -> Option<(NodeId, &'a Node, Affine, f64)> {
    let [id] = sel else { return None };
    let n = doc.node(*id)?;
    let t = doc.world_transform(*id);
    let s = t.determinant().abs().sqrt();
    let ok = !n.locked && n.visible && zoom.is_finite() && zoom > 0.0 && s.is_finite() && s > 1e-9;
    (ok && n.width.min(n.height) * s * zoom >= MIN_PX && n.kind.has_corner_radius() && !matches!(n.kind, NodeKind::Vector { .. }))
        .then(|| (*id, n, t, INSET_PX / (zoom * s)))
}

/// A star's ratio handle (local) for `ratio`.
fn ratio_point(n: &Node, count: u32, ratio: f64) -> Option<Point> {
    figcraft_geom::star_points(n.width, n.height, count, ratio).get(1).copied()
}

/// The shape handles of the selection at `zoom` (screen pixels per page unit): each handle and
/// where it is (page coordinates).
pub fn handles(doc: &Document, sel: &[NodeId], zoom: f64) -> Vec<(ShapeHandle, [f64; 2])> {
    let Some((_, n, t, inset)) = target(doc, sel, zoom) else { return vec![] };
    let pt = |p: Point| {
        let q = t * p;
        [q.x, q.y]
    };
    let mut out: Vec<(ShapeHandle, [f64; 2])> =
        corners(n).iter().enumerate().map(|(i, (c, r))| (ShapeHandle::Radius { corner: i }, pt(c.handle(*r, inset)))).collect();
    if let NodeKind::Star { count, inner_ratio } = n.kind
        && let Some(p) = ratio_point(n, count, inner_ratio)
    {
        out.push((ShapeHandle::Ratio, pt(p)));
    }
    out
}

/// The shape handle under page point `p` (the nearest), with its layer.
pub fn hit(doc: &Document, sel: &[NodeId], p: Point, zoom: f64) -> Option<(NodeId, ShapeHandle)> {
    let id = *sel.first()?;
    handles(doc, sel, zoom)
        .into_iter()
        .map(|(h, q)| (h, (Point::new(q[0], q[1]) - p).hypot()))
        .filter(|(_, d)| *d <= HIT_PX / zoom)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(h, _)| (id, h))
}

/// The `node.set` props for handle `h` of layer `id` dragged to page point `p` (`alt`: only that
/// corner of a rectangle or frame).
pub fn drag(doc: &Document, id: NodeId, h: ShapeHandle, p: Point, alt: bool) -> Option<Map<String, Value>> {
    let n = doc.node(id)?;
    let q = doc.world_transform(id).inverse() * p;
    if !(q.x.is_finite() && q.y.is_finite()) {
        return None;
    }
    match h {
        ShapeHandle::Radius { corner } => {
            let r = corners(n).get(corner)?.0.radius(q);
            let key = if alt && n.kind.has_independent_corners() {
                ["topLeftRadius", "topRightRadius", "bottomRightRadius", "bottomLeftRadius"].get(corner).copied()?
            } else {
                "cornerRadius"
            };
            Some(Map::from_iter([(key.to_string(), json!(r))]))
        }
        ShapeHandle::Ratio => {
            let NodeKind::Star { count, .. } = n.kind else { return None };
            // Along the inner point's ray, measured on the unit circle the star's box maps from.
            let (rx, ry) = (n.width / 2.0, n.height / 2.0);
            let dir = ratio_point(n, count, 1.0)?;
            let ray = Vec2::new(dir.x / rx - 1.0, dir.y / ry - 1.0);
            let k = Vec2::new(q.x / rx - 1.0, q.y / ry - 1.0).dot(ray) / ray.hypot2().max(1e-12);
            k.is_finite().then(|| Map::from_iter([("innerRadius".to_string(), json!(k.clamp(0.001, 1.0)))]))
        }
    }
}

/// The live readout while dragging `h` of `id`: the corner's radius, or the star's ratio in percent.
pub fn value(doc: &Document, id: NodeId, h: ShapeHandle) -> Option<f64> {
    let n = doc.node(id)?;
    let v = match (h, &n.kind) {
        (ShapeHandle::Ratio, NodeKind::Star { inner_ratio, .. }) => inner_ratio * 100.0,
        (ShapeHandle::Radius { corner }, _) => corners(n).into_iter().nth(corner).map(|(c, r)| r.clamp(0.0, c.max_r))?,
        _ => return None,
    };
    Some((v * 10.0).round() / 10.0)
}

/// Where handle `h` of `id` is now at `zoom` (the readout's anchor while dragging).
pub fn handle_at(doc: &Document, id: NodeId, h: ShapeHandle, zoom: f64) -> Option<[f64; 2]> {
    handles(doc, &[id], zoom).into_iter().find(|(x, _)| *x == h).map(|(_, at)| at)
}

/// The readout of a shape handle drag: the handle, its value and where it is.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ShapeHint {
    pub handle: ShapeHandle,
    pub value: f64,
    pub at: [f64; 2],
}
