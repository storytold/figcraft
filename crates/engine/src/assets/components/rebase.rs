use super::diff::{diff_groups, variable_groups};
use super::instances::{field_group, path_map};
use super::{BTreeMap, BTreeSet, Document, MAX_DEPTH, Node, NodeId, NodeKind, default_instance_name, holds_content};

/// Bounded source and parent lookups while a rebase reads an unchanged document.
pub(super) struct RebaseSources<'a> {
    doc: &'a Document,
    chains: BTreeMap<NodeId, Option<Vec<NodeId>>>,
    parents: BTreeMap<(NodeId, NodeId), Option<bool>>,
    main_changes: BTreeMap<NodeId, bool>,
}

impl<'a> RebaseSources<'a> {
    const PARENTS: usize = 512;
    const CACHE: usize = 8192;

    pub(super) fn new(doc: &'a Document) -> Self {
        Self { doc, chains: BTreeMap::new(), parents: BTreeMap::new(), main_changes: BTreeMap::new() }
    }

    fn chain(&mut self, source: NodeId) -> Option<Vec<NodeId>> {
        if let Some(chain) = self.chains.get(&source) {
            return chain.clone();
        }
        let read = || {
            let mut chain = Vec::new();
            let mut seen = BTreeSet::new();
            let mut current = Some(source);
            for _ in 0..MAX_DEPTH {
                let Some(id) = current else { return Some(chain) };
                if !seen.insert(id) {
                    return None;
                }
                let n = self.doc.node(id)?;
                chain.push(id);
                current = n.source;
            }
            current.is_none().then_some(chain)
        };
        let chain = read();
        if self.chains.len() < Self::CACHE {
            self.chains.insert(source, chain.clone());
        }
        chain
    }

    /// Validate the complete parent chain, even if `main` was found before a cycle.
    fn in_main(&mut self, id: NodeId, main: NodeId) -> Option<bool> {
        if let Some(found) = self.parents.get(&(id, main)) {
            return *found;
        }
        let read = || {
            let mut current = Some(id);
            let mut seen = BTreeSet::new();
            let mut found = false;
            for _ in 0..Self::PARENTS {
                let Some(id) = current else { return Some(found) };
                if !seen.insert(id) {
                    return None;
                }
                found |= id == main;
                current = self.doc.node(id)?.parent;
            }
            current.is_none().then_some(found)
        };
        let found = read();
        if self.parents.len() < Self::CACHE {
            self.parents.insert((id, main), found);
        }
        found
    }

    fn member(&mut self, source: NodeId, main: NodeId, target: NodeId) -> Option<NodeId> {
        let chain = self.chain(source)?;
        if chain.contains(&target) {
            return None;
        }
        let mut member = None;
        for id in chain {
            if self.in_main(id, main)? && id != main && member.is_none() {
                member = Some(id);
            }
        }
        member
    }

    /// Nearest enclosing instances and their current mains. Slots with own content end scope.
    pub(super) fn owners(&self, id: NodeId) -> Option<Vec<(NodeId, NodeId)>> {
        let mut owners = Vec::new();
        let mut current = self.doc.node(id)?.parent;
        let mut seen = BTreeSet::from([id]);
        for _ in 0..Self::PARENTS {
            let Some(id) = current else { return Some(owners) };
            if !seen.insert(id) {
                return None;
            }
            let n = self.doc.node(id)?;
            if holds_content(n) {
                return Some(owners);
            }
            if let NodeKind::Instance { component, .. } = &n.kind {
                owners.push((id, *component));
            }
            current = n.parent;
        }
        current.is_none().then_some(owners)
    }

    fn contextual(&mut self, source: NodeId, owners: &[(NodeId, NodeId)]) -> bool {
        owners.iter().any(|(_, main)| self.in_main(source, *main) == Some(true) && source != *main)
    }

    /// A contextual child belongs to the same placed slot as its enclosing instance.
    fn valid(&mut self, source: NodeId, target: NodeId, owners: &[(NodeId, NodeId)]) -> bool {
        let Some((owner, main)) = owners.first() else { return false };
        if self.member(source, *main, target).is_none() {
            return false;
        }
        if self.in_main(source, *main) == Some(true) {
            return true;
        }
        let Some(source_owner) = self.doc.node(*owner).and_then(|n| n.source) else { return false };
        matches!(self.doc.node(source_owner).map(|n| &n.kind), Some(NodeKind::Instance { component, .. }) if component == main)
            && self.owners(source).and_then(|owners| owners.first().copied()).is_some_and(|(id, _)| id == source_owner)
    }

    fn placed(&mut self, id: NodeId, owners: &[(NodeId, NodeId)]) -> Option<NodeId> {
        let source = self.doc.node(id)?.source?;
        if !matches!(self.doc.node(source)?.kind, NodeKind::Instance { .. } | NodeKind::Frame)
            || !self.valid(source, id, owners)
            || !self.contextual(source, owners)
        {
            return None;
        }
        Some(source)
    }

    fn beneath_swap(&mut self, owners: &[(NodeId, NodeId)], stop: NodeId) -> bool {
        for (index, (id, main)) in owners.iter().enumerate() {
            if *id == stop {
                break;
            }
            let changed = if let Some(changed) = self.main_changes.get(id) {
                *changed
            } else {
                let changed = self.placed(*id, &owners[index + 1..]).is_some_and(|source| match self.doc.node(source).map(|n| &n.kind) {
                    Some(NodeKind::Instance { component, .. }) => component != main,
                    Some(NodeKind::Frame) => true,
                    _ => false,
                });
                if self.main_changes.len() < Self::CACHE {
                    self.main_changes.insert(*id, changed);
                }
                changed
            };
            if changed {
                return true;
            }
        }
        false
    }
}

/// Re-derive `source` links and overrides of instance `inst` against its own main by comparing
/// values (after detaching an outer instance, restoring a main, or copying a nested instance out):
/// the look is kept exactly. Values a component property sets aren't overrides.
pub fn rebase_instance(doc: &mut Document, inst: NodeId) {
    let Some(NodeKind::Instance { component, .. }) = doc.node(inst).map(|n| n.kind.clone()) else { return };
    let mine = path_map(doc, inst, true);
    let theirs: BTreeMap<String, NodeId> =
        if doc.contains(component) { path_map(doc, component, false).into_iter().map(|(k, v)| (v, k)).collect() } else { BTreeMap::new() };
    let value_groups = |a: &Node, b: &Node| -> Vec<String> {
        diff_groups(a, b).into_iter().filter(|g| !matches!(*g, "size" | "position")).map(str::to_string).collect()
    };
    let mut updates = Vec::new();
    let mut sources = RebaseSources::new(doc);
    let root_owners = sources.owners(inst).unwrap_or_default();
    let placed_source = sources.placed(inst, &root_owners);
    let mut contexts = vec![(inst, component)];
    contexts.extend_from_slice(&root_owners);
    for (id, path) in &mine {
        let Some(n) = doc.node(*id) else { continue };
        let owners = sources.owners(*id).unwrap_or_default();
        let nearest_main = owners.first().map(|(_, main)| *main);
        let valid_source = n.source.filter(|source| sources.valid(*source, *id, &owners));
        let mut main = valid_source.and_then(|source| sources.member(source, component, *id));
        // A swapped or Frame-to-Instance subtree is compared to its current main by the inner
        // rebase. An outer name path must not rebind those children to the old expanded copies.
        if main.is_none() && valid_source.is_some() && sources.beneath_swap(&owners, inst) {
            updates.push((*id, valid_source, n.overrides.clone()));
            continue;
        }
        main = main
            .or_else(|| theirs.get(path).copied().filter(|source| nearest_main.is_some_and(|scope| sources.member(*source, scope, *id).is_some())));
        let source = valid_source.filter(|source| sources.contextual(*source, &contexts)).filter(|_| main.is_some()).or(main);
        let mut overrides: BTreeSet<String> = n.overrides.iter().filter(|g| !matches!(g.as_str(), "component" | "props")).cloned().collect();
        if let Some(m) = source.and_then(|source| doc.node(source)) {
            let mut bound: Vec<&str> = m.prop_refs.keys().map(|f| field_group(f)).collect();
            bound.extend(variable_groups(n, m));
            overrides.extend(value_groups(m, n).into_iter().filter(|g| !bound.contains(&g.as_str())));
            if matches!((&m.kind, &n.kind), (NodeKind::Instance { component: a, .. }, NodeKind::Instance { component: b, .. }) if a != b) {
                overrides.insert("component".into());
            }
        }
        updates.push((*id, source, overrides));
    }
    // A placed nested root compares to its actual slot. A detached root compares to its own main.
    let root_base = placed_source.or_else(|| {
        let chain = sources.chain(component)?;
        (!chain.contains(&inst) && sources.in_main(component, inst) == Some(false)).then_some(component)
    });
    let root_groups = match (root_base.and_then(|source| doc.node(source)), doc.node(inst)) {
        (Some(m), Some(n)) => {
            let from_variables = variable_groups(n, m);
            let mut groups: Vec<String> = value_groups(m, n)
                .into_iter()
                .filter(|g| !matches!(g.as_str(), "name" | "exports" | "interactions") && !from_variables.contains(&g.as_str()))
                .filter(|g| placed_source.is_some() || !matches!(g.as_str(), "resizing" | "modes"))
                .collect();
            let named = if placed_source.is_some() { n.name != m.name } else { n.name != default_instance_name(doc, component) };
            if named {
                groups.push("name".into());
            }
            if placed_source.is_some()
                && matches!((&m.kind, &n.kind), (NodeKind::Instance { component: a, .. }, NodeKind::Instance { component: b, .. }) if a != b)
            {
                groups.push("component".into());
            }
            groups
        }
        _ => Vec::new(),
    };
    drop(sources);
    for (id, main, overrides) in updates {
        if let Some(n) = doc.node_mut(id) {
            n.source = main;
            n.overrides = if main.is_some() { overrides } else { BTreeSet::new() };
        }
    }
    if let Some(n) = doc.node_mut(inst) {
        n.source = root_base;
        // A top-level instance's resizing and modes are its own (a nested one's override its main
        // layer's).
        n.overrides.retain(|g| {
            !matches!(g.as_str(), "component" | "props" | "position") && (placed_source.is_some() || !matches!(g.as_str(), "resizing" | "modes"))
        });
        n.overrides.extend(root_groups);
    }
}
