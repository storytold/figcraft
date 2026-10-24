//! The transform box of vector edit mode. With two or more points selected (Move tool) a box in the
//! vector's own axes wraps them (in the page's axes when they are in several layers): its corners
//! and edges scale them (Shift keeps proportions, Alt scales from the centre), dragging just
//! outside a corner rotates them about its centre (Shift: 15° steps) and dragging inside it moves
//! them; each drag is one undo step for every layer (the gestures live in `pointer.rs`). Commands make the same edits for the Design panel, the control channel and MCP,
//! and while points are selected the flip and 90°/180° rotation commands act on them.

use figcraft_doc::{Document, NodeId};
use figcraft_geom::{Affine, Point};
use serde_json::{Value, json};

use super::{VHit, VTool, VectorEdit, default_points, edit, editing, indices, network, ops, selected_points, selection_position};
use crate::cmd::CommandSpec;
use crate::params::*;
use crate::tools::{Handle, handle_at_corners};
use crate::{Result, Session, bad, spec};

/// Box sizes at or below this have no extent on that axis (nothing to scale).
const EPS: f64 = 1e-9;

/// The selected points' box: the page transform of its top-left corner (axes along the vector's
/// own), its size in those axes and the points in it: the active layer's, and other edited
/// layers' in `others`.
#[derive(Clone, Debug, PartialEq)]
pub struct PointBox {
    pub frame: Affine,
    pub width: f64,
    pub height: f64,
    pub points: Vec<usize>,
    pub others: Vec<(NodeId, Vec<usize>)>,
}

impl PointBox {
    /// How many points the box wraps, in every layer.
    pub fn point_count(&self) -> usize {
        self.points.len() + self.others.iter().map(|(_, v)| v.len()).sum::<usize>()
    }

    /// Corners in page coordinates (tl, tr, br, bl).
    pub fn corners(&self) -> [Point; 4] {
        let (f, w, h) = (self.frame, self.width, self.height);
        [f * Point::ZERO, f * Point::new(w, 0.0), f * Point::new(w, h), f * Point::new(0.0, h)]
    }

    pub fn center(&self) -> Point {
        self.frame * Point::new(self.width / 2.0, self.height / 2.0)
    }

    /// The page transform mapping this box onto (x0, y0)–(x1, y1) in its own axes (a reversed
    /// pair flips). An axis without extent keeps its points where they are.
    pub fn onto(&self, x0: f64, y0: f64, x1: f64, y1: f64) -> Affine {
        let (x0, sx) = if self.width > EPS { (x0, (x1 - x0) / self.width) } else { (0.0, 1.0) };
        let (y0, sy) = if self.height > EPS { (y0, (y1 - y0) / self.height) } else { (0.0, 1.0) };
        self.frame * Affine::new([sx, 0.0, 0.0, sy, x0, y0]) * self.frame.inverse()
    }
}

/// The box of the selected points (and selected segments' ends); one point makes an empty box.
pub fn point_box(doc: &Document, ve: &VectorEdit) -> Option<PointBox> {
    let net = network(doc, ve.id)?;
    let points = selected_points(net, ve);
    let (x0, y0, x1, y1) = points
        .iter()
        .filter_map(|i| net.vertices.get(*i))
        .fold((f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY), |(a, b, c, d), v| {
            (a.min(v.x), b.min(v.y), c.max(v.x), d.max(v.y))
        });
    let (width, height) = (x1 - x0, y1 - y0);
    if !(x0.is_finite() && y0.is_finite() && width.is_finite() && height.is_finite()) {
        return None;
    }
    Some(PointBox { frame: doc.world_transform(ve.id) * Affine::translate((x0, y0)), width, height, points, others: vec![] })
}

/// The box of the selected points of every edited layer, in the page's axes (the layers' own may
/// differ).
fn layers_box(doc: &Document, ve: &VectorEdit) -> Option<PointBox> {
    let points = selected_points(network(doc, ve.id)?, ve);
    let others = Session::others_moving(doc, ve);
    let page = |id: NodeId, verts: &[usize]| -> Vec<Point> {
        let w = doc.world_transform(id);
        network(doc, id).map(|n| verts.iter().filter_map(|v| n.vertices.get(*v)).map(|v| w * v.point()).collect()).unwrap_or_default()
    };
    let all = page(ve.id, &points).into_iter().chain(others.iter().flat_map(|(id, v)| page(*id, v)));
    let (x0, y0, x1, y1) = all.fold((f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY), |(a, b, c, d), p| {
        (a.min(p.x), b.min(p.y), c.max(p.x), d.max(p.y))
    });
    let (width, height) = (x1 - x0, y1 - y0);
    if !(x0.is_finite() && y0.is_finite() && width.is_finite() && height.is_finite()) {
        return None;
    }
    Some(PointBox { frame: Affine::translate((x0, y0)), width, height, points, others })
}

/// The box drawn and dragged on the canvas: the Move tool, no open pen path, and two or more
/// points that aren't all in one place, in any of the edited layers.
pub fn shown_box(doc: &Document, ve: &VectorEdit) -> Option<PointBox> {
    if ve.tool != VTool::Move || ve.anchor.is_some() {
        return None;
    }
    let b = if ve.others_selected() { layers_box(doc, ve) } else { point_box(doc, ve) };
    b.filter(|b| b.point_count() >= 2 && (b.width > EPS || b.height > EPS))
}

/// What the transform box does under the pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoxHit {
    /// A corner or edge (scale) or a rotation zone.
    Handle(Handle),
    /// Inside the box, clear of the network: drag to move the points.
    Inside,
}

/// The box's part under page point `p` when it wins over `h`, the network's part there: Bezier
/// handles come first, then the box's corners, points, the box's edges, segments, the rotation
/// zones, and the inside of the box last. Edges along an axis without extent are left out.
pub fn box_hit(doc: &Document, ve: &VectorEdit, h: Option<VHit>, p: Point, zoom: f64) -> Option<BoxHit> {
    let b = shown_box(doc, ve)?;
    match handle_at_corners(&b.corners(), p, zoom, false) {
        Some(handle) => {
            let wins = match (handle, h) {
                (_, Some(VHit::Handle { .. })) => false,
                (Handle::Corner(_), _) => true,
                (Handle::Edge(e), None | Some(VHit::Segment { .. })) => {
                    if e % 2 == 0 {
                        b.height > EPS
                    } else {
                        b.width > EPS
                    }
                }
                (Handle::Rotate(_), None) => true,
                _ => false,
            };
            wins.then_some(BoxHit::Handle(handle))
        }
        None => {
            let q = b.frame.inverse() * p;
            (h.is_none() && (0.0..=b.width).contains(&q.x) && (0.0..=b.height).contains(&q.y)).then_some(BoxHit::Inside)
        }
    }
}

/// Map points `verts` of vector `id` (and the handles at them) through page transform `m`, then
/// re-fit the vector.
pub fn transform_points(doc: &mut Document, id: NodeId, verts: &[usize], m: Affine) -> Result<()> {
    let w = doc.world_transform(id);
    let local = w.inverse() * m * w;
    edit(doc, id, |net| ops::transform_vertices(net, verts, local))
}

/// Map the points of box `b` (`id`: the active layer) in every layer through page transform `m`.
pub fn transform_box(doc: &mut Document, id: NodeId, b: &PointBox, m: Affine) -> Result<()> {
    transform_points(doc, id, &b.points, m)?;
    for (o, verts) in &b.others {
        transform_points(doc, *o, verts, m)?;
    }
    Ok(())
}

/// Transform the selected points by `f(box)` (page space) when editing with points selected and
/// the command names no layers (`p` has no `ids` or `id`); `None` otherwise (it then acts on
/// layers).
pub fn transform_in_edit_mode(s: &mut Session, p: &Value, f: impl FnOnce(&PointBox) -> Affine) -> Option<Result<Value>> {
    if get(p, "ids").is_some() || get(p, "id").is_some() {
        return None;
    }
    let ve = editing(s).ok()?;
    let b = point_box(s.doc().ok()?, &ve).filter(|b| !b.points.is_empty())?;
    let m = f(&b);
    Some(s.doc_mut().and_then(|doc| transform_points(doc, ve.id, &b.points, m)).map(|()| json!({"ok": true})))
}

/// The selected points' box, or an error naming `cmd`.
fn selected_box(s: &Session, cmd: &str) -> Result<(VectorEdit, PointBox)> {
    let ve = editing(s)?;
    let b = point_box(s.doc()?, &ve).filter(|b| !b.points.is_empty()).ok_or_else(|| bad(cmd, "select points"))?;
    Ok((ve, b))
}

/// `[a, b, c, d, e, f]` from params.
fn matrix(cmd: &str, p: &Value) -> Result<Affine> {
    let c: Vec<f64> = match get(p, "matrix") {
        Some(Value::Array(a)) => a.iter().filter_map(f64_of).collect(),
        _ => vec![],
    };
    match c.as_slice() {
        [a, b, c, d, e, f] => Ok(Affine::new([*a, *b, *c, *d, *e, *f])),
        _ => Err(bad(cmd, "`matrix` must be [a, b, c, d, e, f]")),
    }
}

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "vector.edit.bounds",
            "Selected points' bounds",
            "{} → {x, y (top-left in the parent's coordinates), width, height (in the vector's axes), corners (page), points}",
            false,
            |s, _| {
                let (ve, b) = selected_box(s, "vector.edit.bounds")?;
                let pos = selection_position(s.doc()?, &ve);
                let corners: Vec<[f64; 2]> = b.corners().iter().map(|c| [c.x, c.y]).collect();
                Ok(
                    json!({"x": pos.map(|p| p.x), "y": pos.map(|p| p.y), "width": b.width, "height": b.height, "corners": corners, "points": b.points}),
                )
            }
        ),
        spec!(
            "vector.edit.resize",
            "Resize points",
            "{width?, height?, center? (about the centre; default keeps the top-left)} — scales the selected points' box in the vector's axes",
            true,
            |s, p| {
                let (ve, b) = selected_box(s, "vector.edit.resize")?;
                let w = num(p, "width").map_or(b.width, |v| v.clamp(0.0, 1e7));
                let h = num(p, "height").map_or(b.height, |v| v.clamp(0.0, 1e7));
                let (x0, y0) = if boolean(p, "center").unwrap_or(false) { ((b.width - w) / 2.0, (b.height - h) / 2.0) } else { (0.0, 0.0) };
                transform_points(s.doc_mut()?, ve.id, &b.points, b.onto(x0, y0, x0 + w, y0 + h))?;
                Ok(json!({"ok": true}))
            }
        ),
        spec!(
            "vector.edit.rotate",
            "Rotate points",
            "{angle (degrees, counter-clockwise), cx?, cy? (page pivot; default the box centre)}",
            true,
            |s, p| {
                let (ve, b) = selected_box(s, "vector.edit.rotate")?;
                let angle = req_num("vector.edit.rotate", p, "angle")?;
                let c = b.center();
                let pivot = Point::new(num_or(p, "cx", c.x), num_or(p, "cy", c.y));
                transform_points(s.doc_mut()?, ve.id, &b.points, figcraft_geom::rotate_about(Affine::IDENTITY, pivot, angle))?;
                Ok(json!({"ok": true}))
            }
        ),
        spec!(
            "vector.edit.transform",
            "Transform points",
            "{matrix: [a, b, c, d, e, f] (page space), vertices? (default the selected points)}",
            true,
            |s, p| {
                let ve = editing(s)?;
                let m = matrix("vector.edit.transform", p)?;
                let verts = indices(p, "vertices").unwrap_or_else(|| default_points(s, &ve));
                transform_points(s.doc_mut()?, ve.id, &verts, m)?;
                Ok(json!({"ok": true}))
            }
        ),
    ]
}
