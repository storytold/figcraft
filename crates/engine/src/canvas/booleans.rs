//! Live boolean editing: a boolean group's operands stay layers of their own.
//!
//! - **Selecting operands.** A click selects the boolean group; a double click (or Cmd/Ctrl-click,
//!   or a click while a sibling operand is selected) selects the operand under the pointer, and a
//!   double click on a selected shape operand enters vector edit mode on it. Enter selects every
//!   operand. Clicks hit a boolean group where its result is (a click in a subtracted hole goes
//!   through to what's behind), except while it or a layer in it is selected: then its operands
//!   are hit, so each can be picked ([`result_outline`]).
//! - **Live results.** Moving, resizing or editing the points of an operand changes the document
//!   on every pointer move, so the group's result is redrawn during the drag (one undo step per
//!   gesture); a dragged operand stays in its group. The renderer caches every prefix of the group's fold, so only the operands from the
//!   edited one up are recombined ([`figcraft_render::boolean`]). While an operand is selected or
//!   edited, the canvas outlines every operand of its group ([`outlined_operands`]).
//! - **Per-operand operations.** Each operand above the bottom one combines with the result of the
//!   operands below it by its own operation (`Node::bool_op`), or by the group's. Choosing an
//!   operation with operands selected sets theirs; with one boolean group selected it sets the
//!   group's (and every operand follows it again); otherwise the selection is wrapped in a new
//!   boolean group ([`apply`]).

use figcraft_doc::{BoolOp, Document, Node, NodeId, NodeKind};
use figcraft_geom::BezPath;
use serde_json::{Value, json};

use crate::params::req_targets;
use crate::{Result, Session};

/// Whether `id` is a boolean group.
pub fn is_group(doc: &Document, id: NodeId) -> bool {
    doc.node(id).is_some_and(|n| matches!(n.kind, NodeKind::BooleanOp { .. }))
}

/// The boolean group `id` is an operand of.
pub fn group_of(doc: &Document, id: NodeId) -> Option<NodeId> {
    let parent = doc.node(id)?.parent?;
    is_group(doc, parent).then_some(parent)
}

/// How operand `id` combines with the operands below it: its own operation or its group's
/// (`None` outside boolean groups).
pub fn operand_op(doc: &Document, id: NodeId) -> Option<BoolOp> {
    let g = group_of(doc, id)?;
    let NodeKind::BooleanOp { op } = doc.node(g)?.kind else { return None };
    Some(doc.node(id)?.bool_op.unwrap_or(op))
}

/// An operand above the bottom one whose operation differs from its group's (what the layers
/// panel marks with the operation's icon).
pub fn distinct_op(doc: &Document, id: NodeId) -> Option<BoolOp> {
    let g = doc.node(group_of(doc, id)?)?;
    let NodeKind::BooleanOp { op } = g.kind else { return None };
    let own = doc.node(id)?.bool_op.filter(|o| *o != op)?;
    (g.children.first() != Some(&id)).then_some(own)
}

/// The operation the boolean controls show for `ids`: the group's for a lone boolean group, the
/// operands' when they share one (`None` otherwise).
pub fn shown_op(doc: &Document, ids: &[NodeId]) -> Option<BoolOp> {
    if let [one] = ids
        && let Some(NodeKind::BooleanOp { op }) = doc.node(*one).map(|n| &n.kind)
    {
        return Some(*op);
    }
    let mut ops = ids.iter().map(|i| operand_op(doc, *i));
    let first = ops.next()??;
    ops.all(|o| o == Some(first)).then_some(first)
}

/// Boolean group results for hit testing ([`figcraft_doc::geometry::BoolResult`]): none for a
/// group that is selected or holds a selected layer, whose operands are hit instead.
pub fn result_outline<'a>(doc: &'a Document, selection: &'a [NodeId]) -> impl Fn(NodeId, &Node) -> Option<BezPath> + 'a {
    move |id, n| {
        if selection.iter().any(|s| doc.is_ancestor(id, *s)) {
            return None;
        }
        figcraft_render::node_outline(doc, n).map(|(p, _)| p)
    }
}

/// The operands to outline on the canvas while layers inside a boolean group are selected or
/// edited (`editing`: the layer in vector edit mode): every operand of the nearest boolean group
/// holding each of them, in layer order, without repeats.
pub fn outlined_operands(doc: &Document, selection: &[NodeId], editing: Option<NodeId>) -> Vec<NodeId> {
    let mut out = Vec::new();
    for id in selection.iter().copied().chain(editing) {
        let Some(g) = doc.ancestors(id).into_iter().find(|a| is_group(doc, *a)) else { continue };
        for c in doc.node(g).map(|n| n.children.clone()).unwrap_or_default() {
            if doc.node(c).is_some_and(|n| n.visible) && !out.contains(&c) {
                out.push(c);
            }
        }
    }
    out
}

/// `bool.union` / `bool.subtract` / `bool.intersect` / `bool.exclude`: see the module docs.
pub(crate) fn apply(s: &mut Session, cmd: &str, p: &Value, op: BoolOp) -> Result<Value> {
    let ids = req_targets(cmd, s, p)?;
    let doc = s.doc()?;
    let lone_group = match ids.as_slice() {
        [one] if is_group(doc, *one) => Some(*one),
        _ => None,
    };
    if let Some(g) = lone_group {
        let doc = s.doc_mut()?;
        if let NodeKind::BooleanOp { op: o } = &mut doc.get_mut(g)?.kind {
            *o = op;
        }
        doc.get_mut(g)?.name = renamed(&doc.get(g)?.name, op);
        for c in doc.get(g)?.children.clone() {
            if doc.get(c)?.bool_op.is_some() {
                doc.get_mut(c)?.bool_op = None;
            }
        }
        return Ok(json!({"id": g, "operation": op}));
    }
    if ids.iter().all(|i| group_of(doc, *i).is_some()) {
        let doc = s.doc_mut()?;
        for i in &ids {
            let group_op = match group_of(doc, *i).and_then(|g| doc.node(g)).map(|g| &g.kind) {
                Some(NodeKind::BooleanOp { op }) => *op,
                _ => continue,
            };
            doc.get_mut(*i)?.bool_op = (op != group_op).then_some(op);
        }
        return Ok(json!({"ids": ids, "operation": op}));
    }
    let out = crate::cmd::structure::wrap_boolean(s, p, cmd, op)?;
    // Operands of a new group follow its operation.
    if let Some(g) = crate::params::id_of(&out["id"]) {
        let doc = s.doc_mut()?;
        for c in doc.get(g)?.children.clone() {
            if doc.get(c)?.bool_op.is_some() {
                doc.get_mut(c)?.bool_op = None;
            }
        }
    }
    Ok(out)
}

/// A boolean group's default name follows its operation; other names stay.
fn renamed(name: &str, op: BoolOp) -> String {
    let default = [BoolOp::Union, BoolOp::Subtract, BoolOp::Intersect, BoolOp::Exclude].iter().any(|o| o.label() == name);
    if default { op.label().to_string() } else { name.to_string() }
}
