//! Handoff snapshots: what a ready-for-dev frame or section looked like before it changed, so Dev
//! Mode can compare it with what it is now.
//!
//! A snapshot is taken (by the engine) the moment a layer marked Ready for dev first changes, from
//! the document as it was just before that edit, and dropped when the change is resolved or the
//! layer is marked again. It holds the layer and everything under it; the nodes are shared with the
//! undo history in memory, so taking one costs little. Files without snapshots load unchanged.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{Document, Node, NodeId};

/// Most layers a snapshot holds (a larger frame isn't snapshotted and can't be compared).
pub const MAX_SNAPSHOT_NODES: usize = 20_000;

/// A layer and everything under it, in pre-order (the layer first), as it was.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DevSnapshot {
    pub nodes: Vec<Arc<Node>>,
}

impl DevSnapshot {
    /// The layer the snapshot is of.
    pub fn root(&self) -> Option<&Node> {
        self.nodes.first().map(Arc::as_ref)
    }

    /// The snapshot's nodes by id.
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id).map(Arc::as_ref)
    }
}

impl Document {
    /// Layer `root` and everything under it as they are now (`None` when it doesn't exist or holds
    /// more than [`MAX_SNAPSHOT_NODES`] layers).
    pub fn snapshot_subtree(&self, root: NodeId) -> Option<DevSnapshot> {
        let ids = self.subtree(root);
        if ids.is_empty() || ids.len() > MAX_SNAPSHOT_NODES {
            return None;
        }
        Some(DevSnapshot { nodes: ids.iter().filter_map(|i| self.nodes.get(i).cloned()).collect() })
    }

    /// A copy of this document in which layer `root` is as its snapshot shows it (`None` without a
    /// usable snapshot). Only `root`'s subtree is meant to be read from the copy: layers moved out
    /// of it since are back inside, and layers added to it since are gone.
    pub fn with_dev_snapshot(&self, root: NodeId) -> Option<Document> {
        let snap = self.dev_snapshots.get(&root)?;
        if snap.root().map(|n| n.id) != Some(root) || !self.contains(root) {
            return None;
        }
        let mut d = self.clone();
        let kept: BTreeSet<NodeId> = snap.nodes.iter().map(|n| n.id).collect();
        for id in self.subtree(root) {
            if !kept.contains(&id) {
                d.nodes.remove(&id);
            }
        }
        for n in snap.nodes.iter().take(MAX_SNAPSHOT_NODES) {
            // Pages are never part of a snapshot (a damaged file could claim otherwise).
            if n.is_page() || self.pages.contains(&n.id) {
                continue;
            }
            d.nodes.insert(n.id, n.clone());
        }
        // The copy's root keeps its current place in the page.
        if let (Some(now), Some(r)) = (self.node(root), d.nodes.get_mut(&root)) {
            Arc::make_mut(r).parent = now.parent;
        }
        // A damaged file can give a historical layer itself (or an ancestor) as a child; a copy
        // with a cycle under the root would send every walk of it round forever.
        if !d.is_tree_under(root) {
            return None;
        }
        d.dev_snapshots.clear();
        d.dirty = crate::Dirty::default();
        Some(d)
    }

    /// Whether every layer under `root` is reached once (no layer is its own descendant or listed
    /// by two parents), walking iteratively and stopping at the first repeat.
    fn is_tree_under(&self, root: NodeId) -> bool {
        let mut seen = BTreeSet::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                return false;
            }
            let Some(n) = self.nodes.get(&id) else {
                return false;
            };
            for &c in &n.children {
                let Some(child) = self.nodes.get(&c) else {
                    return false;
                };
                if child.parent != Some(id) {
                    return false;
                }
                stack.push(c);
            }
        }
        true
    }
}
