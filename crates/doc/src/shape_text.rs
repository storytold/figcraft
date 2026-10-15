//! Shapes with text: a text layer that belongs to a shape ([`Node::text_host`]).
//!
//! The text is an ordinary text layer right above its shape, in the same parent. It is kept inside
//! the shape's box (inset so it stays within ellipses and diamonds), wraps to that width and is
//! centred both ways; resizing the shape changes the room for the text, not its size. The engine
//! calls [`follow`] after every edit, so the text moves, turns and goes away with its shape.

use crate::{Document, Node, NodeId, NodeKind};
use figcraft_geom::Affine;
use std::collections::BTreeSet;

/// The room left around a rectangle's text (px).
pub const PAD: f64 = 12.0;

/// The plugin data key that marks a rectangle as a sticky note (its value: the author's name,
/// shown at the bottom, or empty).
pub const STICKY: &str = "craftjam.sticky";

/// The room around a sticky note's text, and under it for the author's name.
pub const STICKY_PAD: f64 = 16.0;
pub const STICKY_AUTHOR: f64 = 22.0;

/// The plugin data key that marks a mind map topic (see the engine's `mindmap`): "root", or the
/// id of the topic it branches from.
pub const MIND: &str = "craftjam.mind";

/// Whether `n` is a sticky note.
pub fn is_sticky(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Rectangle) && n.plugin_data.contains_key(STICKY)
}

/// The author's name a sticky note shows (`None` when it shows none).
pub fn sticky_author(n: &Node) -> Option<&str> {
    n.plugin_data.get(STICKY).map(String::as_str).filter(|a| is_sticky(n) && !a.trim().is_empty())
}

/// Whether layers of `kind` can hold a text inside them (shapes).
pub fn can_host(kind: &NodeKind) -> bool {
    matches!(kind, NodeKind::Rectangle | NodeKind::Ellipse { .. } | NodeKind::Polygon { .. } | NodeKind::Star { .. } | NodeKind::Vector { .. })
}

/// Whether layers of `kind` can have a text of their own: shapes, inside them, and connectors, a
/// label on their line.
pub fn can_have_text(kind: &NodeKind) -> bool {
    can_host(kind) || matches!(kind, NodeKind::Connector { .. })
}

/// Where a connector's label sits: halfway along its line (in the connector's parent's space).
pub fn label_point(host: &Node) -> Option<figcraft_geom::Point> {
    let NodeKind::Connector { line_type, route, .. } = &host.kind else { return None };
    let path = crate::connector::path(*line_type, route);
    let mut pts: Vec<figcraft_geom::Point> = Vec::new();
    figcraft_geom::kurbo::flatten(path.iter(), 0.25, |el| {
        use figcraft_geom::kurbo::PathEl::*;
        match el {
            MoveTo(p) | LineTo(p) => pts.push(p),
            _ => {}
        }
    });
    let total: f64 = pts.windows(2).map(|w| (w[1] - w[0]).hypot()).sum();
    let mut left = total / 2.0;
    for w in pts.windows(2) {
        let d = (w[1] - w[0]).hypot();
        if d >= left && d > 0.0 {
            return Some(host.transform * w[0].lerp(w[1], left / d));
        }
        left -= d;
    }
    pts.first().map(|p| host.transform * *p)
}

/// The shape text `id` belongs to, when that shape is still there.
pub fn host_of(doc: &Document, id: NodeId) -> Option<NodeId> {
    let h = doc.node(id)?.text_host?;
    doc.node(h).is_some_and(|n| can_have_text(&n.kind)).then_some(h)
}

/// The text inside shape `host` (it sits next to it, in the same parent).
pub fn text_of(doc: &Document, host: NodeId) -> Option<NodeId> {
    let parent = doc.node(host)?.parent?;
    doc.node(parent)?
        .children
        .iter()
        .copied()
        .find(|c| doc.node(*c).is_some_and(|n| n.text_host == Some(host) && matches!(n.kind, NodeKind::Text(_))))
}

/// What a click on `id` selects: the shape when `id` is the text inside one.
pub fn click_target(doc: &Document, id: NodeId) -> NodeId {
    host_of(doc, id).unwrap_or(id)
}

/// The inset of a text inside `host` from each side of its box: ellipses and stars keep it
/// within their curves, diamonds and other polygons within their slanted sides.
fn inset(host: &Node) -> (f64, f64) {
    let (w, h) = (host.width, host.height);
    let (fx, fy) = match host.kind {
        NodeKind::Ellipse { .. } => (0.146, 0.146),
        NodeKind::Polygon { .. } | NodeKind::Star { .. } => (0.22, 0.22),
        // Library shapes (parallelograms, cylinders, documents…): clear of slants and rims.
        NodeKind::Vector { .. } => (0.16, 0.16),
        _ => (0.0, 0.0),
    };
    let pad = |side: f64, f: f64| (side * f).max(PAD.min(side / 4.0));
    (pad(w, fx), pad(h, fy))
}

/// Put text `t` in `host`'s box (a connector's label: centred halfway along its line), right above
/// it in the same parent.
fn place(doc: &mut Document, t: NodeId, host: NodeId) {
    let Some(h) = doc.node(host) else { return };
    let Some(parent) = h.parent else { return };
    let (transform, w, hh) = match label_point(h) {
        // A label keeps its own size (it grows with its words).
        Some(mid) => {
            let Some(tn) = doc.node(t) else { return };
            (Affine::translate((mid.x - tn.width / 2.0, mid.y - tn.height / 2.0)), tn.width, tn.height)
        }
        // A mind map topic's words keep their size, centred (the topic is sized to them).
        None if h.plugin_data.contains_key(MIND) => {
            let Some(tn) = doc.node(t) else { return };
            (h.transform * Affine::translate(((h.width - tn.width) / 2.0, (h.height - tn.height) / 2.0)), tn.width, tn.height)
        }
        // A sticky note's text starts at the top left and keeps its own height (the note grows).
        None if is_sticky(h) => {
            let Some(tn) = doc.node(t) else { return };
            (h.transform * Affine::translate((STICKY_PAD, STICKY_PAD)), (h.width - 2.0 * STICKY_PAD).max(1.0), tn.height)
        }
        None => {
            let (ix, iy) = inset(h);
            (h.transform * Affine::translate((ix, iy)), (h.width - 2.0 * ix).max(1.0), (h.height - 2.0 * iy).max(1.0))
        }
    };
    let in_layout = doc.node(parent).is_some_and(|p| p.layout.is_some());
    // Same parent, just above the shape.
    if doc.node(t).and_then(|n| n.parent) != Some(parent) {
        let _ = doc.reparent(t, parent, None);
    }
    let order_ok = doc.index_in_parent(host).zip(doc.index_in_parent(t)).is_some_and(|(a, b)| b == a + 1);
    if !order_ok && let Some(p) = doc.node_mut(parent) {
        p.children.retain(|c| *c != t);
        let at = p.children.iter().position(|c| *c == host).map(|i| i + 1).unwrap_or(p.children.len());
        p.children.insert(at.min(p.children.len()), t);
    }
    let Some(n) = doc.node(t) else { return };
    let same = n.transform.as_coeffs().iter().zip(transform.as_coeffs()).all(|(a, b)| (a - b).abs() < 1e-9)
        && (n.width - w).abs() < 1e-9
        && (n.height - hh).abs() < 1e-9
        && n.child_layout.absolute == in_layout;
    if same {
        return;
    }
    if let Some(n) = doc.node_mut(t) {
        n.transform = transform;
        n.width = w;
        n.height = hh;
        if in_layout {
            n.child_layout.absolute = true;
        }
    }
}

/// Keep shape texts with their shapes after `touched` changed: texts whose shape (or one of its
/// parents) changed follow it; texts whose shape is gone go too. Returns the texts changed.
pub fn follow(doc: &mut Document, touched: &BTreeSet<NodeId>) -> Vec<NodeId> {
    if touched.is_empty() {
        return Vec::new();
    }
    let hit = |doc: &Document, t: NodeId, h: NodeId| {
        touched.contains(&t) || touched.contains(&h) || !doc.contains(h) || doc.ancestors(h).iter().any(|a| touched.contains(a))
    };
    let texts: Vec<(NodeId, NodeId)> = doc
        .nodes()
        .filter(|n| matches!(n.kind, NodeKind::Text(_)))
        .filter_map(|n| n.text_host.map(|h| (n.id, h)))
        .filter(|(t, h)| hit(doc, *t, *h))
        .collect();
    let mut out = Vec::new();
    for (t, h) in texts {
        match doc.node(h) {
            None => {
                let _ = doc.remove(t);
            }
            Some(n) if !can_have_text(&n.kind) || n.is_page() => {
                if let Some(tn) = doc.node_mut(t) {
                    tn.text_host = None;
                }
            }
            Some(_) => place(doc, t, h),
        }
        out.push(t);
    }
    out
}

/// `ids` and the texts inside those of them that are shapes (for copies, which take a shape's
/// text along).
pub fn with_texts(doc: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let mut out = ids.to_vec();
    for i in ids {
        if let Some(t) = text_of(doc, *i)
            && !out.contains(&t)
        {
            out.push(t);
        }
    }
    out
}
