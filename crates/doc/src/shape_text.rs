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

/// Whether layers of `kind` can hold a text.
pub fn can_host(kind: &NodeKind) -> bool {
    matches!(kind, NodeKind::Rectangle | NodeKind::Ellipse { .. } | NodeKind::Polygon { .. } | NodeKind::Star { .. } | NodeKind::Vector { .. })
}

/// The shape text `id` belongs to, when that shape is still there.
pub fn host_of(doc: &Document, id: NodeId) -> Option<NodeId> {
    let h = doc.node(id)?.text_host?;
    doc.node(h).is_some_and(|n| can_host(&n.kind)).then_some(h)
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

/// Put text `t` in `host`'s box, right above it in the same parent.
fn place(doc: &mut Document, t: NodeId, host: NodeId) {
    let Some(h) = doc.node(host) else { return };
    let Some(parent) = h.parent else { return };
    let (ix, iy) = inset(h);
    let transform = h.transform * Affine::translate((ix, iy));
    let (w, hh) = ((h.width - 2.0 * ix).max(1.0), (h.height - 2.0 * iy).max(1.0));
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
            Some(n) if !can_host(&n.kind) || n.is_page() => {
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
