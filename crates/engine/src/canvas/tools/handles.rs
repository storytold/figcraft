use super::{Affine, BTreeMap, Cursor, Document, Handle, NodeId, NodeKind, Point, ShapeHandle, ShapeHint, roots};

/// The frame whose name label (see [`ToolState::labels`]) is under `p` on `page`: the topmost
/// one drawn, if it can be selected (still there, on the page, visible and unlocked).
pub fn label_at(doc: &Document, page: NodeId, labels: &[(NodeId, [Point; 4])], p: Point) -> Option<NodeId> {
    labels
        .iter()
        .rev()
        .find(|(id, q)| in_quad(q, p) && doc.page_of(*id) == Some(page) && doc.node(*id).is_some_and(|n| n.visible && !n.locked))
        .map(|(id, _)| *id)
}

/// Whether `p` is inside the convex quadrilateral `q` (either winding; edges included).
pub(super) fn in_quad(q: &[Point; 4], p: Point) -> bool {
    let (mut left, mut right) = (false, false);
    for (a, b) in q.iter().zip(q.iter().cycle().skip(1)) {
        let side = (*b - *a).cross(p - *a);
        if !side.is_finite() {
            return false;
        }
        left |= side < 0.0;
        right |= side > 0.0;
    }
    // On one side of every edge (a point can't be on all four edges of a quad with an area).
    left != right
}

/// An anchor name (`tl`, `t`, `tr`, `l`, `c`, `r`, `bl`, `b`, `br`) as column and row (0 start,
/// 1 middle, 2 end).
pub fn anchor_of(name: &str) -> Option<(u8, u8)> {
    ANCHORS.iter().position(|a| a.eq_ignore_ascii_case(name.trim())).map(|i| ((i % 3) as u8, (i / 3) as u8))
}

/// The name of anchor `(column, row)`.
pub fn anchor_name(a: (u8, u8)) -> &'static str {
    ANCHORS.get(usize::from(a.1.min(2)) * 3 + usize::from(a.0.min(2))).copied().unwrap_or("tl")
}

/// The fill new text takes to be readable where it is drawn, in `parent`: white on a dark
/// background (None: the model's default, black). The background is the nearest layer from `parent`
/// out whose top visible fill is a mostly opaque solid colour, else the page (#362). For text made
/// in the app: by the Text tool, or by a command that names no fill while the interface is up.
pub(crate) fn readable_text_fills(doc: &Document, parent: NodeId) -> Option<Vec<figcraft_color::Paint>> {
    let dark = std::iter::once(parent).chain(doc.ancestors(parent)).filter_map(|id| doc.node(id)).find_map(|n| match &n.kind {
        NodeKind::Page { background, .. } => Some(background.is_dark()),
        _ => n.fills.iter().rev().find(|p| p.visible).and_then(|p| p.solid_color()).filter(|c| c.a >= 0.5).map(|c| c.is_dark()),
    });
    dark.unwrap_or(false).then(|| vec![figcraft_color::Paint::solid(figcraft_color::Color::WHITE)])
}

const ANCHORS: [&str; 9] = ["tl", "t", "tr", "l", "c", "r", "bl", "b", "br"];

const HANDLE_PX: f64 = 5.0;

const ROTATE_PX: f64 = 18.0;

/// Snap distance in screen pixels.
pub(crate) const SNAP_PX: f64 = 5.0;

/// Screen pixels the pointer travels after a press before a move or resize starts: a click or a
/// double-click made with an unsteady hand doesn't drag what it lands on.
pub(super) const DRAG_SLOP_PX: f64 = 3.0;

/// Screen pixels from a path within which the Text on a path tool takes it.
pub(super) const PATH_REACH_PX: f64 = 6.0;

/// The visible layers under `id` whose box, grown by `reach` (page units), holds `p`, front-most
/// first: what the Text on a path tool measures its distance to. Content a clipping frame or a
/// mask hides at `p` and the operands of boolean groups are left out.
pub(super) fn path_candidates(doc: &Document, id: NodeId, p: Point, reach: f64, depth: usize, out: &mut Vec<NodeId>) {
    let Some(n) = doc.node(id) else { return };
    if !n.visible || depth > 512 || matches!(n.kind, NodeKind::BooleanOp { .. }) {
        return;
    }
    if n.clip && n.kind.is_frame_like() && !n.local_rect().contains(doc.world_transform(id).inverse() * p) {
        return;
    }
    for c in n.paint_order().rev() {
        let before = out.len();
        path_candidates(doc, c, p, reach, depth + 1, out);
        if out.len() > before && figcraft_doc::geometry::masked_out(doc, n, c, p) {
            out.truncate(before);
        }
    }
    if n.children.is_empty() && !n.is_mask && doc.world_bounds(id).is_some_and(|b| b.inflate(reach, reach).contains(p)) {
        out.push(id);
    }
}

/// Corners of the selection's handle box in page coordinates (tl, tr, br, bl).
pub fn selection_box(doc: &Document, sel: &[NodeId]) -> Option<[Point; 4]> {
    let (f, w, h) = selection_frame(doc, sel)?;
    Some([f * Point::ZERO, f * Point::new(w, 0.0), f * Point::new(w, h), f * Point::new(0.0, h)])
}

/// The selection's handle box: the page transform of its top-left corner (axes along the box) and
/// its size. One layer: its own box; several sharing a rotation: a box turned with them;
/// otherwise the page-aligned bounds.
pub fn selection_frame(doc: &Document, sel: &[NodeId]) -> Option<(Affine, f64, f64)> {
    let ids = roots(doc, sel);
    if let [id] = ids.as_slice() {
        let n = doc.node(*id)?;
        return Some((doc.world_transform(*id), n.width, n.height));
    }
    let Some(angle) = shared_rotation(doc, &ids) else {
        let b = doc.bounds_of(&ids)?;
        return Some((Affine::translate((b.x0, b.y0)), b.width(), b.height()));
    };
    let rot = Affine::rotate(angle);
    let inv = rot.inverse();
    let b = ids
        .iter()
        .filter_map(|i| Some(figcraft_geom::transform_rect_bbox(inv * doc.world_transform(*i), doc.node(*i)?.local_rect())))
        .reduce(|a, b| a.union(b))?;
    Some((rot * Affine::translate((b.x0, b.y0)), b.width(), b.height()))
}

/// The rotation (radians) every layer in `ids` shares, unless it's a multiple of 90 degrees (the
/// page-aligned box is then the same box). Flipped layers count by their axis, not its direction.
fn shared_rotation(doc: &Document, ids: &[NodeId]) -> Option<f64> {
    use std::f64::consts::{FRAC_PI_2, PI};
    const EPS: f64 = 1e-4;
    let mut angles = ids.iter().map(|i| {
        let c = doc.world_transform(*i).as_coeffs();
        c[1].atan2(c[0])
    });
    let a = angles.next()?;
    let same = |b: f64| {
        let d = (b - a).rem_euclid(PI);
        d.min(PI - d) < EPS
    };
    let axis_aligned = (a - (a / FRAC_PI_2).round() * FRAC_PI_2).abs() < EPS;
    (a.is_finite() && !axis_aligned && angles.all(same)).then_some(a)
}

/// Whether layer `id` is locked or inside a locked layer: it can be selected in the layers panel
/// but not moved or resized on the canvas.
pub fn locked_in(doc: &Document, id: NodeId) -> bool {
    std::iter::once(id).chain(doc.ancestors(id)).any(|a| doc.node(a).is_some_and(|n| n.locked))
}

/// Whether layer `t` paints below layer `s`: it holds `s`, or it is behind `s` in the stack of the
/// nearest layer holding both.
pub(super) fn paints_below(doc: &Document, t: NodeId, s: NodeId) -> bool {
    let chain = |id: NodeId| {
        let mut c: Vec<NodeId> = std::iter::once(id).chain(doc.ancestors(id)).collect();
        c.reverse();
        c
    };
    let (ct, cs) = (chain(t), chain(s));
    let shared = ct.iter().zip(&cs).take_while(|(a, b)| a == b).count();
    match (ct.get(shared), cs.get(shared)) {
        (None, Some(_)) => true,
        (Some(a), Some(b)) => shared.checked_sub(1).and_then(|i| ct.get(i)).and_then(|p| doc.node(*p)).is_some_and(|p| {
            let order: Vec<NodeId> = p.paint_order().collect();
            order.iter().position(|c| c == a) < order.iter().position(|c| c == b)
        }),
        _ => false,
    }
}

/// Which handle (if any) is under `p`. A selection with a locked layer has none.
pub fn handle_at(doc: &Document, sel: &[NodeId], p: Point, zoom: f64) -> Option<Handle> {
    if sel.is_empty() || sel.iter().any(|i| locked_in(doc, *i)) {
        return None;
    }
    // A connector's handles are its ends (see `board::end_handle_at`): no box to resize or turn.
    if crate::board::selected_connector_ends(doc, sel).is_some() {
        return None;
    }
    let corners = selection_box(doc, sel)?;
    // Lines (zero height) only get end handles.
    let is_line = sel.len() == 1 && sel.first().and_then(|i| doc.node(*i)).is_some_and(|n| matches!(n.kind, NodeKind::Line));
    handle_at_corners(&corners, p, zoom, is_line)
}

/// Half the side of the square the canvas draws on each corner of the handle box (screen pixels,
/// upright whatever the box's rotation).
const CORNER_SQUARE_PX: f64 = 4.0;

/// Whether `p` is on a corner square of the selection's handle box as drawn: there the corner
/// resizes, although a corner-radius handle 10 px in reaches that far too.
pub(super) fn on_corner_square(doc: &Document, sel: &[NodeId], p: Point, zoom: f64) -> bool {
    let reach = CORNER_SQUARE_PX / zoom.max(1e-6);
    matches!(handle_at(doc, sel, p, zoom), Some(Handle::Corner(_)))
        && selection_box(doc, sel).is_some_and(|c| c.iter().any(|c| (c.x - p.x).abs() <= reach && (c.y - p.y).abs() <= reach))
}

/// A box whose sides are both shorter than this on screen (pixels) would be covered by its
/// handles: the canvas draws just its outline (the handles still answer just outside it).
pub const SMALL_BOX_PX: f64 = 16.0;

/// The stroke the Pencil draws: its colour and weight (the canvas previews it the same).
pub const PENCIL_STROKE: (figcraft_color::Color, f64) = (figcraft_color::Color::BLACK, 2.0);

/// The shorter and longer sides, in screen pixels, of the box with `corners` (tl, tr, br, bl).
pub fn box_sides_px(corners: &[Point; 4], zoom: f64) -> (f64, f64) {
    let [a, b, _, d] = corners;
    let (w, h) = ((*b - *a).hypot() * zoom, (*d - *a).hypot() * zoom);
    (w.min(h), w.max(h))
}

/// Which handle of the box with `corners` (tl, tr, br, bl in page coordinates) is under `p`;
/// `is_line` keeps only the two end handles.
pub fn handle_at_corners(corners: &[Point; 4], p: Point, zoom: f64, is_line: bool) -> Option<Handle> {
    let tol = HANDLE_PX / zoom.max(1e-6);
    // Inside the box, handles reach in at most a quarter of its shorter side, so that a press in
    // the middle of a small layer moves it; outside it they reach as far as ever.
    let inner = if !is_line && in_quad(corners, p) { box_sides_px(corners, zoom).0 / 4.0 / zoom.max(1e-6) } else { f64::INFINITY };
    let (corner_reach, edge_reach) = ((tol * 1.6).min(inner), tol.min(inner));
    // The nearest corner, then the nearest edge, in reach (a small box's reaches overlap).
    let nearest = |hits: &mut dyn Iterator<Item = (f64, Handle)>| hits.min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, h)| h);
    let corner = nearest(
        &mut corners
            .iter()
            .enumerate()
            .filter(|(i, _)| !(is_line && *i >= 2))
            .map(|(i, c)| ((*c - p).hypot(), Handle::Corner(i as u8)))
            .filter(|(d, _)| *d <= corner_reach),
    );
    if corner.is_some() {
        return corner;
    }
    if !is_line {
        use figcraft_geom::kurbo::ParamCurveNearest;
        let edge = nearest(&mut (0..4).filter_map(|i| {
            let (a, b) = (corners.get(i)?, corners.get((i + 1) % 4)?);
            let near = figcraft_geom::kurbo::Line::new(*a, *b).nearest(p, 0.001);
            let d = near.distance_sq.sqrt();
            (d <= edge_reach && near.t > 0.05 && near.t < 0.95).then_some((d, Handle::Edge(i as u8)))
        }));
        if edge.is_some() {
            return edge;
        }
    }
    // Rotation zones: just outside each corner.
    let center = Point::new(corners.iter().map(|c| c.x).sum::<f64>() / 4.0, corners.iter().map(|c| c.y).sum::<f64>() / 4.0);
    let rot = ROTATE_PX / zoom.max(1e-6);
    for (i, c) in corners.iter().enumerate() {
        let d = (*c - p).hypot();
        let outward = (p - *c).dot(*c - center) > 0.0;
        if d <= rot && d > tol && outward {
            return Some(Handle::Rotate(i as u8));
        }
    }
    None
}

pub(super) fn resize_cursor(doc: &Document, sel: &[NodeId], h: Handle) -> Cursor {
    selection_box(doc, sel).map_or(Cursor::Default, |c| handle_cursor(&c, h))
}

/// The cursor over handle `h` of the box with corners `c`.
pub(crate) fn handle_cursor(c: &[Point; 4], h: Handle) -> Cursor {
    let angle = |a: Point, b: Point| (b - a).atan2().to_degrees();
    match h {
        Handle::Corner(i) => {
            let a = c.get(i as usize).copied().unwrap_or_default();
            let b = c.get((i as usize + 2) % 4).copied().unwrap_or_default();
            // Along the box's own axes at 45°, not the diagonal, so wide or tall boxes still get a
            // diagonal cursor. A box with no width or height falls back to the diagonal.
            let unit = |v: figcraft_geom::kurbo::Vec2| {
                let len = v.hypot();
                (len.is_finite() && len > 1e-9).then(|| v / len)
            };
            let next = c.get((i as usize + 1) % 4).copied().unwrap_or_default();
            let prev = c.get((i as usize + 3) % 4).copied().unwrap_or_default();
            match (unit(next - a), unit(prev - a)) {
                (Some(u), Some(v)) => Cursor::Resize((u + v).atan2().to_degrees()),
                _ => Cursor::Resize(angle(a, b)),
            }
        }
        Handle::Edge(i) => {
            let a = c.get(i as usize).copied().unwrap_or_default();
            let b = c.get((i as usize + 1) % 4).copied().unwrap_or_default();
            Cursor::Resize(angle(a, b) + 90.0)
        }
        Handle::Rotate(i) => {
            let centre = Point::new(c.iter().map(|p| p.x).sum::<f64>() / 4.0, c.iter().map(|p| p.y).sum::<f64>() / 4.0);
            Cursor::Rotate(angle(centre, c.get(i as usize).copied().unwrap_or_default()))
        }
    }
}

/// The box (x0, y0, x1, y1) a `w0`×`h0` handle box becomes when `handle` is dragged to `q`, all in
/// the box's own axes: `keep` constrains proportions, `from_center` resizes about the centre (the
/// opposite side mirrors the dragged one). Edges that cross over come back reversed (x1 < x0 or
/// y1 < y0: a flip).
pub(crate) fn resized_box(handle: Handle, q: Point, w0: f64, h0: f64, keep: bool, from_center: bool) -> (f64, f64, f64, f64) {
    let (mut x0, mut y0, mut x1, mut y1) = (0.0, 0.0, w0, h0);
    // The dragged sides: x0 or x1 (`Some(false)` / `Some(true)`), y0 or y1.
    let (sx, sy) = match handle {
        Handle::Corner(0) => (Some(false), Some(false)),
        Handle::Corner(1) => (Some(true), Some(false)),
        Handle::Corner(2) => (Some(true), Some(true)),
        Handle::Corner(_) => (Some(false), Some(true)),
        Handle::Edge(0) => (None, Some(false)),
        Handle::Edge(1) => (Some(true), None),
        Handle::Edge(2) => (None, Some(true)),
        Handle::Edge(_) => (Some(false), None),
        Handle::Rotate(_) => (None, None),
    };
    match sx {
        Some(false) => (x0, x1) = (q.x, if from_center { w0 - q.x } else { w0 }),
        Some(true) => (x0, x1) = (if from_center { w0 - q.x } else { 0.0 }, q.x),
        None => {}
    }
    match sy {
        Some(false) => (y0, y1) = (q.y, if from_center { h0 - q.y } else { h0 }),
        Some(true) => (y0, y1) = (if from_center { h0 - q.y } else { 0.0 }, q.y),
        None => {}
    }
    if keep && w0 > 1e-9 && h0 > 1e-9 && !matches!(handle, Handle::Rotate(_)) {
        // One factor for both axes: the dragged axis's (a corner: the one that moved the most),
        // about the point that stays put (the opposite handle, or the centre).
        let (kx, ky) = ((x1 - x0) / w0, (y1 - y0) / h0);
        let k = match (sx, sy) {
            (None, _) => ky,
            (_, None) => kx,
            _ if kx.abs() > ky.abs() => kx,
            _ => ky,
        };
        let pivot = if from_center { Point::new(w0 / 2.0, h0 / 2.0) } else { handle_point(opposite(handle), w0, h0) };
        (x0, x1) = (pivot.x * (1.0 - k), pivot.x + (w0 - pivot.x) * k);
        (y0, y1) = (pivot.y * (1.0 - k), pivot.y + (h0 - pivot.y) * k);
    }
    (x0, y0, x1, y1)
}

/// A handle's point in its box's axes (`w0`×`h0`): a corner or an edge's middle.
fn handle_point(h: Handle, w0: f64, h0: f64) -> Point {
    match h {
        Handle::Corner(0) | Handle::Rotate(0) => Point::new(0.0, 0.0),
        Handle::Corner(1) | Handle::Rotate(1) => Point::new(w0, 0.0),
        Handle::Corner(2) | Handle::Rotate(2) => Point::new(w0, h0),
        Handle::Corner(_) | Handle::Rotate(_) => Point::new(0.0, h0),
        Handle::Edge(0) => Point::new(w0 / 2.0, 0.0),
        Handle::Edge(1) => Point::new(w0, h0 / 2.0),
        Handle::Edge(2) => Point::new(w0 / 2.0, h0),
        Handle::Edge(_) => Point::new(0.0, h0 / 2.0),
    }
}

/// Anchor `(column, row)` of a `w`×`h` box, in its axes.
fn anchor_in_box(anchor: (u8, u8), w: f64, h: f64) -> Point {
    Point::new(w * f64::from(anchor.0.min(2)) / 2.0, h * f64::from(anchor.1.min(2)) / 2.0)
}

/// The Scale tool's anchor point on the selection's handle box, in page coordinates.
pub fn scale_anchor_point(doc: &Document, sel: &[NodeId], anchor: (u8, u8)) -> Option<Point> {
    let (f, w, h) = selection_frame(doc, sel)?;
    Some(f * anchor_in_box(anchor, w, h))
}

/// The handle across the box from `h`.
fn opposite(h: Handle) -> Handle {
    match h {
        Handle::Corner(i) => Handle::Corner((i + 2) % 4),
        Handle::Edge(i) => Handle::Edge((i + 2) % 4),
        Handle::Rotate(i) => Handle::Rotate((i + 2) % 4),
    }
}

/// The Scale tool's factor and pivot (in the box's axes) for dragging `handle` of a `w0`×`h0` box
/// to `q`. The pivot is the anchor point (`anchor`: column, row), or the opposite handle when the
/// anchor lies on the side being dragged, or the centre (`from_center`). Corners follow the axis
/// that moved the most; a negative factor flips. None when the box has no size to scale by.
pub(crate) fn scale_factor(handle: Handle, q: Point, w0: f64, h0: f64, anchor: (u8, u8), from_center: bool) -> Option<(f64, Point)> {
    const EPS: f64 = 1e-9;
    if !(q.x.is_finite() && q.y.is_finite() && w0.is_finite() && h0.is_finite()) || matches!(handle, Handle::Rotate(_)) {
        return None;
    }
    let h = handle_point(handle, w0, h0);
    let anchor_pt = anchor_in_box(anchor, w0, h0);
    // The axes the handle moves along.
    let (ax_x, ax_y) = match handle {
        Handle::Edge(0) | Handle::Edge(2) => (false, true),
        Handle::Edge(_) => (true, false),
        _ => (true, true),
    };
    let on_side = (ax_x && (anchor_pt.x - h.x).abs() < EPS) || (ax_y && (anchor_pt.y - h.y).abs() < EPS);
    let pivot = if from_center {
        Point::new(w0 / 2.0, h0 / 2.0)
    } else if on_side {
        handle_point(opposite(handle), w0, h0)
    } else {
        anchor_pt
    };
    let ratio = |on: bool, to: f64, from: f64, p: f64| (on && (from - p).abs() > EPS).then(|| (to - p) / (from - p));
    let kx = ratio(ax_x, q.x, h.x, pivot.x);
    let ky = ratio(ax_y, q.y, h.y, pivot.y);
    let k = match (kx, ky) {
        (Some(a), Some(b)) => {
            if a.abs() > b.abs() {
                a
            } else {
                b
            }
        }
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => return None,
    };
    if !k.is_finite() {
        return None;
    }
    // Never collapse to nothing (or blow up past a usable size); 0 counts as positive.
    Some((k.signum() * k.abs().clamp(0.01, 1000.0), pivot))
}

/// The readout of a shape handle drag on `id`.
pub(super) fn shape_hint(doc: &Document, id: NodeId, handle: ShapeHandle, zoom: f64) -> Option<ShapeHint> {
    Some(ShapeHint { handle, value: crate::shape_handles::value(doc, id, handle)?, at: crate::shape_handles::handle_at(doc, id, handle, zoom)? })
}

#[allow(dead_code)]
fn unused(_: BTreeMap<u8, u8>) {}

/// An end point of a selected open vector within `tol` of page point `p`: (layer, point).
pub(super) fn selected_end_at(doc: &Document, sel: &[NodeId], p: Point, tol: f64) -> Option<(NodeId, usize)> {
    sel.iter().find_map(|id| {
        let net = crate::vectoredit::network(doc, *id)?;
        let w = doc.world_transform(*id);
        let near = |v: &usize| net.vertices.get(*v).is_some_and(|x| (w * x.point() - p).hypot() <= tol);
        (0..net.vertices.len()).filter(near).find(|v| net.incident(*v).len() == 1).map(|v| (*id, v))
    })
}
