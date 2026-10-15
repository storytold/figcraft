//! Mind maps on boards: a tree of topics joined by curved connectors, laid out by itself.
//!
//! Topics are rounded shapes with text, marked in their plugin data: `craftjam.mind` (on the root:
//! "root"; on the others the id of the topic they branch from) and `craftjam.order` (their place
//! among their siblings). The connectors between them are marked `craftjam.mindEdge`. Each branch
//! off the root takes a colour that its topics' borders and lines keep. After every edit the tree
//! is laid out again from where its root is ([`follow`]): children to the right of their parent,
//! stacked and centred on it. A collapsed topic (`craftjam.collapsed`) hides what branches off it.

use std::collections::{BTreeMap, BTreeSet};

use figcraft_color::{Color, Paint};
use figcraft_doc::{ConnectorEnd, ConnectorLineType, Document, Magnet, Node, NodeId, NodeKind, StrokeCap, shape_text};
use figcraft_geom::Affine;

pub use figcraft_doc::shape_text::MIND;
pub const ORDER: &str = "craftjam.order";
pub const EDGE: &str = "craftjam.mindEdge";
pub const COLLAPSED: &str = "craftjam.collapsed";

/// Between a topic and its children, and between siblings' subtrees.
pub const HGAP: f64 = 72.0;
pub const VGAP: f64 = 16.0;

/// The colours branches take, in turn.
pub const BRANCH_COLORS: [Color; 8] = [
    Color::from_rgb8_const(0x2F, 0x7F, 0xD6),
    Color::from_rgb8_const(0xE0, 0x82, 0x2B),
    Color::from_rgb8_const(0x3A, 0x9A, 0x5B),
    Color::from_rgb8_const(0x8A, 0x5C, 0xC7),
    Color::from_rgb8_const(0xD6, 0x40, 0x9F),
    Color::from_rgb8_const(0x12, 0xA5, 0x94),
    Color::from_rgb8_const(0xE5, 0x48, 0x4D),
    Color::from_rgb8_const(0xC9, 0x9A, 0x06),
];

/// The topic `n` branches from (`None` for a root or a layer that isn't a topic).
pub fn parent_of(n: &Node) -> Option<NodeId> {
    n.plugin_data.get(MIND).and_then(|v| v.parse::<u64>().ok()).map(NodeId)
}

pub fn is_topic(n: &Node) -> bool {
    n.plugin_data.contains_key(MIND)
}

pub fn is_root(n: &Node) -> bool {
    n.plugin_data.get(MIND).is_some_and(|v| v == "root")
}

pub fn is_collapsed(n: &Node) -> bool {
    n.plugin_data.contains_key(COLLAPSED)
}

fn order(n: &Node) -> u64 {
    n.plugin_data.get(ORDER).and_then(|v| v.parse().ok()).unwrap_or(0)
}

/// The topics branching from `id`, in order.
pub fn children(doc: &Document, id: NodeId) -> Vec<NodeId> {
    let Some(parent) = doc.node(id).and_then(|n| n.parent) else { return vec![] };
    let mut kids: Vec<&Node> =
        doc.node(parent).map(|p| p.children.iter().filter_map(|c| doc.node(*c)).filter(|c| parent_of(c) == Some(id)).collect()).unwrap_or_default();
    kids.sort_by_key(|c| (order(c), c.id));
    kids.into_iter().map(|c| c.id).collect()
}

/// The root of the tree `id` is in.
pub fn root_of(doc: &Document, id: NodeId) -> Option<NodeId> {
    let mut at = id;
    for _ in 0..10_000 {
        let n = doc.node(at)?;
        if is_root(n) {
            return Some(at);
        }
        at = parent_of(n)?;
    }
    None
}

/// Every topic under `id` (not it), depth first.
pub fn descendants(doc: &Document, id: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = children(doc, id);
    stack.reverse();
    while let Some(c) = stack.pop() {
        if out.contains(&c) || out.len() > 100_000 {
            continue;
        }
        out.push(c);
        let mut k = children(doc, c);
        k.reverse();
        stack.extend(k);
    }
    out
}

/// The branch colour of topic `id`: its own border's, else the next one in turn for a new branch.
fn branch_color(doc: &Document, id: NodeId) -> Option<Color> {
    let n = doc.node(id)?;
    n.strokes.iter().find_map(|p| match &p.kind {
        figcraft_color::PaintKind::Solid { color } => Some(*color),
        _ => None,
    })
}

/// A topic's look: the root dark with white words, the others white with a border in their
/// branch's colour; both pill-shaped.
fn dress(n: &mut Node, root: bool, color: Color) {
    if root {
        n.fills = vec![Paint::solid(crate::board::STRONG[0].color)];
        n.strokes.clear();
    } else {
        n.fills = vec![Paint::solid(Color::WHITE)];
        n.strokes = vec![Paint::solid(color)];
        n.stroke.weight = 2.0;
        n.stroke.align = figcraft_doc::StrokeAlign::Inside;
    }
    n.effects.clear();
}

/// A new topic `id` (its size is set by [`follow`] from its words).
pub fn new_topic(id: NodeId, parent: Option<NodeId>, order: u64, color: Color) -> Node {
    let mut n = Node::new(id, NodeKind::Rectangle, if parent.is_none() { "Mind map" } else { "Topic" }, 140.0, 44.0);
    dress(&mut n, parent.is_none(), color);
    n.corners.radii = [22.0; 4];
    n.plugin_data.insert(MIND.into(), parent.map(|p| p.0.to_string()).unwrap_or_else(|| "root".into()));
    n.plugin_data.insert(ORDER.into(), order.to_string());
    n
}

/// The curved line from `from` to topic `to`, in `color`.
pub fn new_edge(id: NodeId, from: NodeId, to: NodeId, color: Color) -> Node {
    let mut c = Node::new_connector(id, "Branch", ConnectorEnd::attached(from, Magnet::Right), ConnectorEnd::attached(to, Magnet::Left));
    if let NodeKind::Connector { line_type, .. } = &mut c.kind {
        *line_type = ConnectorLineType::Curved;
    }
    c.strokes = vec![Paint::solid(color)];
    c.stroke.weight = 2.5;
    c.stroke.cap_end = StrokeCap::None;
    c.plugin_data.insert(EDGE.into(), String::new());
    c
}

/// Add a topic branching from `parent` (after its last child) and the line to it. Returns its id.
pub fn add_child(doc: &mut Document, parent: NodeId) -> Option<NodeId> {
    let p = doc.node(parent)?;
    if !is_topic(p) {
        return None;
    }
    let container = p.parent?;
    let kids = children(doc, parent);
    let next = kids.last().and_then(|k| doc.node(*k)).map(order).map_or(0, |o| o + 1);
    // A branch off the root takes the next colour; deeper topics keep their branch's.
    let color = if is_root(p) {
        BRANCH_COLORS.get(kids.len() % BRANCH_COLORS.len()).copied().unwrap_or(BRANCH_COLORS[0])
    } else {
        branch_color(doc, parent).unwrap_or(BRANCH_COLORS[0])
    };
    let id = doc.alloc_id();
    let mut topic = new_topic(id, Some(parent), next, color);
    // Where the layout will put it, so it doesn't fly in from the corner.
    if let Some(pn) = doc.node(parent) {
        topic.transform = pn.transform * Affine::translate((pn.width + HGAP, 0.0));
    }
    doc.insert(topic, container, None).ok()?;
    let eid = doc.alloc_id();
    let page = doc.page_of(parent).unwrap_or(container);
    doc.insert(new_edge(eid, parent, id, color), page, None).ok()?;
    Some(id)
}

/// Add a topic after `id` among its siblings (a child, for the root). Returns its id.
pub fn add_sibling(doc: &mut Document, id: NodeId) -> Option<NodeId> {
    let n = doc.node(id)?;
    let Some(parent) = parent_of(n) else { return add_child(doc, id) };
    let mine = order(n);
    // Make room right after it.
    for k in children(doc, parent) {
        if let Some(kn) = doc.node_mut(k) {
            let o = order(kn);
            if o > mine {
                kn.plugin_data.insert(ORDER.into(), (o + 1).to_string());
            }
        }
    }
    let new = add_child(doc, parent)?;
    if let Some(nn) = doc.node_mut(new) {
        nn.plugin_data.insert(ORDER.into(), (mine + 1).to_string());
    }
    Some(new)
}

/// Each topic's subtree height (what's shown of it).
fn subtree_heights(doc: &Document, root: NodeId) -> BTreeMap<NodeId, f64> {
    let mut out = BTreeMap::new();
    fn walk(doc: &Document, id: NodeId, out: &mut BTreeMap<NodeId, f64>, depth: usize) -> f64 {
        let Some(n) = doc.node(id) else { return 0.0 };
        let kids = if is_collapsed(n) || depth > 512 { vec![] } else { children(doc, id) };
        let sum: f64 = kids.iter().map(|k| walk(doc, *k, out, depth + 1)).sum::<f64>() + VGAP * kids.len().saturating_sub(1) as f64;
        let h = n.height.max(sum);
        out.insert(id, h);
        h
    }
    walk(doc, root, &mut out, 0);
    out
}

/// Lay the tree under `root` out from where the root is: each topic's children to its right,
/// stacked in order and centred on it; what's under a collapsed topic hidden.
pub fn layout(doc: &mut Document, root: NodeId) {
    let heights = subtree_heights(doc, root);
    let Some(r) = doc.node(root) else { return };
    let (x0, y0) = (r.transform.translation().x, r.transform.translation().y);
    let mut stack = vec![(root, x0, y0 + r.height / 2.0, true)];
    let mut seen = BTreeSet::new();
    while let Some((id, x, cy, shown)) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(n) = doc.node(id) else { continue };
        let (w, h, collapsed) = (n.width, n.height, is_collapsed(n));
        let target = Affine::translate((x, cy - h / 2.0));
        if let Some(nm) = doc.node_mut(id)
            && (nm.transform != target || nm.visible != shown)
        {
            if id != root {
                nm.transform = target;
            }
            nm.visible = shown;
        }
        let kids = children(doc, id);
        let total: f64 = kids.iter().map(|k| heights.get(k).copied().unwrap_or(0.0)).sum::<f64>() + VGAP * kids.len().saturating_sub(1) as f64;
        let mut y = cy - total / 2.0;
        for k in kids {
            let kh = heights.get(&k).copied().unwrap_or(0.0);
            stack.push((k, x + w + HGAP, y + kh / 2.0, shown && !collapsed));
            y += kh + VGAP;
        }
    }
}

/// Size topics to their words, drop what branched off deleted topics, hide the lines into hidden
/// topics, and lay out every tree with a topic in `touched`. Returns the layers changed.
pub fn follow(doc: &mut Document, touched: &BTreeSet<NodeId>) -> Vec<NodeId> {
    if touched.is_empty() || !doc.nodes().any(is_topic) {
        return Vec::new();
    }
    let before: BTreeSet<NodeId> = doc.dirty().iter().copied().collect();
    // Topics whose parent topic is gone go too (with what branches off them), then their lines.
    loop {
        let orphans: Vec<NodeId> = doc.nodes().filter(|n| parent_of(n).is_some_and(|p| !doc.contains(p))).map(|n| n.id).collect();
        if orphans.is_empty() {
            break;
        }
        for o in orphans {
            let _ = doc.remove(o);
        }
    }
    let dead: Vec<NodeId> = doc
        .nodes()
        .filter(|n| n.plugin_data.contains_key(EDGE))
        .filter(|n| match &n.kind {
            NodeKind::Connector { start, end, .. } => [start, end].iter().any(|e| e.node.is_none_or(|t| !doc.contains(t))),
            _ => false,
        })
        .map(|n| n.id)
        .collect();
    for d in dead {
        let _ = doc.remove(d);
    }
    // The trees to lay out: those with a touched topic (or a touched text in one).
    let mut roots = BTreeSet::new();
    for t in touched {
        let id = doc.node(*t).and_then(|n| n.text_host).unwrap_or(*t);
        if doc.node(id).is_some_and(is_topic)
            && let Some(r) = root_of(doc, id)
        {
            roots.insert(r);
        }
    }
    if roots.is_empty() {
        return Vec::new();
    }
    for r in &roots {
        let mut tree = vec![*r];
        tree.extend(descendants(doc, *r));
        // Each topic as big as its words and some room.
        for id in &tree {
            let Some(t) = shape_text::text_of(doc, *id) else { continue };
            let Some((tw, th)) = doc.node(t).and_then(figcraft_text::auto_size) else { continue };
            let (w, h) = ((tw + 40.0).max(96.0), (th + 20.0).max(44.0));
            if let Some(n) = doc.node_mut(*id)
                && ((n.width - w).abs() > 1e-6 || (n.height - h).abs() > 1e-6)
            {
                n.width = w;
                n.height = h;
                n.corners.radii = [h / 2.0; 4];
            }
        }
        layout(doc, *r);
        // Words show and hide with their topics.
        for id in &tree {
            let shown = doc.node(*id).is_some_and(|n| n.visible);
            if let Some(t) = shape_text::text_of(doc, *id)
                && let Some(tn) = doc.node_mut(t)
                && tn.visible != shown
            {
                tn.visible = shown;
            }
        }
        // Lines into hidden topics hide too.
        let edges: Vec<(NodeId, bool)> = doc
            .nodes()
            .filter(|n| n.plugin_data.contains_key(EDGE))
            .filter_map(|n| match &n.kind {
                NodeKind::Connector { end, .. } => end.node.filter(|t| tree.contains(t)).map(|t| (n.id, doc.node(t).is_some_and(|x| x.visible))),
                _ => None,
            })
            .collect();
        for (e, shown) in edges {
            if let Some(n) = doc.node_mut(e)
                && n.visible != shown
            {
                n.visible = shown;
            }
        }
    }
    doc.dirty().iter().copied().filter(|i| !before.contains(i)).collect()
}
