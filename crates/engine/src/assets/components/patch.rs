use super::diff::{binding_group, bindings_of, diff_groups, follow_boolean_defaults, geometry_derived, variable_groups};
use super::guard::{own_slot_content, relink_copy};
use super::instances::{copy_group, field_group, sync_instance};
use super::sync::insert_linked;
use super::{
    BTreeMap, BTreeSet, Document, MAX_DEPTH, Node, NodeId, NodeKind, default_instance_name, holds_content, inside_instance, owning_component,
    owning_instance, root_instance, scope, slot_edited, slot_scope,
};

type OverrideGroups = (Vec<&'static str>, Vec<&'static str>, Vec<&'static str>);

/// After an edit: record overrides inside instances, then rebuild instances whose main changed,
/// whose properties or main were switched, or whose overrides were reset.
pub fn sync_after_edit(doc: &mut Document, before: &Document, changed: &[NodeId], touched: &BTreeSet<NodeId>) {
    let mut changed = changed.to_vec();
    for id in follow_boolean_defaults(doc, before, &changed) {
        if !changed.contains(&id) {
            changed.push(id);
        }
    }
    let changed = changed.as_slice();
    adopt_slot_content(doc, before, changed);
    let mut resync: BTreeSet<NodeId> = BTreeSet::new();
    record_overrides(doc, before, changed, &mut resync);
    // 2. Components whose layers changed, were added or removed. Value-only edits (paint, text,
    //    visibility, …) are carried to the copies directly; anything else rebuilds the instances.
    let mut comps: BTreeSet<NodeId> = BTreeSet::new();
    let mut patches: Vec<(NodeId, Vec<&'static str>)> = Vec::new();
    // Main layers that only gained new layers or lost some: their copies take the same change.
    let mut restructured: Vec<NodeId> = Vec::new();
    let mut covered: BTreeSet<NodeId> = BTreeSet::new();
    for &id in changed {
        if let (Some(b), Some(n)) = (before.node(id), doc.node(id))
            && b.children != n.children
            && owning_instance(doc, id).is_none()
            && owning_component(doc, id).is_some()
            && let Some(layers) = restructurable(doc, before, b, n)
        {
            restructured.push(id);
            covered.extend(layers);
        }
    }
    for &id in changed {
        if covered.contains(&id) || restructured.contains(&id) {
            continue;
        }
        let Some(c) = owning_component(doc, id) else { continue };
        let (Some(b), Some(n)) = (before.node(id), doc.node(id)) else {
            comps.insert(c);
            continue;
        };
        if b == n {
            continue;
        }
        match patchable(doc, b, n) {
            Some(groups) => patches.push((id, groups)),
            None => {
                comps.insert(c);
            }
        }
    }
    for &id in touched {
        if !doc.contains(id)
            && !covered.contains(&id)
            && let Some(c) = owning_component(before, id)
            && doc.contains(c)
        {
            comps.insert(c);
        }
    }
    for r in resync {
        if doc.contains(r) {
            sync_instance(doc, r);
            if let Some(c) = owning_component(doc, r) {
                comps.insert(c);
            }
        }
    }
    // A copy the patch can't follow exactly rebuilds instead (as does any other change).
    let patched = comps.is_empty()
        && !full_rebuild()
        && (restructured.is_empty() || patch_structure(doc, before, &restructured))
        && (patches.is_empty() || patch_copies(doc, &patches));
    #[cfg(test)]
    if patched && (!restructured.is_empty() || !patches.is_empty()) {
        PATCHED.with(|c| c.set(c.get() + 1));
    }
    if !patched {
        comps.extend(patches.iter().map(|(id, _)| *id).chain(restructured.iter().copied()).filter_map(|id| owning_component(doc, id)));
    }
    // 3. Instances of changed components follow; an instance inside another component makes that
    //    one change too.
    rebuild_instances(doc, &comps);
}

/// Slot content changes: layers added, removed or reordered in a slot become the instance's own.
fn adopt_slot_content(doc: &mut Document, before: &Document, changed: &[NodeId]) {
    // 0. A slot whose layers were added, removed or reordered holds the instance's own content
    //    from now on; copies of instance layers placed elsewhere stop mirroring their main.
    for &id in changed {
        let (Some(n), Some(b)) = (doc.node(id), before.node(id)) else { continue };
        if n.children == b.children || owning_instance(doc, id).is_none() {
            continue;
        }
        if let Some(slot) = slot_scope(doc, id)
            && !slot_edited(doc, slot)
        {
            own_slot_content(doc, slot);
        }
    }
    for &id in changed {
        if !before.contains(id) {
            relink_copy(doc, id);
        }
    }
}

/// Overrides for layers edited inside instances; the instances to rebuild go into `resync`.
fn record_overrides(doc: &mut Document, before: &Document, changed: &[NodeId], resync: &mut BTreeSet<NodeId>) {
    for &id in changed {
        let (Some(n), Some(b)) = (doc.node(id), before.node(id)) else { continue };
        let Some(inst) = owning_instance(doc, id) else { continue };
        let top_root = inst == id && !inside_instance(doc, id);
        if top_root
            && let (NodeKind::Instance { component: c1, props: p1 }, NodeKind::Instance { component: c2, props: p2 }) = (&n.kind, &b.kind)
            && (c1 != c2 || p1 != p2)
        {
            // Switching the main or a property value of an instance isn't an override.
            resync.insert(id);
        }
        let (raw, groups, cleared) = override_groups(doc, before, id, n, b, inst, top_root);
        let lost = b.overrides.iter().any(|g| !n.overrides.contains(g));
        let nested_switch = !top_root && raw.iter().any(|g| matches!(*g, "component" | "props"));
        if (!groups.is_empty() || !cleared.is_empty())
            && let Some(m) = doc.node_mut(id)
        {
            for g in groups {
                m.overrides.insert(g.to_string());
            }
            for g in cleared {
                m.overrides.remove(g);
            }
        }
        if (lost || nested_switch)
            && let Some(r) = root_instance(doc, id)
        {
            resync.insert(r);
        }
    }
}

/// `(raw, groups, cleared)` of layer `id` edited from `b` to `n`: the raw changed groups, the ones
/// that are now overrides and the overrides that were set back to the main's value.
fn override_groups(doc: &Document, before: &Document, id: NodeId, n: &Node, b: &Node, inst: NodeId, top_root: bool) -> OverrideGroups {
    let raw = diff_groups(b, n);
    let mut groups = raw.clone();
    // An instance's own place, resizing, export settings, interactions and variable modes are
    // its own, not overrides.
    if top_root {
        groups.retain(|g| !matches!(*g, "component" | "props" | "position" | "resizing" | "exports" | "interactions" | "modes"));
    }
    if groups.iter().any(|g| matches!(*g, "size" | "position")) {
        groups.retain(|g| !matches!(*g, "size" | "position") || !geometry_derived(doc, before, id, g));
    }
    // A value equal to the main's isn't an override (setting it back clears the override).
    let mut cleared = Vec::new();
    if let Some(src) = n.source.and_then(|s| doc.node(s)) {
        let mut differs = diff_groups(src, n);
        if inst == id {
            differs.retain(|g| *g != "name");
            if n.name != default_instance_name(doc, src.id) {
                differs.push("name");
            }
        }
        // A value from the same variable as the main's (resolved in this layer's modes) isn't an
        // override either.
        let from_variables = variable_groups(n, src);
        groups.retain(|g| differs.contains(g) && !from_variables.contains(g));
        // Binding another variable (or none) where the main binds one is an override.
        for (k, _) in b.bound.iter().chain(n.bound.iter()) {
            let Some(g) = binding_group(k) else { continue };
            let rebound = b.bound.get(k) != n.bound.get(k) && !bindings_of(n, g).eq(bindings_of(src, g));
            if rebound && !groups.contains(&g) {
                groups.push(g);
            }
        }
        cleared = raw.iter().filter(|g| !matches!(**g, "size" | "position") && !differs.contains(g) && n.overrides.contains(**g)).copied().collect();
        if top_root {
            cleared.retain(|g| !matches!(*g, "component" | "props"));
        }
    }
    (raw, groups, cleared)
}

#[cfg(test)]
thread_local! {
    /// Tests: rebuild instances even for edits that could be patched (to compare both ways).
    pub(crate) static FULL_REBUILD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Tests: edits carried to the copies without rebuilding instances.
    pub(crate) static PATCHED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn full_rebuild() -> bool {
    FULL_REBUILD.with(std::cell::Cell::get)
}

#[cfg(not(test))]
fn full_rebuild() -> bool {
    false
}

/// The property groups of a main-component layer edited from `b` to `n` when the edit changed
/// values only (paint, text, visibility, size, position, …) that copies take as they are: carried to them without
/// rebuilding the instances ([`patch_copies`]). `None` when anything else changed.
fn patchable(doc: &Document, b: &Node, n: &Node) -> Option<Vec<&'static str>> {
    const VALUES: [&str; 17] = [
        "fills",
        "strokes",
        "effects",
        "opacity",
        "visible",
        "name",
        "corners",
        "characters",
        "textStyle",
        "padding",
        "gap",
        "alignment",
        "grids",
        "exports",
        "interactions",
        "size",
        "position",
    ];
    if b.children != n.children || b.parent != n.parent || b.kind.type_name() != n.kind.type_name() {
        return None;
    }
    let groups = diff_groups(b, n);
    if groups.is_empty() || groups.iter().any(|g| !VALUES.contains(g)) {
        return None;
    }
    // Exactly those groups (and the layer's own overrides) changed.
    let mut probe = n.clone();
    for g in &groups {
        copy_group(b, &mut probe, g);
    }
    probe.overrides = b.overrides.clone();
    if probe != *b {
        return None;
    }
    // Values a component property sets (on this layer or a main it mirrors) come from instances.
    let mut cur = Some(n);
    for _ in 0..=MAX_DEPTH {
        let Some(x) = cur else { break };
        if x.prop_refs.keys().any(|f| groups.contains(&field_group(f))) {
            return None;
        }
        cur = x.source.and_then(|s| doc.node(s));
    }
    Some(groups)
}

/// Copies by the layer they mirror; nested instances swapped to a main also follow it.
fn copies_index(doc: &Document) -> BTreeMap<NodeId, Vec<NodeId>> {
    let mut copies: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    for n in doc.nodes() {
        let Some(s) = n.source else { continue };
        copies.entry(s).or_default().push(n.id);
        if let (NodeKind::Instance { component, .. }, Some(NodeKind::Instance { component: other, .. })) = (&n.kind, doc.node(s).map(|x| &x.kind))
            && component != other
        {
            copies.entry(*component).or_default().push(n.id);
        }
    }
    copies
}

/// When main layer `b` → `n` only gained new plain layers (created in this edit: no instances,
/// components or property bindings in them) or lost layers that were deleted: those layers, which
/// [`patch_structure`] carries to the copies. `None` for anything else.
fn restructurable(doc: &Document, before: &Document, b: &Node, n: &Node) -> Option<Vec<NodeId>> {
    let mut probe = n.clone();
    probe.children = b.children.clone();
    probe.overrides = b.overrides.clone();
    if probe != *b {
        return None;
    }
    let mut layers = Vec::new();
    for c in n.children.iter().filter(|c| !b.children.contains(c)) {
        for d in doc.subtree(*c) {
            let x = doc.node(d)?;
            let plain = !matches!(x.kind, NodeKind::Instance { .. } | NodeKind::Component { .. } | NodeKind::ComponentSet { .. });
            if before.contains(d) || !plain || !x.prop_refs.is_empty() {
                return None;
            }
            layers.push(d);
        }
    }
    for c in b.children.iter().filter(|c| !n.children.contains(c)) {
        if doc.contains(*c) {
            return None;
        }
        layers.extend(before.subtree(*c));
    }
    Some(layers)
}

/// Carry the new and deleted layers of main layers `seeds` ([`restructurable`]) to every copy,
/// through copies of copies (slots holding an instance's own content keep it). Returns false
/// (leaving a partial patch for the caller to rebuild over) when a copy can't follow exactly.
fn patch_structure(doc: &mut Document, before: &Document, seeds: &[NodeId]) -> bool {
    let copies = copies_index(doc);
    let mut work: Vec<NodeId> = seeds.to_vec();
    let mut steps = 0usize;
    while let Some(p) = work.pop() {
        steps += 1;
        if steps > 10_000_000 {
            return false;
        }
        let Some(pn) = doc.node(p).cloned() else { continue };
        for c in copies.get(&p).cloned().unwrap_or_default() {
            let Some(cn) = doc.node(c).cloned() else { continue };
            if holds_content(&cn) {
                continue;
            }
            if let (NodeKind::Instance { component: a, .. }, NodeKind::Instance { component: b, .. }) = (&pn.kind, &cn.kind)
                && a != b
            {
                continue;
            }
            // The copy's layers by the layer of `p` they mirror.
            let mut mirrors: BTreeMap<NodeId, NodeId> = BTreeMap::new();
            for k in &cn.children {
                let Some(src) = doc.node(*k).and_then(|x| x.source) else { return false };
                if mirrors.insert(src, *k).is_some() {
                    return false;
                }
            }
            let mut order = Vec::with_capacity(pn.children.len());
            for pc in &pn.children {
                if let Some(k) = mirrors.remove(pc) {
                    order.push(k);
                    continue;
                }
                // A layer the copy lacks must be new in this edit, placed as in the main.
                let absolute = doc.node(*pc).is_some_and(|x| x.child_layout.absolute);
                if before.contains(*pc) || !child_follows_main(doc, c, absolute) {
                    return false;
                }
                match copy_layers(doc, *pc, c) {
                    Some(k) => order.push(k),
                    None => return false,
                }
            }
            // Copies of layers the main lost.
            for (src, k) in mirrors {
                if doc.contains(src) || doc.remove(k).is_err() {
                    return false;
                }
            }
            if doc.node(c).is_some_and(|x| x.children != order)
                && let Some(m) = doc.node_mut(c)
            {
                m.children = order;
            }
            work.push(c);
        }
    }
    true
}

/// Whether a layer placed in copy `parent` (absolutely positioned or not) takes its main layer's
/// place and size: nothing on the way up applies constraints ([`follows_main_geometry`]).
fn child_follows_main(doc: &Document, parent: NodeId, absolute: bool) -> bool {
    let Some(p) = doc.node(parent) else { return false };
    let constrains = p.layout.is_none() || absolute;
    let resized = sized_own(p)
        || match &p.kind {
            NodeKind::Instance { component, .. } => {
                doc.node(*component).is_none_or(|m| (m.width - p.width).abs() > 1e-9 || (m.height - p.height).abs() > 1e-9)
            }
            _ => false,
        };
    !(constrains && resized) && follows_main_geometry(doc, parent)
}

/// Copy main layer `src` (with its layers) into copy `parent` as layers mirroring them; returns the
/// new copy (the caller places it among `parent`'s layers).
fn copy_layers(doc: &mut Document, src: NodeId, parent: NodeId) -> Option<NodeId> {
    let sub = doc.subtree(src);
    let map: BTreeMap<NodeId, NodeId> = sub.iter().map(|d| (*d, doc.alloc_id())).collect();
    for d in &sub {
        let mut n = doc.node(*d)?.clone();
        n.id = *map.get(d)?;
        n.parent = if *d == src { Some(parent) } else { n.parent.and_then(|x| map.get(&x).copied()) };
        n.children = n.children.iter().filter_map(|k| map.get(k).copied()).collect();
        n.source = Some(*d);
        n.overrides.clear();
        n.prop_refs.clear();
        n.annotations.clear();
        n.measurements.clear();
        insert_linked(doc, n).ok()?;
    }
    map.get(&src).copied()
}

/// Carry value-only edits of main-component layers (`seeds`: layer, groups) to every copy, through
/// copies of copies, except where a copy overrides the group. Returns false (leaving a partial
/// patch for the caller to rebuild over) when a copy can't follow exactly.
fn patch_copies(doc: &mut Document, seeds: &[(NodeId, Vec<&'static str>)]) -> bool {
    let copies = copies_index(doc);
    let text = |g: &str| matches!(g, "characters" | "textStyle");
    let mut work: Vec<(NodeId, Vec<&'static str>)> = seeds.to_vec();
    let mut steps = 0usize;
    while let Some((p, groups)) = work.pop() {
        steps += 1;
        if steps > 10_000_000 {
            return false;
        }
        let Some(pn) = doc.node(p).cloned() else { continue };
        for c in copies.get(&p).cloned().unwrap_or_default() {
            let Some(cn) = doc.node(c) else { continue };
            // A swapped nested instance shows its own main, not the layer it stands in for.
            if let (NodeKind::Instance { component: a, .. }, NodeKind::Instance { component: b, .. }) = (&pn.kind, &cn.kind)
                && a != b
            {
                continue;
            }
            // An instance keeps its own visibility, place, export settings and interactions; its
            // name follows the main's unless renamed; a resized one lays its layers out against
            // the main's size.
            let root = matches!(pn.kind, NodeKind::Component { .. });
            if root && ((groups.contains(&"name") && !cn.overrides.contains("name")) || (groups.contains(&"size") && cn.overrides.contains("size"))) {
                return false;
            }
            let gs: Vec<&'static str> = groups
                .iter()
                .copied()
                .filter(|g| !(cn.overrides.contains(*g) || root && matches!(*g, "visible" | "name" | "position" | "exports" | "interactions")))
                .collect();
            if gs.is_empty() {
                continue;
            }
            let geometry = gs.iter().any(|g| matches!(*g, "size" | "position"));
            if geometry && !follows_main_geometry(doc, c) {
                return false;
            }
            // Text content and style share their runs: a copy overriding one keeps its runs when the
            // other follows the main (as a rebuild carries them).
            let runs = match &cn.kind {
                NodeKind::Text(t) if gs.iter().any(|g| text(g)) && cn.overrides.iter().any(|g| text(g)) => Some(t.runs.clone()),
                _ => None,
            };
            if let Some(m) = doc.node_mut(c) {
                for g in &gs {
                    copy_group(&pn, m, g);
                }
                if let (Some(r), NodeKind::Text(t)) = (runs, &mut m.kind) {
                    t.runs = r;
                }
            }
            if (geometry || gs.iter().any(|g| text(g)))
                && let Some((w, h)) = doc.node(c).and_then(figcraft_text::auto_size)
                && doc.node(c).is_some_and(|n| (n.width - w).abs() > 1e-6 || (n.height - h).abs() > 1e-6)
            {
                figcraft_layout::set_size(doc, c, w, h);
            }
            work.push((c, gs));
        }
    }
    true
}

/// Whether a copy's size and place are its main layer's: no layer around it inside its instances
/// applies constraints on the way down to it (a resized instance or frame without auto layout, or
/// a resized auto layout instance holding it absolutely positioned). Auto layout re-flows the rest.
fn follows_main_geometry(doc: &Document, id: NodeId) -> bool {
    let chain = scope(doc, id);
    chain.windows(2).all(|w| {
        let (Some(below), Some(a)) = (w.first().and_then(|i| doc.node(*i)), w.get(1).and_then(|i| doc.node(*i))) else { return false };
        let constrains = a.layout.is_none() || below.child_layout.absolute;
        if sized_own(a) && constrains {
            return false;
        }
        match &a.kind {
            NodeKind::Instance { component, .. } => {
                !constrains || doc.node(*component).is_some_and(|m| (m.width - a.width).abs() <= 1e-9 && (m.height - a.height).abs() <= 1e-9)
            }
            _ => true,
        }
    })
}

/// Whether a layer inside an instance overrides its size or resizing: it may not have its main
/// layer's size, so the layers in it can't take their main layers' places as they are.
fn sized_own(n: &Node) -> bool {
    n.overrides.contains("size") || n.overrides.contains("resizing")
}

/// Rebuild every instance of `comps` (and of the components that hold those instances, and so on
/// outwards), each outermost instance once, inner components first.
pub(super) fn rebuild_instances(doc: &mut Document, comps: &BTreeSet<NodeId>) {
    if comps.is_empty() {
        return;
    }
    // Outermost instances by the components their layers instantiate.
    let mut roots_of: BTreeMap<NodeId, BTreeSet<NodeId>> = BTreeMap::new();
    for n in doc.nodes() {
        if let NodeKind::Instance { component, .. } = &n.kind {
            roots_of.entry(*component).or_default().insert(root_instance(doc, n.id).unwrap_or(n.id));
        }
    }
    // Affected components, with how deep their dependencies go (an instance inside component X of
    // component Y: X follows Y).
    let mut level: BTreeMap<NodeId, usize> = comps.iter().map(|c| (*c, 0)).collect();
    let mut outer: BTreeMap<NodeId, Option<NodeId>> = BTreeMap::new();
    let mut frontier: Vec<NodeId> = comps.iter().copied().collect();
    let mut rounds = 0;
    while !frontier.is_empty() && rounds < 64 {
        rounds += 1;
        let mut next = Vec::new();
        for c in frontier {
            let lc = level.get(&c).copied().unwrap_or(0);
            for r in roots_of.get(&c).into_iter().flatten() {
                let o = *outer.entry(*r).or_insert_with(|| owning_component(doc, *r));
                if let Some(x) = o
                    && x != c
                {
                    let lx = level.entry(x).or_insert(0);
                    if *lx < lc + 1 {
                        *lx = lc + 1;
                        next.push(x);
                    }
                }
            }
        }
        frontier = next;
    }
    // Instances inside components by level, then the rest (each once).
    let mut order: Vec<(usize, NodeId)> = outer.iter().map(|(r, o)| (o.and_then(|x| level.get(&x).copied()).unwrap_or(usize::MAX), *r)).collect();
    order.sort();
    for (_, r) in order {
        if doc.contains(r) {
            sync_instance(doc, r);
        }
    }
}
