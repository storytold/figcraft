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
#[derive(Clone, Debug)]
pub struct Replica {
    doc: Document,
    positions: Positions,
    /// Raw payloads for small, recent image undo steps. This is not synced state. Lazy images
    /// stay in the disk cache instead; cloning a replica only clones persistent-map pointers.
    assets: imbl::OrdMap<figcraft_assets::AssetRef, std::sync::Arc<Vec<u8>>>,
}

impl PartialEq for Replica {
    fn eq(&self, other: &Self) -> bool {
        self.doc == other.doc && self.positions == other.positions
    }
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
        let mut replica = Replica { doc, positions, assets: imbl::OrdMap::new() };
        replica.remember_images();
        replica
    }

    /// Rebuild a replica from a [`snapshot`](Self::snapshot).
    pub fn from_snapshot(changes: &[Change]) -> Result<Replica, Error> {
        let mut r = Replica { doc: Document::without_pages(""), positions: Positions::new(), assets: imbl::OrdMap::new() };
        r.apply(changes, Mode::Strict)?;
        if r.doc.pages.is_empty() {
            return Err(Error::Invalid("a snapshot needs a page".into()));
        }
        Ok(r)
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

    pub fn asset_payload(&self, asset: &figcraft_assets::AssetRef) -> Option<std::sync::Arc<Vec<u8>>> {
        self.assets.get(asset).cloned()
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
        let mut replica = Replica { doc, positions, assets: imbl::OrdMap::new() };
        replica.remember_images();
        replica
    }

    pub(crate) fn from_parts_cached(doc: Document, positions: Positions, previous: &Replica) -> Replica {
        let mut replica = Replica { doc, positions, assets: previous.assets.clone() };
        replica.remember_images();
        replica
    }

    fn remember_images(&mut self) {
        let mut current = BTreeSet::new();
        for image in self.doc.images.values() {
            let reference = image.reference();
            current.insert(reference.clone());
            if image.asset.is_none() {
                self.assets.insert(reference, image.bytes.clone());
            }
        }
        let old: Vec<_> =
            self.assets.iter().filter(|(key, _)| !current.contains(*key)).map(|(key, bytes)| (key.clone(), bytes.len() as u64)).collect();
        let mut held: u64 = old.iter().map(|(_, size)| *size).sum();
        for (key, size) in old {
            if held <= 128 << 20 {
                break;
            }
            self.assets.remove(&key);
            held = held.saturating_sub(size);
        }
    }

    /// Where `id` with `key` goes among `siblings` (sorted by `(key, id)`; `id` itself skipped):
    /// how many of them sort before it.
    fn slot(&self, siblings: &[NodeId], id: NodeId, key: &str) -> usize {
        siblings.iter().filter(|c| **c != id && (self.key(**c), **c) < (key, id)).count()
    }

    /// Apply `changes` in order. Node changes come first in a batch made by [`diff`](crate::diff);
    /// any order is accepted. Pages are re-listed by their keys at the end.
    pub fn apply(&mut self, changes: &[Change], mode: Mode) -> Result<Applied, Error> {
        self.doc.take_dirty();
        let mut out = Applied::default();
        let (mut relist, mut relist_all) = (BTreeSet::new(), false);
        for c in changes {
            let new_image = match c {
                Change::Image(image) => Some(&image.key),
                Change::Put { entry: crate::Entry::Image(key), .. } => Some(key),
                _ => None,
            };
            if new_image.is_some_and(|key| !self.doc.images.contains_key(key)) && self.doc.images.len() >= 20_000 {
                if mode == Mode::Strict {
                    return Err(Error::Invalid("too many images in the document (at most 20000)".into()));
                }
                out.skipped += 1;
                continue;
            }
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
        self.remember_images();
        if mode == Mode::Strict {
            if self.doc.images.len() > 20_000 {
                return Err(Error::Invalid("too many images in the document (at most 20000)".into()));
            }
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
            Change::Image(image) => {
                let mut data = image.data()?;
                if data.asset.is_some()
                    && let Some(bytes) = self.assets.get(&image.asset)
                {
                    data.bytes = bytes.clone();
                    data.asset = None;
                }
                self.doc.images.insert(image.key.clone(), data);
                if let Some(bytes) = &image.bytes {
                    self.assets.insert(image.asset.clone(), bytes.0.clone());
                }
                Ok(Relist::No)
            }
            Change::SetDoc { .. } | Change::Put { .. } | Change::Remove { .. } => {
                let all = docprops::apply(&mut self.doc, c)?;
                if let Change::Put { entry: crate::Entry::Image(key), .. } = c {
                    if let Some(image) = self.doc.images.get_mut(key)
                        && let Some(bytes) = self.assets.get(&image.reference())
                    {
                        image.bytes = bytes.clone();
                        image.asset = None;
                    }
                    if let Some(image) = self.doc.images.get(key)
                        && image.asset.is_none()
                    {
                        self.assets.insert(image.reference(), image.bytes.clone());
                    }
                }
                Ok(if all { Relist::All } else { Relist::No })
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
