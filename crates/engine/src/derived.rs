use super::*;

/// Nodes whose `Arc` differs from `before` (new or edited).
pub fn changed_nodes(doc: &Document, before: &Document) -> Vec<NodeId> {
    doc.ids()
        .filter(|id| match (doc.node_arc(*id), before.node_arc(*id)) {
            (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
            _ => true,
        })
        .collect()
}

/// Keep derived state consistent after edits: text boxes resize to their content, groups wrap
/// their children, instances follow their main components, auto layout re-flows.
pub fn normalize(doc: &mut Document, before: &Document) {
    normalize_with_capture(doc, before, true);
}

/// Gesture previews are local and disposable; capture their final geometry at commit.
pub(super) fn normalize_with_capture(doc: &mut Document, before: &Document, capture: bool) {
    // Only what was touched since the last normalisation (edited, added or removed).
    let mut touched: BTreeSet<NodeId> = doc.take_dirty();
    // A variable or style edit changes no layer directly, but bound and linked layers must follow it.
    if touched.is_empty() && doc.variables == before.variables && doc.styles == before.styles {
        return;
    }
    let m = measure::TextMeasure;
    // 0. Variable bindings and style links feed node values.
    let scope = binding_scope(doc, before, &touched);
    if doc.variables != before.variables || doc.styles != before.styles {
        style_vars::settle(doc);
    }
    cmd::styles::apply_bindings_to(doc, scope.as_ref());
    style_vars::refresh(doc, Some(before), scope.as_ref());
    if doc.styles != before.styles {
        let changed_styles: Vec<String> = doc.styles.iter().filter(|s| !before.styles.contains(s)).map(|s| s.id.clone()).collect();
        for sid in changed_styles {
            cmd::styles::propagate_style(doc, &sid);
        }
    }
    touched.extend(doc.take_dirty());
    // Lists and text chars edited away from their style unlink from it.
    let live_now: Vec<NodeId> = touched.iter().copied().filter(|i| doc.contains(*i)).collect();
    cmd::styles::sync_list_styles(doc, before, &live_now);
    cmd::styles::sync_text_styles(doc, &live_now);
    touched.extend(doc.take_dirty());
    let live = |doc: &Document, t: &BTreeSet<NodeId>| -> Vec<NodeId> { t.iter().copied().filter(|i| doc.contains(*i)).collect() };
    // 0b. Fill and hug only where they can apply (a layer moved out of auto layout, a parent that
    // started to hug or lost auto layout): the rest becomes fixed at its current size.
    for id in live(doc, &touched) {
        figcraft_layout::coherent_sizing(doc, id);
    }
    touched.extend(doc.take_dirty());
    // 1. Text auto-size (a sticky's note first takes the size its length calls for).
    board::settle_text(doc, before, &touched);
    touched.extend(doc.take_dirty());
    for id in live(doc, &touched) {
        let Some(n) = doc.node(id) else { continue };
        if let Some((w, h)) = figcraft_text::auto_size(n)
            && ((w - n.width).abs() > 1e-6 || (h - n.height).abs() > 1e-6)
        {
            figcraft_layout::set_size(doc, id, w, h);
        }
    }
    touched.extend(doc.take_dirty());
    // 2. Instances follow edited main components (and record overrides inside instances).
    let changed = live(doc, &touched);
    components::sync_after_edit(doc, before, &changed, &touched);
    // Rebuilt copies took their mains' values: their bindings resolve in their own modes.
    let synced = doc.take_dirty();
    if !synced.is_empty() {
        cmd::styles::apply_bindings_to(doc, Some(&synced));
        style_vars::refresh(doc, Some(before), Some(&synced));
        touched.extend(synced);
    }
    touched.extend(doc.take_dirty());
    // Parents of removed nodes (as they were before) count as touched.
    let removed_parents: Vec<NodeId> =
        touched.iter().filter(|i| !doc.contains(**i)).filter_map(|i| before.node(*i).and_then(|n| n.parent)).filter(|p| doc.contains(*p)).collect();
    // 3. Groups / booleans wrap their children.
    let mut fit: BTreeSet<NodeId> = BTreeSet::new();
    for id in live(doc, &touched).into_iter().chain(removed_parents.iter().copied()) {
        if doc.node(id).is_some_and(|n| n.kind.bounds_follow_children()) {
            fit.insert(id);
        }
        for a in doc.ancestors(id) {
            if doc.node(a).is_some_and(|n| n.kind.bounds_follow_children()) {
                fit.insert(a);
            } else {
                break;
            }
        }
    }
    let mut fit: Vec<NodeId> = fit.into_iter().collect();
    fit.sort_by_key(|id| std::cmp::Reverse(doc.ancestors(*id).len()));
    for id in fit {
        if doc.node(id).is_some_and(|n| n.children.is_empty()) {
            // Empty groups disappear, like the incumbent.
            let _ = doc.remove(id);
        } else {
            let _ = doc.fit_to_children(id);
        }
    }
    touched.extend(doc.take_dirty());
    // 4. Auto layout for frames where something that sizes or places layers changed.
    let mut frames: BTreeSet<NodeId> = BTreeSet::new();
    let moved = live(doc, &touched).into_iter().filter(|id| affects_layout(before, doc, *id));
    for id in moved.chain(removed_parents) {
        if !doc.contains(id) {
            continue;
        }
        let mut chain = vec![id];
        chain.extend(doc.ancestors(id));
        for c in chain {
            if doc.node(c).is_some_and(|n| n.layout.is_some()) {
                frames.insert(c);
            }
        }
    }
    let mut frames: Vec<NodeId> = frames.into_iter().collect();
    frames.sort_by_key(|id| std::cmp::Reverse(doc.ancestors(*id).len()));
    for f in frames {
        // Layers the layout leaves where they were stay untouched.
        let kept: Vec<(Arc<Node>, bool)> = doc
            .node(f)
            .map(|n| std::iter::once(f).chain(n.children.iter().copied()).collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|i| doc.node_arc(i).map(|a| (a.clone(), doc.dirty().contains(&i))))
            .collect();
        figcraft_layout::layout_frame(doc, f, &m);
        for (a, was_dirty) in kept {
            doc.keep_if_unchanged(a, was_dirty);
        }
    }
    touched.extend(doc.take_dirty());
    // 4b. Board connectors follow the layers they join; their labels sit at their middles.
    board::settle_routes(doc, before, &touched);
    touched.extend(doc.take_dirty());
    if capture {
        figcraft_text::appearance::capture(doc, Some(&touched.iter().copied().collect::<Vec<_>>()));
    }
    // 5. Ready-for-dev layers whose contents changed show "Changed".
    mark_dev_changes(doc, before, &touched);
    doc.take_dirty();
    // 5. Comment pins follow their layers to other pages and stay put when the layers are deleted.
    cmd::comment::follow_anchors(doc, before);
}

/// Whether layer `id` changed (since `before`) in a way that can move or resize layers in an auto
/// layout frame around it: new, resized, moved, shown or hidden, restructured, other text or
/// layout settings.
fn affects_layout(before: &Document, doc: &Document, id: NodeId) -> bool {
    let (Some(a), Some(b)) = (before.node(id), doc.node(id)) else { return true };
    a.width != b.width
        || a.height != b.height
        || a.transform != b.transform
        || a.visible != b.visible
        || a.parent != b.parent
        || a.children != b.children
        || a.layout != b.layout
        || a.child_layout != b.child_layout
        || a.stroke != b.stroke
        || a.strokes.len() != b.strokes.len()
        || a.kind != b.kind
}

/// The layers whose variable bindings an edit may have changed: every bound layer when a variable
/// changed, else the touched layers and everything below those that moved, are new, chose
/// another variable mode or became an instance of another main (modes are inherited, an
/// instance's from its main too), and the instances of a main that chose another mode.
fn binding_scope(doc: &Document, before: &Document, touched: &BTreeSet<NodeId>) -> Option<BTreeSet<NodeId>> {
    if doc.variables != before.variables {
        return None;
    }
    let main = |n: &Node| match n.kind {
        figcraft_doc::NodeKind::Instance { component, .. } => Some(component),
        _ => None,
    };
    let mut out = BTreeSet::new();
    for &id in touched {
        let Some(n) = doc.node(id) else { continue };
        let Some(b) = before.node(id) else {
            out.extend(doc.subtree(id));
            continue;
        };
        let modes = b.var_modes != n.var_modes;
        if modes || b.parent != n.parent || main(b) != main(n) {
            out.extend(doc.subtree(id));
        } else {
            out.insert(id);
        }
        if modes && matches!(n.kind, figcraft_doc::NodeKind::Component { .. }) {
            for i in components::instances_of(doc, id) {
                out.extend(doc.subtree(i));
            }
        }
    }
    Some(out)
}

/// Flag the frames and sections marked ready for dev (before this edit) that contain a layer whose
/// design `touched` changed (added, removed or edited; the handoff status itself and hiding a
/// section's contents don't count).
fn mark_dev_changes(doc: &mut Document, before: &Document, touched: &BTreeSet<NodeId>) {
    let ready = |d: &Document, id: NodeId| d.node(id).is_some_and(|n| n.dev_status == figcraft_doc::DevStatus::ReadyForDev);
    let mut changed: BTreeSet<NodeId> = BTreeSet::new();
    for &id in touched {
        // Ready-for-dev frames around it first (the cheap test).
        let mut chain = vec![id];
        chain.extend(if doc.contains(id) { doc.ancestors(id) } else { before.ancestors(id) });
        chain.retain(|m| ready(before, *m) && ready(doc, *m));
        if chain.is_empty() {
            continue;
        }
        let differs = match (before.node(id), doc.node(id)) {
            (Some(a), Some(b)) => !a.same_design(b),
            (None, None) => false,
            _ => true,
        };
        if differs {
            changed.extend(chain);
        }
    }
    for m in changed {
        if doc.node(m).is_some_and(|n| !n.dev_changed) {
            // Its first change since it was marked: keep what it looked like (for Compare changes).
            if let Some(snap) = before.snapshot_subtree(m) {
                doc.dev_snapshots.insert(m, Arc::new(snap));
            }
            if let Some(n) = doc.node_mut(m) {
                n.dev_changed = true;
            }
        }
    }
    // Snapshots last until the change is resolved, the layer is marked again or it's deleted.
    if !doc.dev_snapshots.is_empty() {
        let keep = |d: &Document, id: NodeId| d.node(id).is_some_and(|n| n.dev_status == figcraft_doc::DevStatus::ReadyForDev && n.dev_changed);
        let stale: Vec<NodeId> = doc.dev_snapshots.keys().copied().filter(|id| !keep(doc, *id)).collect();
        for id in stale {
            doc.dev_snapshots.remove(&id);
        }
    }
}
