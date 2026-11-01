//! A replica: one copy of the synced document state (the document plus every node's fractional
//! position), and applying changes to it.

use std::collections::BTreeSet;

use figcraft_doc::{Document, Node, NodeId, NodeKind};

use crate::change::{Change, Json, Prop, Target};
use crate::frac::{self, FIRST_KEY};
use crate::{Error, docprops};

/// Fractional position keys by node.
pub type Positions = imbl::OrdMap<NodeId, String>;

/// Largest coordinate, size or transform coefficient a synced node may have.
pub const MAX_COORD: f64 = 1e9;

/// One copy of the synced state: the document and the position key of every node. Children are
/// always ordered by `(key, id)` (pages too, as children of the document), so applying the same
/// changes in the same order gives the same document everywhere. Cloning is cheap (persistent maps).
#[derive(Clone, Debug, PartialEq)]
pub struct Replica {
    doc: Document,
    positions: Positions,
}

/// How strictly [`Replica::apply`] treats a change it can't apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Stop at the first invalid change and also check the result (types, numbers, tree rules).
    /// The replica is left partly changed: apply to a clone and keep it only on success.
    Strict,
    /// Skip changes that can't be applied (a client replaying its own unconfirmed edits on top of
    /// newer server state).
    Lenient,
}

/// What [`Replica::apply`] did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Applied {
    /// Nodes added, edited or removed (as the document's dirty set records them).
    pub touched: BTreeSet<NodeId>,
    /// Nodes deleted (subtrees included).
    pub deleted: Vec<NodeId>,
    /// Changes skipped in [`Mode::Lenient`].
    pub skipped: usize,
}

impl Replica {
    /// Start syncing `doc`: every node gets a position key in its current order (pages by the
    /// page list). The published library isn't synced and is dropped from the replica.
    pub fn new(mut doc: Document) -> Replica {
        let mut positions = Positions::new();
        let mut assign = |ids: &[NodeId]| {
            for (id, k) in ids.iter().zip(frac::n_between(None, None, ids.len()).unwrap_or_default()) {
                positions.insert(*id, k);
            }
        };
        assign(&doc.pages);
        for n in doc.nodes() {
            assign(&n.children);
        }
        // Pages that aren't listed (the library store) still need a key.
        let roots: Vec<NodeId> = doc.nodes().filter(|n| n.parent.is_none()).map(|n| n.id).filter(|id| !positions.contains_key(id)).collect();
        for id in roots {
            positions.insert(id, FIRST_KEY.to_string());
        }
        doc.published = None;
        doc.take_dirty();
        Replica { doc, positions }
    }

    /// Rebuild a replica from a [`snapshot`](Self::snapshot).
    pub fn from_snapshot(changes: &[Change]) -> Result<Replica, Error> {
        Self::from_snapshot_leaving_out(changes).map(|(r, _)| r)
    }

    /// [`from_snapshot`](Self::from_snapshot), and how many image entries it left out: a
    /// snapshot saved before the image checks may hold one they refuse, and opens without it.
    pub(crate) fn from_snapshot_leaving_out(changes: &[Change]) -> Result<(Replica, usize), Error> {
        let mut r = Replica { doc: Document::without_pages(""), positions: Positions::new() };
        let mut changes = changes.to_vec();
        let left_out = r.apply_kept(&mut changes, Change::is_image)?;
        if r.doc.pages.is_empty() {
            return Err(Error::Invalid("a snapshot needs a page".into()));
        }
        Ok((r, left_out))
    }

    /// Apply `changes` strictly. When they don't apply together — history written under older
    /// rules, or changes that apply one by one but fail the batch's checks together — apply those
    /// that do, one by one in order, and leave the rest out of `changes`, as long as `may_leave_out`
    /// allows it (otherwise that change's error). Returns how many were left out.
    pub(crate) fn apply_kept(&mut self, changes: &mut Vec<Change>, may_leave_out: impl Fn(&Change) -> bool) -> Result<usize, Error> {
        let mut all = self.clone();
        if all.apply(changes, Mode::Strict).is_ok() {
            *self = all;
            return Ok(0);
        }
        let given = changes.len();
        let mut kept = Vec::with_capacity(given);
        for c in changes.drain(..) {
            let mut t = self.clone();
            match t.apply(std::slice::from_ref(&c), Mode::Strict) {
                Ok(_) => {
                    *self = t;
                    kept.push(c);
                }
                Err(e) if !may_leave_out(&c) => return Err(e),
                Err(_) => {}
            }
        }
        let left_out = given - kept.len();
        *changes = kept;
        Ok(left_out)
    }

    /// The whole state as changes that build it from nothing (pages and their trees in order,
    /// then the document-level data).
    pub fn snapshot(&self) -> Result<Vec<Change>, Error> {
        let mut out = Vec::with_capacity(self.doc.node_count() + 8);
        let mut roots = self.doc.pages.clone();
        roots.extend(self.doc.nodes().filter(|n| n.parent.is_none() && !self.doc.pages.contains(&n.id)).map(|n| n.id));
        for r in roots {
            for id in self.doc.subtree(r) {
                if let Some(n) = self.doc.node(id) {
                    out.push(create_change(n, self.key(id))?);
                }
            }
        }
        docprops::diff(&Document::without_pages(""), &self.doc, &mut out)?;
        Ok(out)
    }

    /// The synced document.
    pub fn document(&self) -> &Document {
        &self.doc
    }

    /// Mutable access to the document for allocation settings (client id, id counter). Edits made
    /// here bypass syncing: make them with [`diff`](crate::diff) and [`apply`](Self::apply).
    pub fn document_mut(&mut self) -> &mut Document {
        &mut self.doc
    }

    /// Every node's position key.
    pub fn positions(&self) -> &Positions {
        &self.positions
    }

    /// Node `id`'s position key ("" when it has none, which sorts first).
    pub fn key(&self, id: NodeId) -> &str {
        self.positions.get(&id).map_or("", String::as_str)
    }

    /// The current value of `target` (`None`: its node or entry doesn't exist).
    pub fn value(&self, target: &Target) -> Result<Option<Json>, Error> {
        match target {
            Target::Node(id, prop) => {
                let Some(n) = self.doc.node(*id) else { return Ok(None) };
                match prop {
                    Prop::Parent => Json::of(&n.parent),
                    Prop::Position => Json::of(self.key(*id)),
                    p => p.get(n),
                }
                .map(Some)
            }
            Target::Doc(prop) => docprops::get(&self.doc, *prop).map(Some),
            Target::Entry(entry) => docprops::entry_value(&self.doc, entry),
        }
    }

    pub(crate) fn from_parts(doc: Document, positions: Positions) -> Replica {
        Replica { doc, positions }
    }

    /// Where `id` with `key` goes among `siblings` (sorted by `(key, id)`; `id` itself skipped):
    /// how many of them sort before it. A binary search on each side of `id`, which may still
    /// sit where its old key put it.
    fn slot(&self, siblings: &[NodeId], id: NodeId, key: &str) -> usize {
        let before = |c: &NodeId| (self.key(*c), *c) < (key, id);
        let (head, tail) = match siblings.iter().position(|c| *c == id) {
            Some(i) => (siblings.get(..i).unwrap_or_default(), siblings.get(i + 1..).unwrap_or_default()),
            None => (siblings, &[][..]),
        };
        head.partition_point(before) + tail.partition_point(before)
    }

    /// Apply `changes` in order. Node changes come first in a batch made by [`diff`](crate::diff);
    /// any order is accepted. Pages are re-listed by their keys at the end.
    pub fn apply(&mut self, changes: &[Change], mode: Mode) -> Result<Applied, Error> {
        self.doc.take_dirty();
        let mut out = Applied::default();
        let (mut relist, mut relist_all) = (BTreeSet::new(), false);
        for c in changes {
            match self.apply_one(c, &mut out) {
                Ok(Relist::No) => {}
                Ok(Relist::Page(p)) => {
                    relist.insert(p);
                }
                Ok(Relist::All) => relist_all = true,
                Err(e) if mode == Mode::Strict => return Err(e),
                Err(_) => out.skipped += 1,
            }
        }
        if relist_all || !relist.is_empty() {
            self.relist_pages(relist, relist_all);
        }
        out.touched = self.doc.take_dirty();
        if mode == Mode::Strict {
            self.check(&out.touched)?;
        }
        Ok(out)
    }

    /// Apply one change. Returns what of the page list needs rebuilding.
    fn apply_one(&mut self, c: &Change, out: &mut Applied) -> Result<Relist, Error> {
        match c {
            Change::Create { id, kind, parent, position, props } => {
                frac::validate(position).map_err(|e| Error::Invalid(e.to_string()))?;
                if self.doc.contains(*id) {
                    return Err(Error::Invalid(format!("node {id} already exists")));
                }
                let kind: NodeKind = kind.read("kind")?;
                let mut node = Node::new(*id, kind, "", 0.0, 0.0);
                for (p, v) in props {
                    if *p == Prop::Kind {
                        return Err(Error::Invalid("a created node's kind is its own field".into()));
                    }
                    p.set(&mut node, v)?;
                }
                let is_page = node.is_page();
                match (is_page, parent) {
                    (true, None) => {
                        self.doc.insert_page(node).map_err(doc_err)?;
                    }
                    (false, Some(p)) => {
                        let at = self.slot(&self.doc.get(*p).map_err(doc_err)?.children, *id, position);
                        self.doc.insert(node, *p, Some(at)).map_err(doc_err)?;
                    }
                    _ => return Err(Error::Invalid("pages, and only pages, have no parent".into())),
                }
                self.positions.insert(*id, position.clone());
                Ok(if is_page { Relist::Page(*id) } else { Relist::No })
            }
            Change::Set { id, prop: Prop::Parent, value } => {
                let parent: Option<NodeId> = value.read("parent")?;
                let node = self.doc.get(*id).map_err(doc_err)?;
                match (node.is_page(), parent) {
                    (true, None) => Ok(Relist::No),
                    (false, Some(p)) => {
                        let at = self.slot(&self.doc.get(p).map_err(doc_err)?.children, *id, self.key(*id));
                        self.doc.place(*id, p, at).map_err(doc_err)?;
                        Ok(Relist::No)
                    }
                    _ => Err(Error::Invalid("pages, and only pages, have no parent".into())),
                }
            }
            Change::Set { id, prop: Prop::Position, value } => {
                let key: String = value.read("position")?;
                frac::validate(&key).map_err(|e| Error::Invalid(e.to_string()))?;
                let node = self.doc.get(*id).map_err(doc_err)?;
                let parent = node.parent;
                self.positions.insert(*id, key.clone());
                match parent {
                    None => Ok(Relist::Page(*id)),
                    Some(p) => {
                        let at = self.slot(&self.doc.get(p).map_err(doc_err)?.children, *id, &key);
                        self.doc.reorder(*id, at).map_err(doc_err)?;
                        Ok(Relist::No)
                    }
                }
            }
            Change::Set { id, prop: Prop::Kind, value } => {
                let kind: NodeKind = value.read("kind")?;
                let node = self.doc.get_mut(*id).map_err(doc_err)?;
                if node.is_page() != matches!(kind, NodeKind::Page { .. }) {
                    return Err(Error::Invalid("a node can't become or stop being a page".into()));
                }
                node.kind = kind;
                Ok(Relist::No)
            }
            Change::Set { id, prop, value } => {
                let node = self.doc.get_mut(*id).map_err(doc_err)?;
                prop.set(node, value)?;
                Ok(Relist::No)
            }
            Change::Delete { id } => {
                let Some(n) = self.doc.node(*id) else { return Ok(Relist::No) };
                let page = n.is_page();
                let gone = self.doc.subtree(*id);
                self.doc.remove(*id).map_err(doc_err)?;
                for g in &gone {
                    self.positions.remove(g);
                }
                out.deleted.extend(gone);
                Ok(if page { Relist::Page(*id) } else { Relist::No })
            }
            Change::SetDoc { .. } | Change::Put { .. } | Change::Remove { .. } => {
                Ok(if docprops::apply(&mut self.doc, c)? { Relist::All } else { Relist::No })
            }
        }
    }

    /// List the pages (all but the library store) ordered by `(key, id)`: the listed ones and
    /// `extra`, or every page node when `all` (the library store may have changed).
    fn relist_pages(&mut self, extra: BTreeSet<NodeId>, all: bool) {
        let store = self.doc.libraries.store;
        let mut ids: BTreeSet<NodeId> = extra;
        ids.extend(self.doc.pages.iter().copied());
        if all {
            ids.extend(self.doc.nodes().filter(|n| n.parent.is_none()).map(|n| n.id));
        }
        let mut pages: Vec<NodeId> =
            ids.into_iter().filter(|p| Some(*p) != store && self.doc.node(*p).is_some_and(|n| n.is_page() && n.parent.is_none())).collect();
        pages.sort_by(|a, b| (self.key(*a), *a).cmp(&(self.key(*b), *b)));
        self.doc.pages = pages;
    }

    /// The checks a server makes on what a batch touched.
    fn check(&self, touched: &BTreeSet<NodeId>) -> Result<(), Error> {
        if self.doc.pages.is_empty() {
            return Err(Error::Invalid("a file needs at least one page".into()));
        }
        let finite = |v: f64| v.is_finite() && v.abs() <= MAX_COORD;
        for id in touched {
            let Some(n) = self.doc.node(*id) else { continue };
            if !n.children.is_empty() && !n.kind.is_container() {
                return Err(Error::Invalid(format!("{id} can't contain layers")));
            }
            if !(finite(n.width) && finite(n.height) && n.width >= 0.0 && n.height >= 0.0) {
                return Err(Error::Invalid(format!("{id} has an invalid size")));
            }
            if !n.transform.as_coeffs().into_iter().all(finite) {
                return Err(Error::Invalid(format!("{id} has an invalid transform")));
            }
            if !(n.opacity.is_finite() && (0.0..=1.0).contains(&n.opacity)) {
                return Err(Error::Invalid(format!("{id} has an invalid opacity")));
            }
        }
        Ok(())
    }
}

/// The change that creates node `n` as it is, at position `key`: its kind, place and the
/// properties that differ from a new node of its kind.
pub(crate) fn create_change(n: &Node, key: &str) -> Result<Change, Error> {
    let blank = Node::new(n.id, n.kind.clone(), "", 0.0, 0.0);
    let mut props = Vec::new();
    for p in Prop::FIELDS {
        if *p != Prop::Kind && p.differs(&blank, n) {
            props.push((*p, p.get(n)?));
        }
    }
    Ok(Change::Create { id: n.id, kind: Json::of(&n.kind)?, parent: n.parent, position: key.to_string(), props })
}

/// What a change needs of the page list.
enum Relist {
    No,
    /// This page was added, moved or removed.
    Page(NodeId),
    /// The library store may have changed: look at every root.
    All,
}

fn doc_err(e: figcraft_doc::DocError) -> Error {
    match e {
        figcraft_doc::DocError::NoNode(id) => Error::Unknown(id),
        other => Error::Invalid(other.to_string()),
    }
}
