use super::diff::geometry_derived;
use super::rebase::rebase_instance;
use super::variants::instance_issues;
use super::{
    ContentAlign, CrossAlign, Document, MainAlign, Node, NodeId, NodeKind, SLOT, Sizing, enclosing_instance, inside_instance, is_instance,
    owning_instance, structure_locked,
};

/// Slot `slot` (inside an instance) takes the instance's own content: the layers it shows (the
/// main's default content until now) stop mirroring the main, and instances among them stand
/// alone, linked to their own mains.
pub(super) fn own_slot_content(doc: &mut Document, slot: NodeId) {
    if let Some(n) = doc.node_mut(slot) {
        n.overrides.insert(SLOT.into());
    }
    for nested in unlink_layers(doc, slot, None) {
        rebase_instance(doc, nested);
    }
}

/// Unlink the layers below `top` that are layers of `owner` (an instance, or `None` for the
/// content of a slot): plain layers forget the main layer they mirrored and their property
/// bindings. The instances among them are returned, to re-link to their own mains (with
/// [`rebase_instance`]) once the caller is done.
pub fn unlink_layers(doc: &mut Document, top: NodeId, owner: Option<NodeId>) -> Vec<NodeId> {
    let picked: Vec<(NodeId, bool)> =
        doc.subtree(top).into_iter().filter(|d| *d != top && enclosing_instance(doc, *d) == owner).map(|d| (d, is_instance(doc, d))).collect();
    let mut nested = Vec::new();
    for (d, instance) in picked {
        let Some(n) = doc.node_mut(d) else { continue };
        n.prop_refs.clear();
        if instance {
            nested.push(d);
        } else {
            n.source = None;
            n.overrides.clear();
        }
    }
    nested
}

/// A new layer copied from inside an instance (pasted, duplicated) to where no instance owns it
/// stops mirroring the main; a copied nested instance is re-linked to its own main (its look kept).
pub(super) fn relink_copy(doc: &mut Document, id: NodeId) {
    let Some(n) = doc.node(id) else { return };
    match &n.kind {
        NodeKind::Instance { component, .. } => {
            let main_ok = doc.node(*component).is_some_and(|m| matches!(m.kind, NodeKind::Component { .. }));
            if main_ok && !inside_instance(doc, id) && (n.source != Some(*component) || !instance_issues(doc, id).is_empty()) {
                rebase_instance(doc, id);
            }
        }
        _ => {
            if (n.source.is_some() || !n.overrides.is_empty())
                && owning_instance(doc, id).is_none()
                && let Some(m) = doc.node_mut(id)
            {
                m.source = None;
                m.overrides.clear();
            }
        }
    }
}

/// Refuse edits that add, remove or reorder layers inside an instance (only its main can), outside
/// its slots.
pub fn check_structure_edit(before: &Document, after: &Document) -> Result<(), String> {
    for &id in after.dirty() {
        let (Some(n), Some(b)) = (after.node(id), before.node(id)) else { continue };
        // Library copies change only through library updates (and are read-only otherwise).
        if n.children == b.children || after.is_remote(id) {
            continue;
        }
        if structure_locked(before, id) && structure_locked(after, id) {
            let name = owning_instance(after, id).and_then(|i| after.node(i)).map(|i| i.name.clone()).unwrap_or_default();
            return Err(format!(
                "layers can't be added, removed or reordered inside the instance \"{name}\": edit its main component, or detach the instance first"
            ));
        }
    }
    Ok(())
}

/// Refuse edits an instance doesn't take as overrides ([`refused_override`]); the edit is rolled
/// back.
pub fn check_override_edit(before: &Document, after: &Document) -> Result<(), String> {
    match after.dirty().iter().find_map(|id| refused_override(before, after, *id)) {
        Some(what) => Err(format!("{what} comes from the main component in an instance: edit the main component, or detach the instance")),
        None => Ok(()),
    }
}

/// What about layer `id` an edit from `before` to `after` changed that only its main can: the
/// auto layout of an instance or a layer in one but its padding, gap and alignment, or the
/// position (rotation too), constraints or place in auto layout (absolute, grid cell) of a layer
/// inside an instance (its size and resizing are overrides). `None` when the edit is fine, also
/// when the main changed the same way in this edit (a library update).
pub fn refused_override(before: &Document, after: &Document, id: NodeId) -> Option<&'static str> {
    let (n, b) = (after.node(id)?, before.node(id)?);
    if after.is_remote(id) || n.source != b.source || owning_instance(before, id).is_none() || owning_instance(after, id).is_none() {
        return None;
    }
    let (mb, ma) = (before.node(n.source?)?, after.node(n.source?)?);
    let mode = |x: &Node| x.layout.as_ref().map(|l| l.mode);
    // Auto layout settings other than the overridable ones.
    let settings = |x: &Node| {
        x.layout.clone().map(|mut l| {
            (l.gap, l.cross_gap, l.padding, l.main_align, l.cross_align) = (0.0, 0.0, [0.0; 4], MainAlign::Min, CrossAlign::Min);
            l.content_align = ContentAlign::Auto;
            l
        })
    };
    // Child layout settings other than the resizing ones.
    let placing = |x: &Node| {
        let mut c = x.child_layout.clone();
        (c.sizing_h, c.sizing_v, c.min_w, c.max_w, c.min_h, c.max_h) = (Sizing::Fixed, Sizing::Fixed, None, None, None, None);
        c
    };
    if mode(n) != mode(b) && mode(ma) == mode(mb) {
        return Some("The auto layout direction");
    }
    if settings(n) != settings(b) && settings(ma) == settings(mb) {
        return Some("This auto layout setting");
    }
    if !inside_instance(after, id) || !inside_instance(before, id) {
        return None;
    }
    if n.transform != b.transform && ma.transform == mb.transform && !geometry_derived(after, before, id, "position") {
        return Some("A layer's position");
    }
    if n.constraints != b.constraints && ma.constraints == mb.constraints {
        return Some("A layer's constraints");
    }
    if placing(n) != placing(b) && placing(ma) == placing(mb) {
        return Some("A layer's place in auto layout");
    }
    None
}
