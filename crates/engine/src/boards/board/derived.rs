use figcraft_doc::RouteEnd;

use super::edit::set_font_size;
use super::items::{owner_of_text, shape_padding, sticky_font_size, text_of};
use super::{
    Affine, BTreeSet, BezPath, BoardItem, Connector, Document, End, NodeId, NodeKind, PathEl, Point, Rect, Routing, Shape, VectorNetwork, is_board,
};

// ---------------------------------------------------------------------------------------------
// Derived data (the normalize pass).

/// Before text is measured: a sticky's note takes the size its length calls for, and a resized
/// shape keeps its text inside its outline.
pub(crate) fn settle_text(doc: &mut Document, before: &Document, touched: &BTreeSet<NodeId>) {
    if !is_board(doc) {
        return;
    }
    for &id in touched {
        let Some(n) = doc.node(id) else { continue };
        if let Some(owner) =
            owner_of_text(doc, id).or(Some(id)).filter(|o| doc.node(*o).is_some_and(|k| matches!(k.board, Some(BoardItem::Sticky { .. }))))
            && let Some(t) = text_of(doc, owner)
            && let Some(NodeKind::Text(td)) = doc.node(t).map(|k| &k.kind)
        {
            let size = sticky_font_size(&td.characters);
            let off = td.style.font_size != size || td.runs.iter().any(|r| r.style.font_size != size);
            if off && let Some(NodeKind::Text(td)) = doc.node_mut(t).map(|k| &mut k.kind) {
                set_font_size(td, size);
            }
            continue;
        }
        if let Some(BoardItem::Shape { shape }) = n.board
            && before.node(id).is_none_or(|b| (b.width, b.height) != (n.width, n.height))
        {
            let pad = shape_padding(shape, n.width, n.height);
            if n.layout.as_ref().is_some_and(|l| l.padding != pad)
                && let Some(l) = doc.node_mut(id).and_then(|k| k.layout.as_mut())
            {
                l.padding = pad;
            }
        }
    }
}

/// After layout: connectors run between their ends again (an end whose layer is gone stays where
/// it was), one moved on its own takes its free ends along, and labels sit at their middles. Only
/// the connectors and labels an edit can have moved are looked at: touched themselves, or with a
/// touched (or gone) end, connector or layer around them.
pub(crate) fn settle_routes(doc: &mut Document, before: &Document, touched: &BTreeSet<NodeId>) {
    if !is_board(doc) || touched.is_empty() {
        return;
    }
    let hit = |doc: &Document, id: NodeId| touched.contains(&id) || doc.ancestors(id).iter().any(|a| touched.contains(a));
    let (mut connectors, mut labels) = (Vec::new(), Vec::new());
    for n in doc.nodes() {
        match &n.board {
            Some(BoardItem::Connector(c)) => {
                let ends = [c.from.node(), c.to.node()].into_iter().flatten();
                if hit(doc, n.id) || ends.into_iter().any(|e| !doc.contains(e) || hit(doc, e)) {
                    connectors.push((n.id, c.clone()));
                }
            }
            Some(BoardItem::Label { connector }) => labels.push((n.id, *connector)),
            _ => {}
        }
    }
    let rerouted: BTreeSet<NodeId> = connectors.iter().map(|(id, _)| *id).collect();
    for (id, mut c) in connectors {
        let changed = detach_gone_ends(doc, id, &mut c) | carry_free_ends(doc, before, touched, id, &mut c);
        if changed && let Some(n) = doc.node_mut(id) {
            n.board = Some(BoardItem::Connector(c.clone()));
        }
        if let Some(pts) = route(doc, &c) {
            place_route(doc, id, &pts, c.routing);
        }
    }
    for (label, connector) in labels {
        if rerouted.contains(&connector) || touched.contains(&connector) || hit(doc, label) {
            place_label(doc, label, connector);
        }
    }
}

/// After layout: a stamp moves with the object it sits on; one moved by itself (a drag, a nudge)
/// sits on whatever is under its centre now; one whose object is gone, or on another page, sits on
/// nothing and stays where it is.
pub(crate) fn settle_stamps(doc: &mut Document, before: &Document, touched: &BTreeSet<NodeId>) {
    if !is_board(doc) || touched.is_empty() {
        return;
    }
    let hit = |doc: &Document, id: NodeId| touched.contains(&id) || doc.ancestors(id).iter().any(|a| touched.contains(a));
    let stamps: Vec<(NodeId, Option<NodeId>)> = doc
        .nodes()
        .filter_map(|n| match &n.board {
            Some(BoardItem::Stamp { on, .. }) => Some((n.id, *on)),
            _ => None,
        })
        .collect();
    for (id, on) in stamps {
        let moved_itself = touched.contains(&id) && before.node(id).zip(doc.node(id)).is_some_and(|(b, n)| b.transform != n.transform);
        let gone = on.is_some_and(|t| !doc.contains(t) || doc.page_of(t) != doc.page_of(id));
        let now = if moved_itself {
            match (doc.page_of(id), doc.world_bounds(id)) {
                (Some(page), Some(b)) => stamp_target(doc, page, b.center(), Some(id)),
                _ => None,
            }
        } else if gone {
            None
        } else {
            if let Some(t) = on.filter(|t| hit(doc, *t)) {
                carry(doc, before, id, t);
            }
            on
        };
        if now != on
            && let Some(BoardItem::Stamp { on: slot, .. }) = doc.node_mut(id).and_then(|n| n.board.as_mut())
        {
            *slot = now;
        }
    }
}

/// The object a stamp at `p` on `page` sits on: the topmost layer of the page whose box holds `p`,
/// other than stamps, connectors and their labels, ink and `except`.
pub(crate) fn stamp_target(doc: &Document, page: NodeId, p: Point, except: Option<NodeId>) -> Option<NodeId> {
    let top = doc.node(page)?.children.clone();
    top.into_iter().rev().filter(|c| Some(*c) != except).find(|c| {
        let takes = doc.node(*c).is_some_and(|n| {
            n.visible && !matches!(n.board, Some(BoardItem::Stamp { .. } | BoardItem::Connector(_) | BoardItem::Label { .. } | BoardItem::Ink))
        });
        takes && doc.world_bounds(*c).is_some_and(|b| b.contains(p))
    })
}

/// Stamp `id` follows object `on` from where it was in `before`: its centre goes where the object's
/// move takes it (the stamp itself neither turns nor scales).
fn carry(doc: &mut Document, before: &Document, id: NodeId, on: NodeId) {
    // One inside its object (dropped into it before objects stopped taking layers in) already
    // moves with it.
    if before.node(on).is_none() || doc.is_ancestor(on, id) {
        return;
    }
    let (was, is) = (before.world_transform(on), doc.world_transform(on));
    let Some(c) = doc.world_bounds(id).map(|b| b.center()) else { return };
    let d = is * (was.inverse() * c) - c;
    if !(d.x.is_finite() && d.y.is_finite()) || d.hypot() <= 1e-9 {
        return;
    }
    let parent = doc.node(id).and_then(|n| n.parent).map_or(Affine::IDENTITY, |p| doc.world_transform(p));
    if let Some(n) = doc.node_mut(id) {
        n.transform = parent.inverse() * Affine::translate(d) * parent * n.transform;
    }
}

/// The ends of connector `id` (as `c`) whose layer is gone or on another page stay where they are
/// drawn, as points. Whether any did.
fn detach_gone_ends(doc: &Document, id: NodeId, c: &mut Connector) -> bool {
    let (page, ends) = (doc.page_of(id), ends_now(doc, id));
    let mut detached = false;
    for (end, at) in [(&mut c.from, ends.map(|e| e.0)), (&mut c.to, ends.map(|e| e.1))] {
        if let (End::Node { node, .. }, Some(p)) = (*end, at)
            && (!doc.contains(node) || doc.page_of(node) != page)
        {
            *end = End::Point { x: p.x, y: p.y };
            detached = true;
        }
    }
    detached
}

/// Connector `id` (as `c`) moved by itself (a drag, a nudge: its place changed but not what it
/// joins): its free ends move with it. Whether they did.
fn carry_free_ends(doc: &Document, before: &Document, touched: &BTreeSet<NodeId>, id: NodeId, c: &mut Connector) -> bool {
    let (Some(b), Some(n)) = (before.node(id), doc.node(id)) else { return false };
    if !touched.contains(&id) || b.transform == n.transform || b.board != n.board {
        return false;
    }
    let Some(expected) = route(doc, c).and_then(|pts| finite_box(&route_path(&pts, c.routing))).map(|b| b.origin()) else { return false };
    let d = (doc.world_transform(id) * Point::ZERO) - expected;
    if d.hypot() <= 1e-9 {
        return false;
    }
    let mut moved = false;
    for end in [&mut c.from, &mut c.to] {
        if let End::Point { x, y } = end {
            (*x, *y) = (*x + d.x, *y + d.y);
            moved = true;
        }
    }
    moved
}

/// Label `label` at the middle of connector `connector`; gone with it when it is gone.
fn place_label(doc: &mut Document, label: NodeId, connector: NodeId) {
    let Some(mid) = doc.node(connector).and_then(|_| route_mid(doc, connector)) else {
        let _ = doc.remove(label);
        return;
    };
    let Some(n) = doc.node(label) else { return };
    let pw = n.parent.map_or(Affine::IDENTITY, |p| doc.world_transform(p));
    let local = pw.inverse() * mid;
    let t = Affine::translate((local.x - n.width / 2.0, local.y - n.height / 2.0));
    if n.transform != t
        && let Some(n) = doc.node_mut(label)
    {
        n.transform = t;
    }
}

/// Connector `id`'s first and last points on the page as it is drawn now.
pub(super) fn ends_now(doc: &Document, id: NodeId) -> Option<(Point, Point)> {
    let NodeKind::Vector { network } = &doc.node(id)?.kind else { return None };
    let w = doc.world_transform(id);
    Some((w * network.vertices.first()?.point(), w * network.vertices.last()?.point()))
}

/// The middle of connector `id`'s route by length, on the page.
fn route_mid(doc: &Document, id: NodeId) -> Option<Point> {
    let NodeKind::Vector { network } = &doc.node(id)?.kind else { return None };
    let w = doc.world_transform(id);
    let mut pts: Vec<Point> = Vec::new();
    figcraft_geom::kurbo::flatten(network.stroke_path().iter(), 0.25, |el| {
        if let PathEl::MoveTo(p) | PathEl::LineTo(p) = el {
            pts.push(w * p);
        }
    });
    let total: f64 = pts.windows(2).map(|s| (s[1] - s[0]).hypot()).sum();
    let mut left = total / 2.0;
    for s in pts.windows(2) {
        let (a, b) = (s[0], s[1]);
        let l = (b - a).hypot();
        if l >= left && l > 0.0 {
            return Some(a.lerp(b, left / l));
        }
        left -= l;
    }
    pts.first().copied()
}

/// The path through a route's page points `pts` (see [`route`]): one curve for a curved
/// connector, straight runs otherwise.
fn route_path(pts: &[Point], routing: Routing) -> BezPath {
    let mut path = BezPath::new();
    match (routing, pts) {
        (Routing::Curved, [p0, c0, c1, p1]) => {
            path.move_to(*p0);
            path.curve_to(*c0, *c1, *p1);
        }
        _ => {
            for (i, p) in pts.iter().enumerate() {
                if i == 0 { path.move_to(*p) } else { path.line_to(*p) }
            }
        }
    }
    path
}

/// The box a path is drawn in, when it is finite.
fn finite_box(path: &BezPath) -> Option<Rect> {
    let b = path.bounding_box();
    (b.x0.is_finite() && b.y0.is_finite() && b.x1.is_finite() && b.y1.is_finite()).then_some(b)
}

/// Write the route `pts` (page points, see [`route`]) into connector `id`: its box, place and
/// path. Untouched when it already runs there, so a connector that didn't move isn't an edit.
pub(super) fn place_route(doc: &mut Document, id: NodeId, pts: &[Point], routing: Routing) {
    let Some(n) = doc.node(id) else { return };
    let pw = n.parent.map_or(Affine::IDENTITY, |p| doc.world_transform(p));
    let local = pw.inverse() * route_path(pts, routing);
    let Some(b) = finite_box(&local) else { return };
    let rel = Affine::translate((-b.x0, -b.y0)) * local;
    let network = match routing {
        Routing::Curved => VectorNetwork::from_path(&rel),
        _ => VectorNetwork::from_points(&rel.elements().iter().filter_map(PathEl::end_point).collect::<Vec<_>>(), false),
    };
    let t = Affine::translate((b.x0, b.y0));
    let same =
        n.transform == t && n.width == b.width() && n.height == b.height() && matches!(&n.kind, NodeKind::Vector { network: old } if *old == network);
    if same {
        return;
    }
    if let Some(n) = doc.node_mut(id) {
        n.kind = NodeKind::Vector { network };
        n.transform = t;
        (n.width, n.height) = (b.width(), b.height());
    }
}

/// The page points connector `c` runs through (see [`figcraft_doc::route_points`]): from the
/// middle of the side of each attached layer facing the other end (or the side it names).
pub fn route(doc: &Document, c: &Connector) -> Option<Vec<Point>> {
    let (from, to) = route_ends(doc, c)?;
    figcraft_doc::route_along(from, to, c.routing, c.path.as_ref())
}

/// Connector `c`'s ends as they are now.
pub(super) fn route_ends(doc: &Document, c: &Connector) -> Option<(RouteEnd, RouteEnd)> {
    let end = |e: &End| match e {
        End::Point { x, y } => Some(RouteEnd::Point(Point::new(*x, *y))),
        End::Node { node, side } => doc.world_bounds(*node).map(|b| RouteEnd::Box(b, *side)),
    };
    Some((end(&c.from)?, end(&c.to)?))
}
