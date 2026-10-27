//! Diffing: the changes between a replica's document and an edited version of it.

use std::collections::{BTreeMap, BTreeSet};

use figcraft_doc::{Document, NodeId};

use crate::change::{Change, Json, Prop};
use crate::frac::{self, FIRST_KEY, KEY_RESPACE_LEN};
use crate::replica::{Replica, create_change};
use crate::{Error, docprops};

/// The result of [`diff`]: the changes, and the replica of the edited document (with the position
/// keys the changes give it).
#[derive(Clone, Debug)]
pub struct Diff {
    /// One batch: field writes, creations (parents first), moves, deletions, then document-level
    /// changes. Empty when nothing synced changed.
    pub changes: Vec<Change>,
    /// The edited document as a replica: what applying `changes` to the original gives.
    pub replica: Replica,
}

/// The changes that turn `from`'s document into `to` (an edited version of it, e.g. after a
/// command), at property granularity. Only nodes whose `Arc` differs from `from`'s are looked at,
/// so the cost follows the size of the edit, not of the document. Children that were added, moved
/// or reordered get new position keys; their siblings keep theirs.
pub fn diff(from: &Replica, to: &Document) -> Result<Diff, Error> {
    let a = from.document();
    let ids = to.changed_ids(a, usize::MAX).unwrap_or_default();
    let mut positions = from.positions().clone();
    let (mut fields, mut added, mut removed) = (Vec::new(), Vec::new(), BTreeSet::new());
    // Parents whose children need keys (`None`: the page list).
    let mut parents: BTreeSet<Option<NodeId>> = BTreeSet::new();
    let mut moved: BTreeSet<NodeId> = BTreeSet::new();
    for id in ids {
        match (a.node(id), to.node(id)) {
            (None, Some(n)) => {
                added.push(id);
                parents.insert(n.parent);
            }
            (Some(_), None) => {
                removed.insert(id);
            }
            (Some(o), Some(n)) => {
                for p in Prop::FIELDS {
                    if p.differs(o, n) {
                        fields.push(Change::Set { id, prop: *p, value: field_update(*p, o, n)? });
                    }
                }
                if o.parent != n.parent {
                    parents.insert(n.parent);
                    moved.insert(id);
                }
                if o.children != n.children {
                    parents.insert(Some(id));
                }
            }
            (None, None) => {}
        }
    }
    if a.pages != to.pages {
        parents.insert(None);
    }
    let mut rekeyed: BTreeMap<NodeId, String> = BTreeMap::new();
    for p in parents {
        let kids = match p {
            None => &to.pages,
            Some(p) => match to.node(p) {
                Some(n) => &n.children,
                None => continue,
            },
        };
        // A child keeps its key only if it was under the same parent before.
        let old = |c: NodeId| a.node(c).filter(|n| n.parent == p).and_then(|_| from.positions().get(&c)).map(String::as_str);
        for (c, k) in keys_for(kids, old)? {
            positions.insert(c, k.clone());
            rekeyed.insert(c, k);
        }
    }
    // Unlisted pages (the library store) still need a key.
    for id in &added {
        if !rekeyed.contains_key(id) && to.node(*id).is_some_and(|n| n.parent.is_none()) {
            positions.insert(*id, FIRST_KEY.to_string());
        }
    }
    let replica = {
        let mut doc = to.clone();
        doc.published = None;
        doc.take_dirty();
        for r in &removed {
            positions.remove(r);
        }
        Replica::from_parts(doc, positions)
    };
    let mut changes = fields;
    let new: BTreeSet<NodeId> = added.iter().copied().collect();
    // Creations, parents before children.
    added.sort_by_cached_key(|id| (depth(to, *id), *id));
    for id in &added {
        if let Some(n) = to.node(*id) {
            changes.push(create_change(n, replica.key(*id))?);
        }
    }
    for (c, k) in &rekeyed {
        if new.contains(c) {
            continue;
        }
        if moved.contains(c) {
            let parent = to.node(*c).and_then(|n| n.parent);
            changes.push(Change::Set { id: *c, prop: Prop::Parent, value: Json::of(&parent)? });
        }
        changes.push(Change::Set { id: *c, prop: Prop::Position, value: Json::of(k)? });
    }
    // Deletions last (a layer moved out of a deleted frame has moved by then); one per subtree.
    for id in &removed {
        if a.node(*id).and_then(|n| n.parent).is_none_or(|p| !removed.contains(&p)) {
            changes.push(Change::Delete { id: *id });
        }
    }
    docprops::diff(a, to, &mut changes)?;
    Ok(Diff { changes, replica })
}

/// Omit unchanged immutable geometry from a text edit. Full creations/snapshots and edits
/// which regenerate geometry still carry the complete appearance. Explicit null clears it;
/// the explicit metadata marker preserves even concurrently changed text and geometry.
fn field_update(prop: Prop, before: &figcraft_doc::Node, after: &figcraft_doc::Node) -> Result<Json, Error> {
    if prop != Prop::Kind {
        return prop.get(after);
    }
    let (Some(old), Some(new)) = (before.kind.text(), after.kind.text()) else { return prop.get(after) };
    if old.appearance.is_some() && old.appearance == new.appearance && old.runs == new.runs && same_text_geometry(old, new) {
        let mut kind = after.kind.clone();
        if let figcraft_doc::NodeKind::Text(t) = &mut kind {
            t.appearance = None;
        }
        let mut value = Json::of(&kind)?;
        if let Some(object) = value.0.as_object_mut() {
            object.insert("unchangedAppearance".into(), serde_json::Value::Bool(true));
        }
        return Ok(value);
    }
    prop.get(after)
}

fn same_text_geometry(a: &figcraft_doc::TextData, b: &figcraft_doc::TextData) -> bool {
    let clean = |t: &figcraft_doc::TextData| {
        let mut t = t.clone();
        t.appearance = None;
        t.style.link = None;
        t.style.style_id = None;
        for run in &mut t.runs {
            run.style.link = None;
            run.style.style_id = None;
        }
        t
    };
    clean(a) == clean(b)
}

/// Depth of `id` in `doc` (pages are 0).
fn depth(doc: &Document, id: NodeId) -> usize {
    let mut d = 0;
    let mut cur = doc.node(id).and_then(|n| n.parent);
    while let Some(p) = cur {
        d += 1;
        if d > 100_000 {
            break;
        }
        cur = doc.node(p).and_then(|n| n.parent);
    }
    d
}

/// New keys for the children `kids` (in their new order) whose `old` key no longer fits: the
/// longest run of old keys that is still increasing is kept, the rest get keys between their kept
/// neighbours. Falls back to re-spacing every child when keys would get long.
fn keys_for<'a>(kids: &[NodeId], old: impl Fn(NodeId) -> Option<&'a str>) -> Result<Vec<(NodeId, String)>, Error> {
    let olds: Vec<Option<&str>> = kids.iter().map(|c| old(*c)).collect();
    let kept = increasing_run(&olds);
    let mut out = Vec::new();
    let mut i = 0;
    let mut prev: Option<&str> = None;
    while i < kids.len() {
        if kept.get(i).copied().unwrap_or(false) {
            prev = olds.get(i).copied().flatten();
            i += 1;
            continue;
        }
        let start = i;
        while i < kids.len() && !kept.get(i).copied().unwrap_or(false) {
            i += 1;
        }
        let next = if i < kids.len() { olds.get(i).copied().flatten() } else { None };
        let keys = frac::n_between(prev, next, i - start).map_err(|e| Error::Invalid(e.to_string()))?;
        if keys.iter().any(|k| k.len() > KEY_RESPACE_LEN) {
            return respace(kids, &olds);
        }
        out.extend(kids.get(start..i).unwrap_or_default().iter().copied().zip(keys));
    }
    Ok(out)
}

/// Fresh, short keys for every child (those whose key changes).
fn respace(kids: &[NodeId], olds: &[Option<&str>]) -> Result<Vec<(NodeId, String)>, Error> {
    let keys = frac::n_between(None, None, kids.len()).map_err(|e| Error::Invalid(e.to_string()))?;
    Ok(kids.iter().zip(olds).zip(keys).filter(|((_, o), k)| **o != Some(k.as_str())).map(|((c, _), k)| (*c, k)).collect())
}

/// Which entries form a longest strictly increasing subsequence of the present keys.
fn increasing_run(keys: &[Option<&str>]) -> Vec<bool> {
    // Patience sorting: `tails[l]` is the index ending the best run of length l + 1.
    let mut tails: Vec<usize> = Vec::new();
    let mut prev: Vec<Option<usize>> = vec![None; keys.len()];
    for (i, k) in keys.iter().enumerate() {
        let Some(k) = k else { continue };
        let at = tails.partition_point(|t| keys.get(*t).copied().flatten().is_some_and(|tk| tk < *k));
        if let Some(p) = prev.get_mut(i) {
            *p = at.checked_sub(1).and_then(|b| tails.get(b)).copied();
        }
        match tails.get_mut(at) {
            Some(t) => *t = i,
            None => tails.push(i),
        }
    }
    let mut kept = vec![false; keys.len()];
    let mut cur = tails.last().copied();
    while let Some(i) = cur {
        if let Some(k) = kept.get_mut(i) {
            *k = true;
        }
        cur = prev.get(i).copied().flatten();
    }
    kept
}
