//! A component can't contain its own instances, directly or through other components (#1084):
//! every instance would show itself without end. Each variant is a component of its own here: one
//! holding an instance of another variant of its set is fine unless that one leads back to it.
//! Commands that would close such a loop are refused ([`check_component_cycles`]);
//! files that hold one are broken on opening ([`break_component_cycles`]); and a rebuild never
//! expands an instance inside a copy of itself ([`expansion_layers`]), for loops that arrive some
//! other way (two edits merged in a live session).

use super::patch::rebuild_instances;
use super::{BTreeMap, BTreeSet, Document, NodeId, NodeKind, PROP_BINDING, PropDef, inside_instance, root_instance};

/// The components each main component holds instances of.
type Graph = BTreeMap<NodeId, BTreeSet<NodeId>>;

fn is_component(doc: &Document, id: NodeId) -> bool {
    doc.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Component { .. }))
}

/// The main components `id` is (in).
fn hosts(doc: &Document, id: NodeId) -> BTreeSet<NodeId> {
    std::iter::once(id).chain(doc.ancestors(id)).filter(|a| is_component(doc, *a)).collect()
}

/// The components instance `id` shows: its main, and those its instance swap properties pick
/// (they swap a layer inside it when it's rebuilt).
fn shown(doc: &Document, id: NodeId) -> Vec<NodeId> {
    let Some(NodeKind::Instance { component, props }) = doc.node(id).map(|n| &n.kind) else { return Vec::new() };
    let Some(NodeKind::Component { props: defs, .. }) = doc.node(*component).map(|n| &n.kind) else { return Vec::new() };
    let swaps =
        defs.iter().filter(|(_, d)| matches!(d, PropDef::InstanceSwap { .. })).filter_map(|(k, _)| props.get(k).and_then(crate::params::id_of));
    std::iter::once(*component).chain(swaps).filter(|c| is_component(doc, *c)).collect()
}

fn graph(doc: &Document) -> Graph {
    let mut g = Graph::new();
    for n in doc.nodes().filter(|n| matches!(n.kind, NodeKind::Instance { .. })) {
        let shown = shown(doc, n.id);
        for h in hosts(doc, n.id) {
            g.entry(h).or_default().extend(shown.iter().copied());
        }
    }
    g
}

/// A loop in `g`, its first component repeated last (`A, B, A`).
fn find_cycle(g: &Graph) -> Option<Vec<NodeId>> {
    let next = |u: &NodeId| -> Vec<NodeId> { g.get(u).map(|s| s.iter().rev().copied().collect()).unwrap_or_default() };
    let mut done: BTreeSet<NodeId> = BTreeSet::new();
    for start in g.keys() {
        if done.contains(start) {
            continue;
        }
        // Depth first: the path so far, and the components each step on it has left to visit.
        let mut path = vec![*start];
        let mut todo = vec![next(start)];
        while let Some(left) = todo.last_mut() {
            match left.pop() {
                Some(u) if path.contains(&u) => {
                    let at = path.iter().position(|p| *p == u)?;
                    let mut cycle = path.split_off(at);
                    cycle.push(u);
                    return Some(cycle);
                }
                Some(u) if done.contains(&u) => {}
                Some(u) => {
                    todo.push(next(&u));
                    path.push(u);
                }
                None => {
                    todo.pop();
                    done.extend(path.pop());
                }
            }
        }
    }
    None
}

/// A loop of components whose mains contain each other's instances, if the document has one.
pub fn component_cycle(doc: &Document) -> Option<Vec<NodeId>> {
    find_cycle(&graph(doc))
}

/// A loop as its names: `Card → Button → Card`.
fn cycle_names(doc: &Document, cycle: &[NodeId]) -> String {
    cycle.iter().map(|u| doc.node(*u).map_or_else(|| u.to_string(), |n| format!("\"{}\"", n.name))).collect::<Vec<_>>().join(" → ")
}

/// Whether an instance of `component` under `parent` would contain itself: `parent` is (in) a
/// main that instance shows, directly or through other components.
pub fn would_contain_itself(doc: &Document, component: NodeId, parent: NodeId) -> bool {
    let hosts = hosts(doc, parent);
    if hosts.is_empty() {
        return false;
    }
    let g = graph(doc);
    let mut seen = BTreeSet::new();
    let mut todo = vec![component];
    while let Some(u) = todo.pop() {
        if hosts.contains(&u) {
            return true;
        }
        if seen.insert(u) {
            todo.extend(g.get(&u).into_iter().flatten().copied());
        }
    }
    false
}

/// Why an instance of `component` can't go where it would contain itself.
pub fn contains_itself_error(doc: &Document, component: NodeId) -> String {
    let name = doc.node(component).map(|n| n.name.clone()).unwrap_or_default();
    format!("an instance of \"{name}\" can't go inside \"{name}\" or a component it contains: a component can't contain itself")
}

/// The loop `after` has and `before` didn't, if layers `touched` (edited, added or removed) closed
/// one: only layers in main components can.
pub fn new_cycle<'a>(before: &Document, after: &Document, mut touched: impl Iterator<Item = &'a NodeId>) -> Option<String> {
    if !touched.any(|d| !hosts(after, *d).is_empty()) {
        return None;
    }
    cycle_text(after).filter(|_| component_cycle(before).is_none())
}

/// The document's loop of components containing each other's instances ([`component_cycle`]),
/// as its names.
pub fn cycle_text(doc: &Document) -> Option<String> {
    component_cycle(doc).map(|c| cycle_names(doc, &c))
}

/// Refuse an edit that makes components contain each other's instances (it is rolled back). A
/// document that already held a loop stays editable.
pub fn check_component_cycles(before: &Document, after: &Document) -> Result<(), String> {
    match new_cycle(before, after, after.dirty().iter()) {
        Some(names) => Err(format!("a component can't contain an instance of itself, directly or through other components: {names}")),
        None => Ok(()),
    }
}

/// The instance to detach to cut `from → to` out of a loop: one in a main of `from` showing `to`,
/// preferably one of the main's own (not a layer of another instance, else its outer instance).
fn cut_point(doc: &Document, from: NodeId, to: NodeId) -> Option<NodeId> {
    let found: Vec<NodeId> = doc
        .nodes()
        .filter(|n| matches!(n.kind, NodeKind::Instance { .. }))
        .map(|n| n.id)
        .filter(|i| shown(doc, *i).contains(&to) && hosts(doc, *i).contains(&from))
        .collect();
    found.iter().copied().find(|i| !inside_instance(doc, *i)).or_else(|| found.first().and_then(|i| root_instance(doc, *i)))
}

/// Detach instance `top` and every instance inside it: its layers stay as they are, linked to no
/// main.
fn detach_all(doc: &mut Document, top: NodeId) {
    for d in doc.subtree(top) {
        let modes = match doc.node(d).map(|n| &n.kind) {
            Some(NodeKind::Instance { component, .. }) => doc.node(*component).map(|m| m.var_modes.clone()),
            _ => None,
        };
        let Some(n) = doc.node_mut(d) else { continue };
        if let Some(modes) = modes {
            n.kind = NodeKind::Frame;
            n.bound.retain(|k, _| !k.starts_with(PROP_BINDING));
            for (c, m) in modes {
                n.var_modes.entry(c).or_insert(m);
            }
        }
        n.source = None;
        n.overrides.clear();
        n.prop_refs.clear();
    }
}

/// Break every loop of components containing each other's instances (a file from elsewhere, or
/// written before they were refused): an instance closing each loop is detached, and the instances
/// of its main rebuild without it. Returns a note for each.
pub fn break_component_cycles(doc: &mut Document) -> Vec<String> {
    let mut notes = Vec::new();
    // Each pass detaches an instance: never more passes than there are layers.
    for _ in 0..doc.nodes().count() {
        let Some(cycle) = component_cycle(doc) else { break };
        let (Some(from), Some(to)) = (cycle.iter().rev().nth(1).copied(), cycle.last().copied()) else { break };
        let Some(cut) = cut_point(doc, from, to) else { break };
        let name = doc.node(cut).map(|n| n.name.clone()).unwrap_or_default();
        detach_all(doc, cut);
        rebuild_instances(doc, &BTreeSet::from([from]));
        notes.push(format!("\"{name}\" was detached: components can't contain their own instances ({})", cycle_names(doc, &cycle)));
    }
    notes
}

/// The layers of `component` an instance `inst` of it copies, in pre-order: all of them but those
/// inside a nested instance of a component being expanded already (its own, or a main `inst` is
/// in), which would show itself again. `None` when `inst` itself is in its own main.
pub(super) fn expansion_layers(doc: &Document, inst: NodeId, component: NodeId) -> Option<Vec<NodeId>> {
    let mut stops = hosts(doc, inst);
    if !stops.insert(component) {
        return None;
    }
    let repeats = |kind: &NodeKind| matches!(kind, NodeKind::Instance { component: c, .. } if stops.contains(c));
    let mut out = Vec::new();
    let mut stack: Vec<NodeId> = doc.node(component).map(|n| n.children.iter().rev().copied().collect()).unwrap_or_default();
    while let Some(id) = stack.pop() {
        let Some(n) = doc.node(id) else { continue };
        out.push(id);
        if out.len() > 5_000_000 {
            break;
        }
        if !repeats(&n.kind) {
            stack.extend(n.children.iter().rev());
        }
    }
    Some(out)
}
