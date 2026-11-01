//! Node geometry: local outline paths, and hit testing with Figma's click-target rules.

use crate::{Document, Node, NodeId, NodeKind};
use figcraft_geom::{BezPath, Point, Rect, Shape};

/// The node's outline in its local space (`None` for groups, booleans, text and pages, whose
/// geometry comes from children or glyphs).
pub fn shape_path(n: &Node) -> Option<BezPath> {
    let (w, h) = (n.width, n.height);
    let c = &n.corners;
    Some(match &n.kind {
        NodeKind::Rectangle
        | NodeKind::Frame
        | NodeKind::Component { .. }
        | NodeKind::ComponentSet { .. }
        | NodeKind::Instance { .. }
        | NodeKind::Section { .. } => figcraft_geom::rounded_rect(w, h, c.radii, c.smoothing),
        NodeKind::Slice => figcraft_geom::rounded_rect(w, h, [0.0; 4], 0.0),
        NodeKind::Ellipse { start, sweep, inner } => figcraft_geom::ellipse(w, h, *start, *sweep, *inner),
        NodeKind::Polygon { count } => figcraft_geom::smoothed_polygon(&figcraft_geom::polygon_points(w, h, *count), c.radii[0], c.smoothing),
        NodeKind::Star { count, inner_ratio } => {
            figcraft_geom::smoothed_polygon(&figcraft_geom::star_points(w, h, *count, *inner_ratio), c.radii[0], c.smoothing)
        }
        NodeKind::Line => {
            let mut p = BezPath::new();
            p.move_to((0.0, 0.0));
            p.line_to((w, 0.0));
            p
        }
        NodeKind::Vector { network } => {
            let f = network.fill_path();
            if f.elements().is_empty() { network.stroke_path() } else { f }
        }
        NodeKind::Text(_) => figcraft_geom::rounded_rect(w, h, [0.0; 4], 0.0),
        NodeKind::Page { .. } | NodeKind::Group | NodeKind::BooleanOp { .. } => return None,
    })
}

/// The open/closed path strokes follow (vector networks stroke every segment, filled or not).
pub fn stroke_geometry(n: &Node) -> Option<BezPath> {
    match &n.kind {
        NodeKind::Vector { network } => Some(network.stroke_path()),
        _ => shape_path(n),
    }
}

/// Whether the shape's path is closed (fills apply).
pub fn is_closed(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Line => false,
        NodeKind::Vector { network } => !network.regions.is_empty(),
        _ => true,
    }
}

/// How much further than `tol` a stroke with nothing filled reaches: a thin line or stroke-only path
/// is hard to land on with the plain tolerance, so it gets a wider band (5 screen px for the usual 2 px tolerance).
const STROKE_REACH: f64 = 2.5;

/// Whether `pt` hits `n` itself; in `outline` mode (nothing filled on the canvas) a shape only where
/// its outline is.
fn hits_node_itself(doc: &Document, id: NodeId, n: &Node, pt: Point, tol: f64, outline: bool) -> bool {
    let inv = doc.world_transform(id).inverse();
    let lp = inv * pt;
    // Tolerance in local units (approximate under scale).
    let tol_l = tol;
    match &n.kind {
        NodeKind::Group | NodeKind::BooleanOp { .. } | NodeKind::Page { .. } => false,
        NodeKind::Text(_) | NodeKind::Slice => n.local_rect().inflate(tol_l, tol_l).contains(lp),
        NodeKind::Line => {
            let half = (n.stroke.weight / 2.0).max(tol_l * STROKE_REACH);
            lp.x >= -half && lp.x <= n.width + half && lp.y.abs() <= half
        }
        NodeKind::Frame | NodeKind::Component { .. } | NodeKind::ComponentSet { .. } | NodeKind::Instance { .. } | NodeKind::Section { .. } => {
            n.local_rect().inflate(tol_l, tol_l).contains(lp)
        }
        NodeKind::Vector { network } if network.regions.iter().any(|r| r.fills.is_some() || r.paint_space.is_some()) => {
            let rounded = network.rounded();
            let net = rounded.as_ref().unwrap_or(network);
            if !outline {
                for (i, region) in net.regions.iter().enumerate().rev() {
                    let paints = region.fills.as_deref().unwrap_or(&n.fills);
                    if paints.iter().any(|p| p.visible) && net.region_contains(i, lp) {
                        return true;
                    }
                }
            }
            let path = net.stroke_path();
            let has_fill = net.regions.iter().any(|region| region.fills.as_deref().unwrap_or(&n.fills).iter().any(|paint| paint.visible));
            let stroke_only = n.has_visible_stroke() && !has_fill;
            let reach = (n.stroke.max_weight() / 2.0).max(if stroke_only { tol_l * STROKE_REACH } else { tol_l });
            path.bounding_box().inflate(reach, reach).contains(lp) && near_path(&path, lp, reach)
        }
        _ => shape_path(n).is_some_and(|path| hits_path(n, &path, lp, tol_l, !outline && is_closed(n) && fills_point(n, &path, lp))),
    }
}

/// Whether the fill of `n`, drawn as `path`, covers `lp` (vectors follow the rule of
/// [`VectorNetwork::fill_contains`](crate::VectorNetwork::fill_contains): even-odd when any region asks for it).
fn fills_point(n: &Node, path: &BezPath, lp: Point) -> bool {
    match &n.kind {
        NodeKind::Vector { network } => network.fill_contains(lp),
        _ => path.winding(lp) != 0,
    }
}

/// Whether local point `lp` hits `n` drawn as `path`: `inside` it (counted when it's filled, or
/// draws nothing at all), or within reach of its outline (its stroke, else the tolerance).
fn hits_path(n: &Node, path: &BezPath, lp: Point, tol: f64, inside: bool) -> bool {
    if inside && (n.fills.iter().any(|f| f.visible) || !n.has_visible_stroke()) {
        return true;
    }
    // Only a stroke with nothing filled gets the wider band: a filled shape's edge keeps the plain one.
    let stroke_only = n.has_visible_stroke() && !n.fills.iter().any(|f| f.visible);
    let reach = (n.stroke.max_weight() / 2.0).max(if stroke_only { tol * STROKE_REACH } else { tol });
    if !path.bounding_box().inflate(reach, reach).contains(lp) {
        return false;
    }
    near_path(path, lp, reach)
}

/// A boolean group's computed result outline in its local space (filled even-odd), or `None` to
/// hit-test its operands instead. Boolean operations live above this crate: callers that can
/// compute results pass them in; `None` for the whole argument hit-tests operands everywhere.
pub type BoolResult<'a> = Option<&'a dyn Fn(NodeId, &Node) -> Option<BezPath>>;

/// With a result for boolean group `id`, whether `pt` hits it (`None`: no result, test operands).
fn hits_result(doc: &Document, id: NodeId, n: &Node, pt: Point, tol: f64, result: BoolResult, outline: bool) -> Option<bool> {
    let path = result.and_then(|f| f(id, n))?;
    let lp = doc.world_transform(id).inverse() * pt;
    Some(hits_path(n, &path, lp, tol, !outline && path.winding(lp) % 2 != 0))
}

fn near_path(path: &BezPath, p: Point, dist: f64) -> bool {
    use figcraft_geom::kurbo::ParamCurveNearest;
    path.segments().any(|s| s.nearest(p, 0.01).distance_sq <= dist * dist)
}

/// The innermost visible, unlocked node under `pt` (page coordinates), honouring clip content;
/// boolean groups are hit where their `result` is.
pub fn hit_leaf(doc: &Document, page: NodeId, pt: Point, tol: f64, result: BoolResult) -> Option<NodeId> {
    let p = doc.node(page)?;
    p.children.iter().rev().find_map(|c| hit_rec(doc, *c, pt, tol, 0, result, None).filter(|_| !masked_out(doc, p, *c, pt)))
}

/// Whether child `c` of `parent` is masked at `pt`: a visible mask below it (the nearest mask
/// below it, a hidden one masks nothing) doesn't cover the point, so nothing of `c` shows there.
pub fn masked_out(doc: &Document, parent: &Node, c: NodeId, pt: Point) -> bool {
    if !doc.has_masks() {
        return false;
    }
    let below = parent.children.iter().position(|k| *k == c).and_then(|i| parent.children.get(..i)).unwrap_or(&[]);
    let mask = below.iter().rev().filter_map(|k| doc.node(*k).map(|n| (*k, n))).find(|(_, n)| n.is_mask);
    mask.is_some_and(|(m, n)| n.visible && !covers(doc, m, n, pt, 0))
}

/// Whether mask `id` draws at `pt` (its shape, or for a group or frame what its layers draw).
fn covers(doc: &Document, id: NodeId, n: &Node, pt: Point, depth: usize) -> bool {
    depth < 512
        && (hits_node_itself(doc, id, n, pt, 0.0, false)
            || n.children.iter().any(|c| doc.node(*c).is_some_and(|k| k.visible && !k.is_mask && covers(doc, *c, k, pt, depth + 1))))
}

/// Like [`hit_leaf`], but only within layer `root` (it included).
pub fn hit_in(doc: &Document, root: NodeId, pt: Point, tol: f64, result: BoolResult) -> Option<NodeId> {
    hit_rec(doc, root, pt, tol, 0, result, None)
}

/// [`hit_leaf`] (or [`hit_in`] for a `root` that isn't a page) in outline mode: shapes are hit
/// where their outlines are, so a click inside one reaches the layers drawn behind it.
///
/// `also` is what outline mode draws besides the shapes: hidden layers are hit like the others
/// when it shows them, and every layer on the edge of its box when it shows object bounds.
pub fn hit_outlines(doc: &Document, root: NodeId, pt: Point, tol: f64, result: BoolResult, also: OutlineHits) -> Option<NodeId> {
    match doc.node(root) {
        Some(p) if p.is_page() => p.children.iter().rev().find_map(|c| hit_rec(doc, *c, pt, tol, 0, result, Some(also))),
        _ => hit_rec(doc, root, pt, tol, 0, result, Some(also)),
    }
}

/// What outline mode shows besides the shapes' outlines (see [`hit_outlines`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OutlineHits {
    pub hidden: bool,
    pub bounds: bool,
}

/// Whether `pt` is within `tol` of the edge of `n`'s box.
fn on_box_edge(doc: &Document, id: NodeId, n: &Node, pt: Point, tol: f64) -> bool {
    let lp = doc.world_transform(id).inverse() * pt;
    let r = n.local_rect();
    r.inflate(tol, tol).contains(lp) && !(r.width() > 2.0 * tol && r.height() > 2.0 * tol && r.inflate(-tol, -tol).contains(lp))
}

fn hit_rec(doc: &Document, id: NodeId, pt: Point, tol: f64, depth: usize, result: BoolResult, also: Option<OutlineHits>) -> Option<NodeId> {
    if depth > 512 {
        return None;
    }
    let n = doc.node(id)?;
    let outline = also.is_some();
    if (!n.visible && !also.is_some_and(|a| a.hidden)) || n.locked {
        return None;
    }
    if let NodeKind::Section { contents_hidden: true } = n.kind {
        return hits_node_itself(doc, id, n, pt, tol, outline).then_some(id);
    }
    let clips = n.clip && n.kind.is_frame_like();
    if clips && !hits_node_itself(doc, id, n, pt, 0.0, false) {
        return None;
    }
    // A boolean group is hit where its result is (without one: where an operand is). The leaf is
    // the operand under the point, so a plain click still selects the group (see `select_target`)
    // while a deep click, a double click or a selected sibling operand reaches the operand itself.
    let in_result = if matches!(n.kind, NodeKind::BooleanOp { .. }) { hits_result(doc, id, n, pt, tol, result, outline) } else { None };
    if in_result == Some(false) {
        return None;
    }
    for c in n.paint_order().rev() {
        if let Some(h) = hit_rec(doc, c, pt, tol, depth + 1, result, also).filter(|_| !masked_out(doc, n, c, pt)) {
            return Some(h);
        }
    }
    if in_result == Some(true) || hits_node_itself(doc, id, n, pt, tol, outline) || also.is_some_and(|a| a.bounds) && on_box_edge(doc, id, n, pt, tol)
    {
        return Some(id);
    }
    None
}

/// Every visible, unlocked layer under `pt` (page coordinates) with its depth below the page, in
/// layer-tree order (each layer before what it contains, the front-most first): what
/// "Select layer" in the context menu lists. Clip content is honoured; boolean groups (hit where
/// their `result` is) and sections with hidden contents count as one layer.
pub fn layers_at(doc: &Document, page: NodeId, pt: Point, tol: f64, result: BoolResult) -> Vec<(NodeId, usize)> {
    fn rec(doc: &Document, id: NodeId, pt: Point, tol: f64, depth: usize, result: BoolResult, out: &mut Vec<(NodeId, usize)>) {
        let Some(n) = doc.node(id) else { return };
        if !n.visible || n.locked || depth > 512 {
            return;
        }
        let clips = n.clip && n.kind.is_frame_like();
        if clips && !hits_node_itself(doc, id, n, pt, 0.0, false) {
            return;
        }
        let at = out.len();
        let whole = matches!(n.kind, NodeKind::BooleanOp { .. } | NodeKind::Section { contents_hidden: true });
        if whole {
            if hit_rec(doc, id, pt, tol, depth, result, None).is_some() {
                out.push((id, depth));
            }
            return;
        }
        for c in n.paint_order().rev() {
            let before = out.len();
            rec(doc, c, pt, tol, depth + 1, result, out);
            if out.len() > before && masked_out(doc, n, c, pt) {
                out.truncate(before);
            }
        }
        if out.len() > at || hits_node_itself(doc, id, n, pt, tol, false) {
            out.insert(at, (id, depth));
        }
    }
    let mut out = Vec::new();
    if let Some(p) = doc.node(page) {
        for c in p.children.iter().rev() {
            let before = out.len();
            rec(doc, *c, pt, tol, 0, result, &mut out);
            if out.len() > before && masked_out(doc, p, *c, pt) {
                out.truncate(before);
            }
        }
    }
    out
}

/// Resolve a clicked leaf to what a plain click selects.
///
/// Figma's rules: top-level frames behave like artboards, so a click selects their direct child;
/// groups select as a whole; when something is already selected, its siblings (and their
/// contents) resolve relative to that context, which is how double-click "enters" groups.
/// `deep` (Cmd-click) selects the leaf itself.
pub fn select_target(doc: &Document, leaf: NodeId, context: &[NodeId], deep: bool) -> NodeId {
    if deep {
        return leaf;
    }
    let target = level_target(doc, leaf, context);
    // A board object is picked whole (a sticky, not its text), unless a layer inside it is selected.
    let mut chain = doc.ancestors(target);
    chain.reverse();
    chain.push(target);
    chain
        .into_iter()
        .find(|c| doc.node(*c).is_some_and(Node::is_board_atom) && !context.iter().any(|s| s != c && doc.is_ancestor(*c, *s)))
        .unwrap_or(target)
}

fn level_target(doc: &Document, leaf: NodeId, context: &[NodeId]) -> NodeId {
    // Chain from the page's child down to the leaf.
    let mut chain: Vec<NodeId> = doc.ancestors(leaf);
    chain.reverse();
    chain.push(leaf);
    // chain[0] is the page.
    let chain: Vec<NodeId> = chain.into_iter().skip(1).collect();
    let Some(top) = chain.first().copied() else { return leaf };
    // Context: if a selected node's parent is in the chain, select that parent's child in the chain.
    let mut best: Option<usize> = None;
    for s in context {
        let Some(parent) = doc.node(*s).and_then(|n| n.parent) else { continue };
        if let Some(pos) = chain.iter().position(|c| *c == parent)
            && pos + 1 < chain.len()
        {
            best = Some(best.map_or(pos + 1, |b: usize| b.max(pos + 1)));
        }
    }
    if let Some(i) = best
        && let Some(id) = chain.get(i)
    {
        return *id;
    }
    let top_is_board = doc.node(top).is_some_and(|n| n.kind.is_frame_like() || matches!(n.kind, NodeKind::Section { .. }));
    if top_is_board && chain.len() > 1 {
        // Sections contain frames: the frame inside a section is the board.
        let is_section = |i: usize| chain.get(i).and_then(|c| doc.node(*c)).is_some_and(|n| matches!(n.kind, NodeKind::Section { .. }));
        let is_board =
            |i: usize| chain.get(i).and_then(|c| doc.node(*c)).is_some_and(|n| n.kind.is_frame_like() || matches!(n.kind, NodeKind::Section { .. }));
        let mut i = 1;
        while is_section(i - 1) && is_board(i) && i + 1 < chain.len() {
            i += 1;
        }
        return chain.get(i).copied().unwrap_or(leaf);
    }
    top
}

/// What a double click on `leaf` selects: one level deeper than the selection, i.e. the layer
/// holding `leaf` just inside the deepest selected layer that holds it (entering groups, frames
/// and boolean groups), or the plain click target when no selected layer holds it.
pub fn enter_target(doc: &Document, leaf: NodeId, selection: &[NodeId]) -> NodeId {
    let mut chain: Vec<NodeId> = doc.ancestors(leaf);
    chain.reverse();
    chain.push(leaf);
    let inner = chain.iter().rposition(|c| selection.contains(c)).and_then(|i| chain.get(i + 1)).copied();
    inner.unwrap_or_else(|| select_target(doc, leaf, selection, false))
}

/// Nodes on `page` whose world bounds intersect `rect` (marquee): top-level children, or the
/// children of top-level frames when the marquee starts inside a frame (`within`).
pub fn marquee(doc: &Document, page: NodeId, rect: Rect, within: Option<NodeId>) -> Vec<NodeId> {
    let parent = within.unwrap_or(page);
    doc.node(parent).map(|p| marquee_children(doc, p, rect)).unwrap_or_default()
}

/// Nodes on `page` a Cmd/Ctrl marquee selects: the innermost layers whose bounds touch `rect`
/// (frames, groups and instances are looked into; a container none of whose children are touched
/// counts itself; boolean groups count as one shape).
pub fn marquee_deep(doc: &Document, page: NodeId, rect: Rect) -> Vec<NodeId> {
    fn walk(doc: &Document, parent: NodeId, rect: Rect, depth: usize, out: &mut Vec<NodeId>) {
        let Some(p) = doc.node(parent) else { return };
        for c in marquee_children(doc, p, rect) {
            let before = out.len();
            let opens = doc.node(c).is_some_and(|n| !n.children.is_empty() && !matches!(n.kind, NodeKind::BooleanOp { .. }));
            if opens && depth < 256 {
                walk(doc, c, rect, depth + 1, out);
            }
            if out.len() == before {
                out.push(c);
            }
        }
    }
    let mut out = Vec::new();
    walk(doc, page, rect, 0, &mut out);
    out
}

/// The visible, unlocked children of `p` whose world bounds touch `rect` (on both axes: lines and
/// other flat layers too).
fn marquee_children(doc: &Document, p: &Node, rect: Rect) -> Vec<NodeId> {
    p.children
        .iter()
        .copied()
        .filter(|c| doc.node(*c).is_some_and(|n| n.visible && !n.locked))
        .filter(|c| doc.world_bounds(*c).is_some_and(|b| b.x0 <= rect.x1 && b.x1 >= rect.x0 && b.y0 <= rect.y1 && b.y1 >= rect.y0))
        .collect()
}

/// Figma marquee rule: a marquee that fully contains a top-level frame selects the frame; one
/// starting on empty canvas selects top-level objects it touches.
pub fn marquee_select(doc: &Document, page: NodeId, rect: Rect, start_inside: Option<NodeId>) -> Vec<NodeId> {
    if let Some(frame) = start_inside {
        let inside = marquee(doc, page, rect, Some(frame));
        return inside;
    }
    marquee(doc, page, rect, None)
}

/// Bounds of a node's outline in page coordinates for snapping (the box corners and centre).
pub fn snap_points(doc: &Document, id: NodeId) -> Vec<Point> {
    let Some(b) = doc.world_bounds(id) else { return vec![] };
    vec![
        Point::new(b.x0, b.y0),
        Point::new(b.x1, b.y0),
        Point::new(b.x1, b.y1),
        Point::new(b.x0, b.y1),
        b.center(),
        Point::new(b.center().x, b.y0),
        Point::new(b.center().x, b.y1),
        Point::new(b.x0, b.center().y),
        Point::new(b.x1, b.center().y),
    ]
}
