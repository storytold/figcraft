//! Where something dropped at a page point lands: the frame under the point (or the page) and, in
//! an auto layout frame, the child slot by pointer position plus the insertion line to draw.

use figcraft_doc::{Document, LayoutMode, NodeId, NodeKind};
use figcraft_geom::Point;
use serde::Serialize;

/// A drop destination.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DropSlot {
    pub parent: NodeId,
    /// Child index in an auto layout parent (None: on top).
    pub index: Option<usize>,
    /// Insertion indicator (x0, y0, x1, y1) for auto layout parents.
    pub line: Option<[f64; 4]>,
}

/// The slot in auto layout frame `parent` for pointer `p`, ignoring `exclude` (layers being
/// moved): the index (among the children left once `exclude` is taken out) before the first
/// placed child whose centre is past the pointer, and the line to draw there. Hidden children and
/// those ignoring auto layout don't take part.
pub fn auto_layout_slot(doc: &Document, parent: NodeId, p: Point, exclude: &[NodeId]) -> Option<(usize, Option<[f64; 4]>)> {
    let pn = doc.node(parent)?;
    let layout = pn.layout.as_ref()?;
    let vertical = layout.mode == LayoutMode::Vertical;
    let kids: Vec<NodeId> = pn.children.iter().copied().filter(|c| !exclude.contains(c)).collect();
    // Placed children as (index in `kids`, id).
    let flow: Vec<(usize, NodeId)> =
        kids.iter().copied().enumerate().filter(|(_, c)| doc.node(*c).is_some_and(|n| n.visible && !n.child_layout.absolute)).collect();
    // A slot before the k-th placed child (k = its count: after the last) as an index in `kids`.
    let index = |k: usize| flow.get(k).map_or(kids.len(), |(i, _)| *i);
    let flow_ids: Vec<NodeId> = flow.iter().map(|(_, c)| *c).collect();
    if layout.wrap && layout.mode != LayoutMode::Grid {
        let (k, line) = wrap_slot(doc, &flow_ids, p, vertical);
        return Some((index(k), line));
    }
    let pos = if vertical { p.y } else { p.x };
    let k = flow_ids
        .iter()
        .position(|c| doc.world_bounds(*c).is_some_and(|b| pos < if vertical { b.center().y } else { b.center().x }))
        .unwrap_or(flow_ids.len());
    let line = flow_ids.get(k).and_then(|c| doc.world_bounds(*c)).map(|b| line_before(b, vertical, 2.0)).or_else(|| {
        flow_ids
            .last()
            .and_then(|c| doc.world_bounds(*c))
            .map(|b| if vertical { [b.x0, b.y1 + 2.0, b.x1, b.y1 + 2.0] } else { [b.x1 + 2.0, b.y0, b.x1 + 2.0, b.y1] })
    });
    Some((index(k), line))
}

/// The insertion line `off` before a child with page bounds `b` (above it in a column).
pub fn line_before(b: figcraft_geom::Rect, vertical: bool, off: f64) -> [f64; 4] {
    if vertical { [b.x0, b.y0 - off, b.x1, b.y0 - off] } else { [b.x0 - off, b.y0, b.x0 - off, b.y1] }
}

/// `r` with x and y swapped when `vertical`: a vertical wrap (columns) is a horizontal one (rows)
/// turned on its side.
pub fn flip(r: figcraft_geom::Rect, vertical: bool) -> figcraft_geom::Rect {
    if vertical { figcraft_geom::Rect::new(r.y0, r.x0, r.y1, r.x1) } else { r }
}

/// The visual rows (columns when `vertical`) of a wrapping layout's placed children `kids`, each
/// child as (its index in `kids`, its page bounds [`flip`]ped so the flow runs along x). Children
/// wrap in flow order, so a new row starts where a child sits before the one before, or where it
/// does not follow the one before along the flow but sits past it across (one child per line).
pub fn wrap_rows(doc: &Document, kids: &[NodeId], vertical: bool) -> Vec<Vec<(usize, figcraft_geom::Rect)>> {
    let mut rows: Vec<Vec<(usize, figcraft_geom::Rect)>> = Vec::new();
    for (i, b) in kids.iter().enumerate().filter_map(|(i, c)| doc.world_bounds(*c).map(|b| (i, flip(b, vertical)))) {
        match rows.last_mut() {
            Some(row) if row.last().is_some_and(|(_, prev)| b.x0 >= prev.x0 - 0.5 && !(b.x0 < prev.x1 - 0.5 && b.y0 >= prev.y1 - 0.5)) => {
                row.push((i, b))
            }
            _ => rows.push(vec![(i, b)]),
        }
    }
    rows
}

/// The slot in a wrapping layout: the visual row (column) under the pointer, then the position in it.
fn wrap_slot(doc: &Document, kids: &[NodeId], p: Point, vertical: bool) -> (usize, Option<[f64; 4]>) {
    let (k, line) = wrap_slot_rows(&wrap_rows(doc, kids, vertical), if vertical { Point::new(p.y, p.x) } else { p });
    (k, line.map(|[x0, y0, x1, y1]| if vertical { [y0, x0, y1, x1] } else { [x0, y0, x1, y1] }))
}

/// [`wrap_slot`] in rows that run along x.
fn wrap_slot_rows(rows: &[Vec<(usize, figcraft_geom::Rect)>], p: Point) -> (usize, Option<[f64; 4]>) {
    let span =
        |row: &[(usize, figcraft_geom::Rect)]| row.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), (_, b)| (lo.min(b.y0), hi.max(b.y1)));
    // The row containing the pointer, else the nearest one.
    let Some(row) = rows.iter().min_by(|a, b| {
        let d = |r: &[(usize, figcraft_geom::Rect)]| {
            let (lo, hi) = span(r);
            if p.y < lo {
                lo - p.y
            } else if p.y > hi {
                p.y - hi
            } else {
                0.0
            }
        };
        d(a).total_cmp(&d(b))
    }) else {
        return (0, None);
    };
    let (y0, y1) = span(row);
    match row.iter().find(|(_, b)| p.x < b.center().x) {
        Some((i, b)) => (*i, Some([b.x0 - 2.0, y0, b.x0 - 2.0, y1])),
        None => match row.last() {
            Some((i, b)) => (i + 1, Some([b.x1 + 2.0, y0, b.x1 + 2.0, y1])),
            None => (0, None),
        },
    }
}

/// The grid cell of auto layout grid `parent` under page point `p` (the nearest one): (column,
/// row, the cell's page-space bounds).
pub fn grid_cell_at(doc: &Document, parent: NodeId, p: Point) -> Option<(u32, u32, [f64; 4])> {
    let g = figcraft_layout::grid_geometry(doc, parent)?;
    let lp = doc.world_transform(parent).inverse() * p;
    let nearest = |tracks: &[(f64, f64)], v: f64| {
        tracks
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                let d = |t: &(f64, f64)| {
                    if v < t.0 {
                        t.0 - v
                    } else if v > t.1 {
                        v - t.1
                    } else {
                        0.0
                    }
                };
                d(a).total_cmp(&d(b))
            })
            .map(|(i, t)| (i, *t))
    };
    let (ci, (cx0, cx1)) = nearest(&g.columns, lp.x)?;
    let (ri, (ry0, ry1)) = nearest(&g.rows, lp.y)?;
    let r = figcraft_geom::transform_rect_bbox(doc.world_transform(parent), figcraft_geom::Rect::new(cx0, ry0, cx1, ry1));
    Some((u32::try_from(ci).ok()?, u32::try_from(ri).ok()?, [r.x0, r.y0, r.x1, r.y1]))
}

/// The container a drop at page point `p` goes into (`None`: the page), skipping `exclude` (layers
/// being dragged) and their descendants. Inside auto layout the deepest auto layout frame under the
/// pointer wins: plain frames there are items of the flow, not destinations (dropping onto a card
/// in a list inserts next to it). Elsewhere the deepest frame or section under the pointer wins.
/// Inside an instance only its slots (and what's in them) take drops; a slot always does. Sections
/// being dragged only go into sections.
pub fn drop_parent(doc: &Document, page: NodeId, p: Point, exclude: &[NodeId]) -> Option<NodeId> {
    /// `locked`: inside an instance, outside its slots.
    fn rec(doc: &Document, id: NodeId, p: Point, exclude: &[NodeId], in_al: bool, locked: bool, depth: usize) -> Option<NodeId> {
        if depth > 128 || exclude.contains(&id) {
            return None;
        }
        let n = doc.node(id)?;
        // Board objects (stickies, shapes with text, tables, stamps) never take other layers in: a
        // drop on one goes to what is under it.
        if !n.visible || n.locked || n.is_board_atom() {
            return None;
        }
        let container = n.kind.is_frame_like() || matches!(n.kind, NodeKind::Section { .. });
        let kids_locked = crate::components::locks_children(doc, id, locked);
        if !container {
            // Groups: frames inside them can still take the drop.
            return n.children.iter().rev().find_map(|c| rec(doc, *c, p, exclude, in_al, kids_locked, depth + 1));
        }
        let inside = doc.world_bounds(id).is_some_and(|b| b.contains(p)) && n.local_rect().contains(doc.world_transform(id).inverse() * p);
        if !inside {
            return None;
        }
        let auto_layout = n.layout.is_some();
        let deeper = n.children.iter().rev().find_map(|c| rec(doc, *c, p, exclude, in_al || auto_layout, kids_locked, depth + 1));
        deeper.or((!kids_locked && (auto_layout || !in_al || crate::components::is_slot(doc, id))).then_some(id))
    }
    let hit = doc.node(page)?.children.iter().rev().find_map(|c| rec(doc, *c, p, exclude, false, false, 0));
    // Sections go on the page or in sections: a frame under the pointer gives way to the section
    // it is in.
    if exclude.iter().any(|m| doc.node(*m).is_some_and(|n| matches!(n.kind, NodeKind::Section { .. }))) {
        return hit.and_then(|h| std::iter::once(h).chain(doc.ancestors(h)).find(|a| doc.takes_sections(*a))).filter(|a| *a != page);
    }
    hit
}

/// Whether layers dragged on the canvas inside `parent` stay in it (they only move): those of a
/// group or a boolean group (which follow their layers) and of an instance outside its slots.
pub fn keeps_layers(doc: &Document, parent: NodeId) -> bool {
    matches!(doc.node(parent).map(|n| &n.kind), Some(NodeKind::Group | NodeKind::BooleanOp { .. }))
        || crate::components::structure_locked(doc, parent)
}

/// Margin (page units) around the auto layout frame a dragged layer comes from that still keeps it.
const HOME_MARGIN: f64 = 8.0;

/// Where layers being dragged (`moving`) land: [`drop_parent`], except that the frame they come
/// from keeps them while the pointer is over it (an auto layout frame: within a few units of it)
/// and over nothing deeper. So a layer in a plain card of an auto layout list moves within the
/// card, and one reordered in a list doesn't fall out of it at the edge.
pub fn drag_parent(doc: &Document, page: NodeId, p: Point, moving: &[NodeId]) -> NodeId {
    let hit = drop_parent(doc, page, p, moving);
    let home = moving.first().and_then(|m| doc.node(*m)).and_then(|n| n.parent).filter(|h| *h != page);
    if let Some(home) = home
        && let Some(hn) = doc.node(home)
        && ((hn.kind.is_frame_like() && !matches!(hn.kind, NodeKind::Instance { .. })) || matches!(hn.kind, NodeKind::Section { .. }))
        && hit.is_none_or(|h| h != home && doc.is_ancestor(h, home))
    {
        let margin = if hn.layout.is_some() { HOME_MARGIN } else { 0.0 };
        if doc.world_bounds(home).is_some_and(|b| b.inflate(margin, margin).contains(p)) {
            return home;
        }
    }
    hit.unwrap_or(page)
}

/// Where a drop at page point `p` lands.
pub fn slot(doc: &Document, page: NodeId, p: Point) -> DropSlot {
    let parent = drop_parent(doc, page, p, &[]).unwrap_or(page);
    match auto_layout_slot(doc, parent, p, &[]) {
        Some((index, line)) => DropSlot { parent, index: Some(index), line },
        None => DropSlot { parent, index: None, line: None },
    }
}

/// The component set a component belongs to, by name (`None` for a lone component): which
/// instances an Alt-drop of it prefers to swap.
pub fn swap_family(doc: &Document, component: NodeId) -> Option<String> {
    crate::components::variant_set(doc, component).and_then(|s| doc.node(s)).map(|s| s.name.clone())
}

/// The instance that a component dropped at page point `p` with Alt held swaps: of the instances
/// under the point (innermost first; never one inside the dropped component `exclude`), the first
/// showing a variant of the same component set (`family`), else the innermost.
pub fn swap_target(doc: &Document, page: NodeId, p: Point, family: Option<&str>, exclude: Option<NodeId>) -> Option<NodeId> {
    let hit = doc.hit_test(page, p, 0.0, Some(&crate::booleans::result_outline(doc, &[])))?;
    let chain: Vec<NodeId> = std::iter::once(hit)
        .chain(doc.ancestors(hit))
        .filter(|i| matches!(doc.node(*i).map(|n| &n.kind), Some(NodeKind::Instance { .. })))
        .filter(|i| exclude.is_none_or(|c| !doc.is_ancestor(c, *i)))
        .collect();
    let same_family = |i: &NodeId| match (doc.node(*i).map(|n| &n.kind), family) {
        (Some(NodeKind::Instance { component, .. }), Some(f)) => swap_family(doc, *component).as_deref() == Some(f),
        _ => false,
    };
    chain.iter().copied().find(same_family).or_else(|| chain.first().copied())
}
