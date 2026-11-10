//! Mind maps (#2226): a node is a frame hugging one text ([`BoardItem::MindNode`]) and a branch is
//! a connector with `branch` set, from the parent to the child. The tree is read from the branches
//! and from nothing else; [`settle_mind_maps`] lays a touched map out again in the normalize pass,
//! so the commands only make nodes and branches.

use std::collections::{BTreeMap, BTreeSet};

use figcraft_color::{Color, Paint};
use figcraft_doc::{
    AutoLayout, BoardItem, Connector, Document, End, LetterSpacing, LineHeight, MIND_GAP_X, MIND_GAP_Y, MIND_MAX_DEPTH, MIND_MAX_NODES, MindBox,
    NodeId, NodeKind, Routing, Side, Sizing, StrokeCap, TextAlignH, TextAutoResize, mind_layout,
};
use figcraft_geom::{Affine, Point, Rect};
use serde_json::{Value, json};

use super::connectors::make;
use super::items::{add, add_text, owner_of_text};
use super::{INK, Result, Session, bad, boolean, get, id, is_board, num_or, parent_of, require, string, textedit};
use crate::EngineError;

/// The first node's text size and the other nodes'.
const MIND_ROOT_SIZE: f64 = 24.0;
const MIND_NODE_SIZE: f64 = 16.0;
/// The line height of every node's text, in percent, and the letter spacing of the first node's
/// and the others' (percent of the size).
const MIND_LINE_HEIGHT: f64 = 150.0;
const MIND_ROOT_TRACKING: f64 = -1.9;
const MIND_NODE_TRACKING: f64 = -1.1;
/// A node's least width: an emptied one stays clickable.
const MIND_MIN_W: f64 = 8.0;
/// What `board.mindMap` puts in the first node and its children when it isn't told.
const MIND_ROOT_TEXT: &str = "Topic";
const MIND_STARTERS: [&str; 3] = ["A concept", "An idea", "A thought"];
/// The most children `board.mindMap` makes, and the longest text it puts in a node.
const MIND_MAX_STARTERS: usize = 20;
const MIND_MAX_TEXT: usize = 500;
/// How far from the page's middle a first node may be put.
const MIND_MAX_COORD: f64 = 1.0e9;
/// Branches' look.
const BRANCH: Color = Color::from_rgb8_const(0x75, 0x75, 0x75);
const BRANCH_WEIGHT: f64 = 4.0;

/// A node's frame and its text, `text` at `size`: the frame hugs the text with no padding, and the
/// text grows on one line.
fn make_node(doc: &mut Document, parent: NodeId, at: (f64, f64), text: &str, root: bool) -> Result<(NodeId, NodeId)> {
    let size = if root { MIND_ROOT_SIZE } else { MIND_NODE_SIZE };
    let line = size * MIND_LINE_HEIGHT / 100.0;
    let frame = add(doc, parent, NodeKind::Frame, "Mind map node", at, (MIND_MIN_W, line), |n| {
        n.fills.clear();
        n.clip = false;
        n.layout = Some(AutoLayout { padding: [0.0; 4], ..AutoLayout::default() });
        n.child_layout.sizing_h = Sizing::Hug;
        n.child_layout.sizing_v = Sizing::Hug;
        n.child_layout.min_w = Some(MIND_MIN_W);
        n.child_layout.min_h = Some(line);
        n.board = Some(BoardItem::MindNode);
    })?;
    let label = add_text(doc, frame, text, size, INK, TextAlignH::Left, 1.0)?;
    if let Some(n) = doc.node_mut(label) {
        n.child_layout.sizing_h = Sizing::Hug;
        if let NodeKind::Text(td) = &mut n.kind {
            td.auto_resize = TextAutoResize::WidthAndHeight;
            td.style.line_height = LineHeight::Percent(MIND_LINE_HEIGHT);
            td.style.letter_spacing = LetterSpacing::Percent(if root { MIND_ROOT_TRACKING } else { MIND_NODE_TRACKING });
            if root {
                td.style.font_style = "Bold".into();
            } else {
                td.style.font_style = "Medium".into();
            }
        }
    }
    Ok((frame, label))
}

/// The branch from `parent` to `child` (on its left side when `left`), on `page`.
fn branch(doc: &mut Document, page: NodeId, parent: NodeId, child: NodeId, left: bool) -> Result<NodeId> {
    let (out, back) = if left { (Side::Left, Side::Right) } else { (Side::Right, Side::Left) };
    let c = Connector {
        from: End::Node { node: parent, side: out, at: None },
        to: End::Node { node: child, side: back, at: None },
        routing: Routing::Curved,
        path: None,
        branch: true,
    };
    let line = make(doc, page, c, (StrokeCap::Round, StrokeCap::Round))?;
    let n = doc.get_mut(line)?;
    n.strokes = vec![Paint::solid(BRANCH)];
    n.stroke.weight = BRANCH_WEIGHT;
    Ok(line)
}

/// A branch of a map as the tree reads it.
#[derive(Clone, Copy)]
struct Branch {
    line: NodeId,
    parent: NodeId,
    child: NodeId,
    left: bool,
}

/// The branches of `doc` that join two different layers of their own page, by id. (Others are
/// the normalize pass's to remove.)
fn branches(doc: &Document) -> Vec<Branch> {
    let mut v: Vec<Branch> = doc
        .nodes()
        .filter_map(|n| match &n.board {
            Some(BoardItem::Connector(Connector { from: End::Node { node: a, side, .. }, to: End::Node { node: b, .. }, branch: true, .. }))
                if a != b && doc.contains(*a) && doc.contains(*b) && doc.page_of(*a) == doc.page_of(n.id) && doc.page_of(*b) == doc.page_of(n.id) =>
            {
                Some(Branch { line: n.id, parent: *a, child: *b, left: *side == Side::Left })
            }
            _ => None,
        })
        .collect();
    v.sort_by_key(|b| b.line);
    v
}

/// The branch arriving at `node`: the one with the lowest connector id, when it has several.
fn arriving(bs: &[Branch], node: NodeId) -> Option<Branch> {
    bs.iter().find(|b| b.child == node).copied()
}

/// The children of `node` on one side, in layout order (by their middles, then id).
fn map_children(doc: &Document, bs: &[Branch], node: NodeId, left: bool) -> Vec<NodeId> {
    let mut v: Vec<(f64, NodeId)> = bs
        .iter()
        .filter(|b| b.parent == node && b.left == left && arriving(bs, b.child).is_some_and(|a| a.line == b.line))
        .filter_map(|b| doc.world_bounds(b.child).map(|r| (r.center().y, b.child)))
        .collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    v.into_iter().map(|p| p.1).collect()
}

/// Whether `node` is on the left of its map (a first node: no).
fn map_side(bs: &[Branch], node: NodeId) -> bool {
    let (mut at, mut left) = (node, false);
    for _ in 0..MIND_MAX_DEPTH {
        let Some(b) = arriving(bs, at) else { break };
        (at, left) = (b.parent, b.left);
    }
    left
}

/// The text `p` gives for `key`, a node's at most [`MIND_MAX_TEXT`] characters.
fn text_param(p: &Value, key: &str) -> String {
    string(p, key).unwrap_or_default().chars().take(MIND_MAX_TEXT).collect()
}

/// A coordinate from the params, finite and within [`MIND_MAX_COORD`] of the origin.
fn coord(cmd: &str, p: &Value, key: &str, default: f64) -> Result<f64> {
    let v = num_or(p, key, default);
    if v.is_finite() && v.abs() <= MIND_MAX_COORD { Ok(v) } else { Err(bad(cmd, format!("`{key}` is a position within a billion of the origin"))) }
}

/// `children?` of `board.mindMap`: the starter texts.
fn starters(cmd: &str, p: &Value) -> Result<Vec<String>> {
    match get(p, "children") {
        None => Ok(MIND_STARTERS.iter().map(|t| (*t).to_string()).collect()),
        Some(Value::Array(a)) if a.len() <= MIND_MAX_STARTERS => {
            a.iter().map(|v| v.as_str().map(|t| t.chars().take(MIND_MAX_TEXT).collect()).ok_or_else(|| bad(cmd, "`children` holds texts"))).collect()
        }
        _ => Err(bad(cmd, format!("`children` is a list of at most {MIND_MAX_STARTERS} texts"))),
    }
}

pub(super) fn mind_map_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.mindMap";
    require(s, cmd)?;
    let parent = parent_of(cmd, s, p)?;
    let (vx, vy) = {
        let v = &s.ds()?.view;
        (v.cx, v.cy)
    };
    let (x, y) = (coord(cmd, p, "x", vx)?, coord(cmd, p, "y", vy)?);
    let texts = starters(cmd, p)?;
    let root_text = string(p, "text").map_or_else(|| MIND_ROOT_TEXT.to_string(), |_| text_param(p, "text"));
    let page = s.page()?;
    let doc = s.doc_mut()?;
    let (root, label) = make_node(doc, parent, (x, y), &root_text, true)?;
    let mut made = Vec::new();
    // Anywhere below one another will do: the normalize pass puts them right, in this order.
    let line = MIND_NODE_SIZE * MIND_LINE_HEIGHT / 100.0 + MIND_GAP_Y;
    for (i, t) in texts.iter().enumerate() {
        let (k, text) = make_node(doc, parent, (x + MIND_MIN_W + MIND_GAP_X, y + i as f64 * line), t, false)?;
        let b = branch(doc, page, root, k, false)?;
        made.push(json!({"id": k, "text": text, "connector": b}));
    }
    s.set_selection(vec![root]);
    if boolean(p, "edit") == Some(true) {
        s.request_ui("board.editText", json!({"id": root}));
    }
    Ok(json!({"id": root, "text": label, "children": made}))
}

pub(super) fn mind_map_add(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.mindMapAdd";
    require(s, cmd)?;
    let editing = s.ds()?.text_edit.as_ref().map(|t| t.id);
    let doc = s.doc()?;
    let from = id(p, "id")
        .or_else(|| editing.and_then(|t| owner_of_text(doc, t)))
        .or_else(|| s.selection().first().copied().map(|i| owner_of_text(doc, i).unwrap_or(i)))
        .ok_or_else(|| EngineError::Disabled(cmd.into(), "select a mind map node first".into()))?;
    let n = doc.get(from)?;
    if n.board != Some(BoardItem::MindNode) {
        return Err(bad(cmd, "that isn't a mind map node"));
    }
    let sibling = match string(p, "relation").as_deref() {
        None | Some("child") => false,
        Some("sibling") => true,
        Some(other) => return Err(bad(cmd, format!("`relation` is child or sibling, not `{other}`"))),
    };
    let (page, container) = (s.page()?, n.parent.ok_or_else(|| bad(cmd, "that node has no parent"))?);
    if doc.page_of(from) != Some(page) {
        return Err(bad(cmd, "that node is on another page"));
    }
    let bs = branches(doc);
    let at = doc.world_bounds(from).ok_or_else(|| bad(cmd, "that node has no place"))?;
    let new_h = MIND_NODE_SIZE * MIND_LINE_HEIGHT / 100.0;
    // The parent, the side, and where the new node's top-left goes on the page.
    let (parent, left, top_left) = if sibling {
        let a = arriving(&bs, from).ok_or_else(|| bad(cmd, "the first node has no siblings: add a child"))?;
        // Its middle lands between this node and the next sibling, so the order puts it right after.
        (a.parent, a.left, Point::new(at.x0, at.y1 + MIND_GAP_Y / 2.0 - new_h / 2.0))
    } else {
        let left = map_side(&bs, from);
        let last = map_children(doc, &bs, from, left).last().and_then(|l| doc.world_bounds(*l));
        let top = last.map_or(at.center().y - new_h / 2.0, |l| l.y1 + MIND_GAP_Y);
        (from, left, Point::new(if left { at.x0 - MIND_GAP_X - MIND_MIN_W } else { at.x1 + MIND_GAP_X }, top))
    };
    let local = doc.world_transform(container).inverse() * top_left;
    if !(local.x.is_finite() && local.y.is_finite()) {
        return Err(bad(cmd, "the new node can't be placed there"));
    }
    let text = text_param(p, "text");
    if editing.is_some() {
        textedit::end(s)?;
    }
    let doc = s.doc_mut()?;
    let (node, label) = make_node(doc, container, (local.x, local.y), &text, false)?;
    let line = branch(doc, page, parent, node, left)?;
    s.set_selection(vec![node]);
    if boolean(p, "edit") == Some(true) {
        s.request_ui("board.editText", json!({"id": node}));
    }
    Ok(json!({"id": node, "text": label, "connector": line}))
}

/// The tree of a document's branches: for each layer its arriving branch (the lowest connector
/// id), and each parent's children by it.
type Children = BTreeMap<NodeId, Vec<Branch>>;

fn children_of(bs: &[Branch]) -> (BTreeSet<NodeId>, Children) {
    let mut arrived = BTreeSet::new();
    let mut kids: Children = BTreeMap::new();
    for b in bs {
        if arrived.insert(b.child) {
            kids.entry(b.parent).or_default().push(*b);
        }
    }
    (arrived, kids)
}

/// Every branch that no longer joins two layers of its own page is removed, and the layers it
/// joined that are still there count as stirred (so do the old ends of a touched branch: one that
/// is gone, or that was moved to join something else).
fn prune_branches(doc: &mut Document, before: &Document, touched: &BTreeSet<NodeId>) -> BTreeSet<NodeId> {
    let mut stirred = BTreeSet::new();
    let ends = |c: &Connector| [c.from.node(), c.to.node()].into_iter().flatten().collect::<Vec<NodeId>>();
    for id in touched {
        if let Some(BoardItem::Connector(c)) = before.node(*id).and_then(|n| n.board.as_ref())
            && c.branch
        {
            stirred.extend(ends(c));
        }
    }
    let mut gone = Vec::new();
    for n in doc.nodes() {
        let Some(BoardItem::Connector(c)) = &n.board else { continue };
        let page = doc.page_of(n.id);
        let joins = |e: &End| matches!(e, End::Node { node, .. } if doc.contains(*node) && page.is_some() && doc.page_of(*node) == page);
        if c.branch && !(joins(&c.from) && joins(&c.to) && c.from.node() != c.to.node()) {
            gone.push((n.id, ends(c)));
        }
    }
    for (line, joined) in gone {
        stirred.extend(joined);
        let _ = doc.remove(line);
    }
    stirred.retain(|i| doc.contains(*i));
    stirred
}

/// The boxes of the map under `root`, breadth first (so a parent always comes before its
/// children), and the connectors that hold it together. `None`: a box is missing, or the map has
/// more than [`MIND_MAX_NODES`] nodes (it is left as it is).
fn map_boxes(doc: &Document, root: NodeId, kids: &Children) -> Option<(Vec<MindBox>, Vec<NodeId>)> {
    let first = doc.world_bounds(root)?;
    let mut boxes = vec![MindBox { id: root, rect: first, parent: None, left: false }];
    let (mut lines, mut seen) = (Vec::new(), BTreeSet::from([root]));
    let (mut at, mut depths) = (0, vec![0_usize]);
    while let Some(b) = boxes.get(at).copied() {
        let depth = depths.get(at).copied().unwrap_or(MIND_MAX_DEPTH);
        for k in kids.get(&b.id).filter(|_| depth < MIND_MAX_DEPTH).into_iter().flatten() {
            if !seen.insert(k.child) {
                continue;
            }
            if boxes.len() >= MIND_MAX_NODES {
                return None;
            }
            boxes.push(MindBox { id: k.child, rect: doc.world_bounds(k.child)?, parent: Some(at), left: k.left });
            depths.push(depth + 1);
            lines.push(k.line);
        }
        at += 1;
    }
    Some((boxes, lines))
}

/// Move layer `id` by `d` on the page.
fn shift(doc: &mut Document, id: NodeId, d: Point) {
    let parent = doc.node(id).and_then(|n| n.parent).map_or(Affine::IDENTITY, |p| doc.world_transform(p));
    let inverse = parent.inverse();
    if !inverse.as_coeffs().iter().all(|v| v.is_finite()) {
        return;
    }
    if let Some(n) = doc.node_mut(id) {
        let moved = inverse * Affine::translate((d.x, d.y)) * parent * n.transform;
        if moved.as_coeffs().iter().all(|v| v.is_finite()) {
            n.transform = moved;
        }
    }
}

/// After layout and before connectors follow: the maps an edit touched are laid out again from
/// their first nodes (a map that is already in place is not written to, so the pass is idempotent).
/// A branch that no longer joins two layers of its page is removed first: the only structural
/// write, and what makes a deleted node's children float free and a detached node stay detached.
pub(crate) fn settle_mind_maps(doc: &mut Document, before: &Document, touched: &BTreeSet<NodeId>) {
    if !is_board(doc) || touched.is_empty() {
        return;
    }
    // One look at every layer: a board without a branch (nearly all) goes no further.
    if !doc.nodes().any(|n| matches!(&n.board, Some(BoardItem::Connector(c)) if c.branch)) {
        return;
    }
    let stirred = prune_branches(doc, before, touched);
    let bs = branches(doc);
    if bs.is_empty() {
        return;
    }
    let hit = |doc: &Document, id: NodeId| touched.contains(&id) || doc.ancestors(id).iter().any(|a| touched.contains(a));
    let (arrived, kids) = children_of(&bs);
    let roots: Vec<NodeId> = kids.keys().copied().filter(|p| !arrived.contains(p)).collect();
    for root in roots {
        let Some((boxes, lines)) = map_boxes(doc, root, &kids) else { continue };
        if boxes.iter().any(|b| !(b.rect.x0.is_finite() && b.rect.y0.is_finite() && b.rect.x1.is_finite() && b.rect.y1.is_finite())) {
            continue;
        }
        // A map holding a layer and something inside it would move that layer with its own
        // children: leave it as it is.
        let ids: BTreeSet<NodeId> = boxes.iter().map(|b| b.id).collect();
        if boxes.iter().any(|b| doc.ancestors(b.id).iter().any(|a| ids.contains(a))) {
            continue;
        }
        let stale = boxes.iter().any(|b| stirred.contains(&b.id) || hit(doc, b.id)) || lines.iter().any(|l| hit(doc, *l));
        if !stale {
            continue;
        }
        let now: BTreeMap<NodeId, Rect> = boxes.iter().map(|b| (b.id, b.rect)).collect();
        for (id, to) in mind_layout(&boxes) {
            let Some(was) = now.get(&id) else { continue };
            let d = to - was.origin();
            if d.x.is_finite() && d.y.is_finite() && d.hypot() > 1e-6 {
                shift(doc, id, Point::new(d.x, d.y));
            }
        }
    }
}
