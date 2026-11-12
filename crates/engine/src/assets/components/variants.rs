use super::{BTreeMap, Document, MAX_DEPTH, Node, NodeId, NodeKind, holds_content, inside_instance};

/// A set's default variant: the one in its top-left corner (nearest to it, then the top-most, then
/// the left-most).
pub fn default_variant(doc: &Document, set: NodeId) -> Option<NodeId> {
    let corner = |c: &NodeId| {
        doc.node(*c).filter(|n| matches!(n.kind, NodeKind::Component { .. })).map(|n| n.transform.as_coeffs()).map(|[.., x, y]| (x + y, y, x))
    };
    let kids = &doc.node(set)?.children;
    kids.iter()
        .copied()
        .filter_map(|c| corner(&c).map(|k| (k, c)))
        .min_by(|(a, _), (b, _)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, c)| c)
}

/// Variants of a set sharing one combination of property values (each group two or more, in
/// layer order): a conflict to resolve by giving each a unique combination.
pub fn variant_conflicts(doc: &Document, set: NodeId) -> Vec<Vec<NodeId>> {
    let mut by: BTreeMap<BTreeMap<String, String>, Vec<NodeId>> = BTreeMap::new();
    for c in doc.node(set).map(|s| s.children.clone()).unwrap_or_default() {
        if let Some(n) = doc.node(c).filter(|n| matches!(n.kind, NodeKind::Component { .. })) {
            by.entry(variant_props(&n.name)).or_default().push(c);
        }
    }
    by.into_values().filter(|v| v.len() > 1).collect()
}

/// Parse a variant name "Size=Large, State=Hover" into pairs.
pub fn variant_props(name: &str) -> BTreeMap<String, String> {
    name.split(',')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

/// Format variant pairs as a layer name, in the given property order (others after, sorted).
pub fn variant_name(pairs: &BTreeMap<String, String>, order: &[String]) -> String {
    let mut keys: Vec<&String> = order.iter().filter(|k| pairs.contains_key(*k)).collect();
    keys.extend(pairs.keys().filter(|k| !order.contains(k)));
    keys.iter().filter_map(|k| pairs.get(*k).map(|v| format!("{k}={v}"))).collect::<Vec<_>>().join(", ")
}

/// Variant properties of a set: property → values, in first-seen order.
pub fn variant_options(doc: &Document, set: NodeId) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for c in doc.node(set).map(|s| s.children.clone()).unwrap_or_default() {
        let Some(cn) = doc.node(c) else { continue };
        for kv in cn.name.split(',') {
            let Some((k, v)) = kv.split_once('=') else { continue };
            let (k, v) = (k.trim().to_string(), v.trim().to_string());
            match out.iter_mut().find(|(p, _)| *p == k) {
                Some((_, vals)) => {
                    if !vals.contains(&v) {
                        vals.push(v);
                    }
                }
                None => out.push((k, vec![v])),
            }
        }
    }
    out
}

/// The variant in `set` matching `wanted` best: all given properties equal, or else the most.
pub fn find_variant(doc: &Document, set: NodeId, wanted: &BTreeMap<String, String>) -> Option<NodeId> {
    let s = doc.node(set)?;
    let score = |c: &NodeId| doc.node(*c).map(|n| variant_props(&n.name)).map(|p| wanted.iter().filter(|(k, v)| p.get(*k) == Some(*v)).count());
    if let Some(exact) = s.children.iter().copied().find(|c| score(c) == Some(wanted.len())) {
        return Some(exact);
    }
    s.children.iter().copied().filter(|c| score(c).is_some()).max_by_key(|c| score(c).unwrap_or(0))
}

/// Check that every instance mirrors its main's layer structure (tests and diagnostics): each
/// layer's `source` is the main layer at the same place, swapped nested instances mirror their
/// own main, slots holding the instance's own content aren't compared. Returns the problems found.
pub fn mirror_issues(doc: &Document) -> Vec<String> {
    doc.nodes()
        .filter(|n| matches!(n.kind, NodeKind::Instance { .. }) && !inside_instance(doc, n.id))
        .flat_map(|n| instance_issues(doc, n.id))
        .collect()
}

/// [`mirror_issues`] for one instance.
pub(super) fn instance_issues(doc: &Document, id: NodeId) -> Vec<String> {
    fn walk(doc: &Document, inst: &Node, main: &Node, depth: usize, out: &mut Vec<String>) {
        if depth > MAX_DEPTH || holds_content(inst) {
            return;
        }
        if inst.children.len() != main.children.len() {
            out.push(format!("{} has {} layers, its main {} has {}", inst.id, inst.children.len(), main.id, main.children.len()));
            return;
        }
        for (a, b) in inst.children.iter().zip(&main.children) {
            let (Some(an), Some(bn)) = (doc.node(*a), doc.node(*b)) else { continue };
            if an.source != Some(bn.id) {
                out.push(format!("{} mirrors {:?}, expected {}", an.id, an.source, bn.id));
                continue;
            }
            match (&an.kind, &bn.kind) {
                (NodeKind::Instance { component: x, .. }, NodeKind::Instance { component: y, .. }) if x != y => {
                    if let Some(m) = doc.node(*x) {
                        walk(doc, an, m, depth + 1, out);
                    }
                }
                _ => walk(doc, an, bn, depth + 1, out),
            }
        }
    }
    let mut out = Vec::new();
    if let Some(n) = doc.node(id)
        && let NodeKind::Instance { component, .. } = &n.kind
        && let Some(m) = doc.node(*component)
    {
        walk(doc, n, m, 0, &mut out);
    }
    out
}

/// The nested instance copies inside instance `id` whose main-component counterpart is exposed
/// (top-most first; an exposed instance's own exposed nested instances follow it).
pub fn exposed_copies(doc: &Document, id: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack: Vec<NodeId> = doc.node(id).map(|n| n.children.iter().rev().copied().collect()).unwrap_or_default();
    let mut seen = 0usize;
    while let Some(c) = stack.pop() {
        seen += 1;
        if seen > 100_000 {
            break;
        }
        let Some(n) = doc.node(c) else { continue };
        // A hidden exposed instance hides its properties too.
        let exposed =
            n.visible && matches!(n.kind, NodeKind::Instance { .. }) && n.source.and_then(|src| doc.node(src)).is_some_and(|src| src.exposed);
        if exposed {
            out.push(c);
        }
        // Only exposed nested instances pass their own exposed instances up.
        if !matches!(n.kind, NodeKind::Instance { .. }) || exposed {
            stack.extend(n.children.iter().rev().copied());
        }
    }
    out
}
