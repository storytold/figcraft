//! Explicit .fig GUID provenance projected through instance expansion contexts.
//! All strings borrow the bounded JSON; no names, node-kind similarity or child order match links.

use std::collections::{BTreeMap, BTreeSet};

use figcraft_doc::{Document, NodeId, NodeKind};
use serde_json::Value;

use super::{FIG_SOURCE_FIELD, MAX_DEPTH};

// A GUID has at most two 20-digit u64 parts and a colon; instance paths contain at most
// MAX_DEPTH + 2 GUIDs. Check before constructing any projected path string.
const MAX_SOURCE_ID_BYTES: usize = 44 * (MAX_DEPTH + 2);

#[derive(Clone, Copy)]
struct Origin<'a> {
    rest: &'a str,
    guid: &'a str,
    expanded: bool,
    slot_content: bool,
    wire_frame: bool,
    wire_instance: bool,
    component: Option<&'a str>,
}

pub(super) struct Sources<'a> {
    /// Most layers recorded.
    max_nodes: usize,
    nodes: BTreeMap<NodeId, Origin<'a>>,
    duplicate_ids: BTreeSet<&'a str>,
    rejected: usize,
}

fn guid(s: &str) -> bool {
    let Some((session, local)) = s.split_once(':') else { return false };
    s.len() <= 41
        && [session, local].into_iter().all(|part| !part.is_empty() && part.bytes().all(|c| c.is_ascii_digit()) && part.parse::<u64>().is_ok())
}

fn expanded_id(instance: &str, origin: &str) -> Option<String> {
    let bytes = instance.len().checked_add(origin.len())?.checked_add(2)?;
    if bytes > MAX_SOURCE_ID_BYTES {
        return None;
    }
    Some(if instance.starts_with('I') { format!("{instance};{origin}") } else { format!("I{instance};{origin}") })
}

impl<'a> Sources<'a> {
    pub(super) fn new(max_nodes: usize) -> Self {
        Self { max_nodes, nodes: BTreeMap::new(), duplicate_ids: BTreeSet::new(), rejected: 0 }
    }

    pub(super) fn record(&mut self, id: NodeId, node: &'a Value, primary: bool) {
        let Some(rest) = node.get("id").and_then(Value::as_str) else { return };
        if !primary {
            self.duplicate_ids.insert(rest);
            return;
        }
        let Some(meta) = node.get(FIG_SOURCE_FIELD) else { return };
        let parsed = meta.get("origin").and_then(Value::as_str).zip(meta.get("expanded").and_then(Value::as_bool));
        let Some((origin, expanded)) = parsed else {
            self.rejected += 1;
            return;
        };
        if self.nodes.len() >= self.max_nodes
            || rest.len() > MAX_SOURCE_ID_BYTES
            || !guid(origin)
            || (!expanded && rest != origin)
            || (expanded && !rest.starts_with('I'))
        {
            self.rejected += 1;
            return;
        }
        let wire_frame = meta.get("wireFrame").and_then(Value::as_bool).unwrap_or(false);
        let wire_instance = !wire_frame
            && meta.get("wireInstance").and_then(Value::as_bool) == Some(true)
            && node.get("type").and_then(Value::as_str) == Some("INSTANCE");
        let component = wire_instance.then(|| node.get("componentId").and_then(Value::as_str).filter(|id| guid(id))).flatten();
        let slot_content = meta.get("slotContent").and_then(Value::as_bool).unwrap_or(false);
        self.nodes.insert(id, Origin { rest, guid: origin, expanded, slot_content, wire_frame, wire_instance, component });
    }

    fn origin(&self, id: NodeId) -> Option<Origin<'a>> {
        let meta = *self.nodes.get(&id)?;
        (!self.duplicate_ids.contains(meta.rest)).then_some(meta)
    }

    fn raw_in_main(&self, origin: &str, main: NodeId, doc: &Document, ids: &BTreeMap<String, NodeId>) -> Option<NodeId> {
        let target = *ids.get(origin)?;
        let meta = self.origin(target)?;
        (!meta.expanded && meta.rest == origin && target != main && doc.is_ancestor(main, target)).then_some(target)
    }

    fn root_source(&self, id: NodeId, doc: &Document) -> Option<NodeId> {
        let NodeKind::Instance { component, .. } = &doc.node(id)?.kind else { return None };
        let main_meta = self.origin(*component)?;
        (!main_meta.expanded
            && matches!(doc.node(*component)?.kind, NodeKind::Component { .. })
            && *component != id
            && !doc.is_ancestor(id, *component)
            && !doc.is_ancestor(*component, id))
        .then_some(*component)
    }

    fn missing_frame(&self, id: NodeId, doc: &Document, ids: &BTreeMap<String, NodeId>) -> bool {
        self.origin(id).is_some_and(|meta| {
            meta.wire_instance
                && matches!(doc.node(id).map(|n| &n.kind), Some(NodeKind::Frame))
                // An existing target of the wrong kind or a cycle-blocked main is not missing.
                && meta.component.is_some_and(|main| !ids.contains_key(main))
        })
    }

    fn owner(&self, id: NodeId, doc: &Document, ids: &BTreeMap<String, NodeId>) -> Option<NodeId> {
        let mut current = doc.node(id)?.parent;
        let mut seen = BTreeSet::from([id]);
        for _ in 0..512 {
            let parent = current?;
            if !seen.insert(parent) {
                return None;
            }
            let node = doc.node(parent)?;
            if matches!(node.kind, NodeKind::Instance { .. }) || self.missing_frame(parent, doc, ids) {
                return Some(parent);
            }
            current = node.parent;
        }
        None
    }

    fn namespace(&self, id: NodeId, owner: NodeId, doc: &Document) -> bool {
        let Some(owner_meta) = self.origin(owner) else { return false };
        let mut current = id;
        let mut seen = BTreeSet::new();
        for _ in 0..512 {
            if current == owner {
                return true;
            }
            if !seen.insert(current) {
                return false;
            }
            let Some(meta) = self.origin(current) else { return false };
            if !meta.expanded || expanded_id(owner_meta.rest, meta.guid).as_deref() != Some(meta.rest) {
                return false;
            }
            let Some(parent) = doc.node(current).and_then(|n| n.parent) else { return false };
            current = parent;
        }
        false
    }

    // A missing-main instance is materialized as a frame. Outside real instances its children
    // are owned layers; known instances there become ordinary roots with their own actual main.
    fn owned_layer(&self, id: NodeId, doc: &Document, ids: &BTreeMap<String, NodeId>) -> bool {
        if doc.ancestors(id).into_iter().any(|p| doc.node(p).is_some_and(|n| matches!(n.kind, NodeKind::Instance { .. }))) {
            return false;
        }
        let mut current = id;
        for _ in 0..=MAX_DEPTH {
            let Some(owner) = self.owner(current, doc, ids) else { return false };
            if !self.missing_frame(owner, doc, ids) || !self.namespace(current, owner, doc) {
                return false;
            }
            let Some(meta) = self.origin(owner) else { return false };
            if !meta.expanded {
                return true;
            }
            current = owner;
        }
        false
    }

    fn frame_target(&self, id: NodeId, owner: NodeId, source_owner: NodeId, doc: &Document, ids: &BTreeMap<String, NodeId>) -> Option<NodeId> {
        let meta = self.origin(id)?;
        let owner_meta = self.origin(owner)?;
        let source_meta = self.origin(source_owner)?;
        if !self.missing_frame(source_owner, doc, ids) || owner_meta.component != source_meta.component {
            return None;
        }
        let target = *ids.get(expanded_id(source_meta.rest, meta.guid)?.as_str())?;
        let target_meta = self.origin(target)?;
        if !target_meta.expanded
            || target_meta.guid != meta.guid
            || self.owner(target, doc, ids) != Some(source_owner)
            || !self.namespace(target, source_owner, doc)
        {
            return None;
        }
        let parent = doc.node(id)?.parent?;
        let target_parent = doc.node(target)?.parent?;
        if parent == owner {
            (target_parent == source_owner).then_some(target)
        } else {
            (target_parent != source_owner && self.origin(parent)?.guid == self.origin(target_parent)?.guid).then_some(target)
        }
    }

    fn source(
        &self,
        id: NodeId,
        doc: &Document,
        ids: &BTreeMap<String, NodeId>,
        memo: &mut BTreeMap<NodeId, Option<NodeId>>,
        active: &mut BTreeSet<NodeId>,
        depth: usize,
    ) -> Option<NodeId> {
        if let Some(source) = memo.get(&id) {
            return *source;
        }
        if depth > MAX_DEPTH || !active.insert(id) {
            return None;
        }
        let found = (|| {
            let meta = self.origin(id)?;
            let _ = doc.node(id)?;
            if meta.slot_content {
                return if matches!(doc.node(id)?.kind, NodeKind::Instance { .. }) { self.root_source(id, doc) } else { None };
            }
            if !meta.expanded {
                return self.root_source(id, doc);
            }
            // An actual wire INSTANCE whose main is absent remains a namespace owner even
            // when the REST importer materializes it as a frame. Ordinary frames never do.
            let owner = self.owner(id, doc, ids)?;
            if !self.namespace(id, owner, doc) {
                return None;
            }
            let source_owner = self.source(owner, doc, ids, memo, active, depth + 1);
            if self.missing_frame(owner, doc, ids) {
                let target = match source_owner {
                    Some(source_owner) => self.frame_target(id, owner, source_owner, doc, ids)?,
                    None if self.owned_layer(id, doc, ids) => return self.root_source(id, doc),
                    _ => return None,
                };
                return (target != id && !doc.is_ancestor(id, target)).then_some(target);
            }
            let NodeKind::Instance { component: main, .. } = &doc.node(owner)?.kind else { return None };
            let source_owner = source_owner?;
            let target = match &doc.node(source_owner)?.kind {
                NodeKind::Component { .. } if source_owner == *main => self.raw_in_main(meta.guid, *main, doc, ids)?,
                NodeKind::Instance { component: source_main, .. } if source_main != main => {
                    // A resolved main switch is evidence for resetting to that CURRENT main.
                    self.raw_in_main(meta.guid, *main, doc, ids)?
                }
                NodeKind::Instance { component: source_main, .. } if source_main == main => {
                    let source_meta = self.origin(source_owner)?;
                    let target = *ids.get(expanded_id(source_meta.rest, meta.guid)?.as_str())?;
                    let target_meta = self.origin(target)?;
                    let target_owner = self.owner(target, doc, ids);
                    if !target_meta.expanded || target_meta.guid != meta.guid || target_owner != Some(source_owner) {
                        return None;
                    }
                    target
                }
                NodeKind::Frame if self.origin(source_owner)?.wire_frame => {
                    // An actual wire FRAME becoming INSTANCE has no previous main. Its resolved
                    // current main is the only proven namespace for its newly expanded children.
                    self.raw_in_main(meta.guid, *main, doc, ids)?
                }
                _ => return None,
            };
            (target != id && !doc.is_ancestor(id, target)).then_some(target)
        })();
        active.remove(&id);
        memo.insert(id, found);
        found
    }

    pub(super) fn apply(self, doc: &mut Document, ids: &BTreeMap<String, NodeId>) -> usize {
        let mut links = BTreeMap::new();
        let mut active = BTreeSet::new();
        for &id in self.nodes.keys() {
            let _ = self.source(id, doc, ids, &mut links, &mut active, 0);
        }
        let mut missing = self.rejected;
        for (&id, meta) in &self.nodes {
            let needs_source = (!meta.slot_content && meta.expanded && !self.owned_layer(id, doc, ids))
                || doc.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Instance { .. }));
            if !needs_source {
                continue;
            }
            let target = links.get(&id).copied().flatten().filter(|_| acyclic(id, &links));
            if let Some(target) = target {
                if let Some(node) = doc.node_mut(id) {
                    node.source = Some(target);
                }
            } else {
                missing += 1;
            }
        }
        missing
    }
}

fn acyclic(id: NodeId, links: &BTreeMap<NodeId, Option<NodeId>>) -> bool {
    let mut current = id;
    for _ in 0..=MAX_DEPTH {
        let Some(next) = links.get(&current).copied().flatten() else { return true };
        if next == id {
            return false;
        }
        current = next;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_cycles_and_overlong_source_chains_are_rejected() {
        let self_link = BTreeMap::from([(NodeId(1), Some(NodeId(1)))]);
        assert!(!acyclic(NodeId(1), &self_link));
        let cycle = BTreeMap::from([(NodeId(1), Some(NodeId(2))), (NodeId(2), Some(NodeId(1)))]);
        assert!(!acyclic(NodeId(1), &cycle));
        let long = (1..=MAX_DEPTH as u64 + 2).map(|i| (NodeId(i), Some(NodeId(i + 1)))).collect();
        assert!(!acyclic(NodeId(1), &long));
        assert!(expanded_id(&"x".repeat(MAX_SOURCE_ID_BYTES), "1:2").is_none());
    }
}
