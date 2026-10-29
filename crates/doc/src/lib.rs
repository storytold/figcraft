//! The FigCraft document model.
//!
//! A [`Document`] holds pages; each page is the root of a tree of [`Node`]s (frames, groups, shapes,
//! text, components, instances…). Nodes are stored by id in a map of `Arc`s, so cloning a document
//! (an undo snapshot) only copies pointers, and editing one node copies only that node.
//!
//! Everything here is pure data plus queries and structural edits; commands, history and rendering
//! live in higher crates.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod board;
pub mod colors;
pub mod comments;
pub mod facilitation;
pub mod geometry;
pub mod handoff;
pub mod ids;
pub mod library;
pub mod node;
pub mod props;
pub mod proto;
pub mod styles;
pub mod text;
pub mod vector;
pub mod vector_faces;
pub mod vector_regions;

pub use board::*;
pub use colors::*;
pub use comments::*;
pub use facilitation::*;
pub use figcraft_color as color;
pub use figcraft_geom as geom;
pub use handoff::*;
pub use library::*;
pub use node::*;
pub use props::*;
pub use styles::*;
pub use text::*;
pub use vector::*;
pub use vector_faces::Face;

use figcraft_color::{Color, ColorProfile};
use figcraft_geom::{Affine, Point, Rect};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Deepest descendant [`Document::world_render_bounds`] follows: as deep as the renderer draws
/// (`figcraft_render::MAX_EXPORT_DEPTH`), so a crafted chain can't overflow the stack (#1062).
const MAX_RENDER_DEPTH: usize = 256;

/// Stable node identifier, unique within a document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub u64);

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for NodeId {
    type Err = std::num::ParseIntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Accept "12" and Figma-style "0:12".
        let t = s.trim();
        let t = t.rsplit(':').next().unwrap_or(t);
        if t.starts_with('+') {
            // `u64::parse` accepts a leading '+'; ids are digits only.
            return "-".parse::<u64>().map(NodeId);
        }
        t.parse::<u64>().map(NodeId)
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum DocError {
    #[error("no such node {0}")]
    NoNode(NodeId),
    #[error("{0} can't contain children")]
    NotContainer(NodeId),
    #[error("moving {0} into {1} would create a cycle")]
    Cycle(NodeId, NodeId),
    #[error("{0}")]
    Invalid(String),
}

pub type DocResult<T> = Result<T, DocError>;

/// The set of touched node ids, whether any layer is a mask (hit testing skips its mask checks
/// without one) and the largest background blur radius (renders keep that much overlap between
/// their bands), both worked out when first asked after a change. Never part of equality or
/// serialisation.
#[derive(Clone, Debug, Default)]
pub struct Dirty(pub std::collections::BTreeSet<NodeId>, std::sync::OnceLock<bool>, std::sync::OnceLock<f64>);

impl Dirty {
    /// Record node `id` as touched (edited, added or removed).
    fn touch(&mut self, id: NodeId) {
        self.0.insert(id);
        self.1 = std::sync::OnceLock::new();
        self.2 = std::sync::OnceLock::new();
    }
}

impl PartialEq for Dirty {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// Hands out node ids. A single-user document counts up through the whole id range; in a live
/// collaboration session each participant allocates in its own space, `client << 32 | counter`, so
/// ids made at the same time on different machines never collide. Saved as the file's `next_id`;
/// the client id is never saved. Never part of equality: two documents with the same content are
/// equal however far their counters went.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(transparent)]
struct IdAlloc {
    /// The next id to hand out.
    next: u64,
    /// The client whose space ids come from (0: the whole range, as in a single-user file).
    #[serde(skip)]
    client: u32,
}

impl PartialEq for IdAlloc {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl IdAlloc {
    /// Whether `id` lies in the space this allocator hands out (any id without a client).
    fn owns(&self, id: NodeId) -> bool {
        self.client == 0 || id.0 >> 32 == u64::from(self.client)
    }

    /// Never hand out `id` (or anything below it in this space) again.
    fn reserve(&mut self, id: NodeId) {
        if self.owns(id) && id.0 >= self.next {
            self.next = id.0.saturating_add(1);
        }
    }

    fn alloc(&mut self) -> NodeId {
        let id = NodeId(self.next);
        let next = self.next.saturating_add(1);
        // A client space holds 2^32 ids; a full one keeps returning its last id (an insert then
        // fails as a duplicate) rather than spilling into another client's space.
        if self.owns(NodeId(next)) {
            self.next = next;
        }
        id
    }
}

/// Raw image bytes (encoded PNG/JPEG/…) kept by content hash.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageData {
    pub bytes: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
}

impl ImageData {
    /// Largest encoded image a document takes from outside: an image member of a `.fig` or
    /// `.sketch` file, an image file a REST import reads, an image entry from a live session.
    pub const MAX_BYTES: u64 = 256 * 1024 * 1024;

    /// Whether `key` claims to be a content hash: 64 hex digits, the shape of the sha256 hex
    /// `figcraft_format::image_hash` makes. Bytes stored under such a key must hash to it.
    pub fn claims_content_hash(key: &str) -> bool {
        key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit())
    }

    /// Whether `key` can name an image: 1–128 ASCII letters, digits, `-` and `_` (content hashes,
    /// the svg importer's FNV keys, the REST importer's `rest-image-missing` placeholder). A key
    /// becomes an archive member name (`images/<key>`) when the file is saved, so only these are
    /// read back from a file or taken from a collaborator: never `/`, `.` or other path text.
    pub fn is_valid_key(key: &str) -> bool {
        (1..=128).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    }
}

/// A whole design file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub name: String,
    /// A design file or a board (files from before boards are design files).
    #[serde(default, skip_serializing_if = "FileKind::is_design")]
    pub kind: FileKind,
    /// Page ids in order.
    pub pages: Vec<NodeId>,
    nodes: imbl::OrdMap<NodeId, Arc<Node>>,
    /// Nodes touched since the last [`take_dirty`](Self::take_dirty) (edited, added or removed).
    #[serde(skip)]
    dirty: Dirty,
    next_id: IdAlloc,
    #[serde(default)]
    pub styles: Vec<Style>,
    #[serde(default)]
    pub variables: Variables,
    /// Images by content hash; serialized separately by the format crate.
    #[serde(skip)]
    pub images: BTreeMap<String, ImageData>,
    /// The page shown when the file opens.
    #[serde(default)]
    pub current_page: Option<NodeId>,
    /// Colours used recently / saved in "Document colors".
    #[serde(default)]
    pub document_colors: Vec<Color>,
    /// Comment threads on any page.
    #[serde(default)]
    pub comments: Vec<Comment>,
    /// Enabled libraries and the components and styles copied from them.
    #[serde(default, skip_serializing_if = "Libraries::is_unused")]
    pub libraries: Arc<Libraries>,
    /// This file's published library; serialized separately by the format crate.
    #[serde(skip)]
    pub published: Option<Arc<PublishedLibrary>>,
    /// The colour space colour values are in (sRGB unless the file says otherwise).
    #[serde(default, skip_serializing_if = "ColorProfile::is_srgb")]
    pub color_profile: ColorProfile,
    /// Image adjustment presets saved in this file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub image_presets: Vec<ImagePreset>,
    /// Annotation categories (`None`: [`AnnotationCategory::defaults`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotation_categories: Option<Vec<AnnotationCategory>>,
    /// What ready-for-dev frames and sections looked like before they changed (by layer; see
    /// [`handoff`]).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dev_snapshots: BTreeMap<NodeId, Arc<DevSnapshot>>,
    /// A board's countdown (see [`facilitation`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timer: Option<BoardTimer>,
    /// A board's vote, running or ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vote: Option<Vote>,
    /// The vote's ballots, by voter.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub votes: Ballots,
}

impl Default for Document {
    fn default() -> Self {
        Self::new("Untitled")
    }
}

impl Document {
    /// A new file with one empty page ("Page 1").
    pub fn new(name: impl Into<String>) -> Self {
        let mut doc = Self::without_pages(name);
        let p = doc.add_page("Page 1");
        doc.current_page = Some(p);
        doc
    }

    /// A document with no pages at all: not a valid file on its own (see [`repair`](Self::repair)),
    /// only a base to build one onto, e.g. by applying a collaboration snapshot.
    pub fn without_pages(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: FileKind::Design,
            pages: Vec::new(),
            nodes: imbl::OrdMap::new(),
            dirty: Dirty::default(),
            next_id: IdAlloc { next: 1, client: 0 },
            styles: Vec::new(),
            variables: Variables::default(),
            images: BTreeMap::new(),
            current_page: None,
            document_colors: Vec::new(),
            comments: Vec::new(),
            libraries: Arc::default(),
            published: None,
            color_profile: ColorProfile::Srgb,
            image_presets: Vec::new(),
            annotation_categories: None,
            dev_snapshots: BTreeMap::new(),
            timer: None,
            vote: None,
            votes: BTreeMap::new(),
        }
    }

    /// Node ids touched since the last call (edited, added or removed), clearing the record.
    pub fn take_dirty(&mut self) -> std::collections::BTreeSet<NodeId> {
        std::mem::take(&mut self.dirty.0)
    }

    /// Record `ids` as touched, for passes that follow the dirty set (e.g. after changes made
    /// elsewhere were applied).
    pub fn mark_dirty(&mut self, ids: impl IntoIterator<Item = NodeId>) {
        self.dirty.0.extend(ids);
    }

    /// Whether any layer of the document is a mask.
    pub fn has_masks(&self) -> bool {
        *self.dirty.1.get_or_init(|| self.nodes.values().any(|n| n.is_mask))
    }

    /// The largest background blur or glass radius of any layer's effects (0 without any).
    pub fn max_backdrop_radius(&self) -> f64 {
        *self.dirty.2.get_or_init(|| self.nodes.values().flat_map(|n| n.effects.iter()).map(Effect::backdrop_radius).fold(0.0, f64::max))
    }

    /// Node ids touched so far (without clearing).
    pub fn dirty(&self) -> &std::collections::BTreeSet<NodeId> {
        &self.dirty.0
    }

    /// A fresh unused id (in this document's client space during a collaboration session; see
    /// [`set_client_id`](Self::set_client_id)).
    pub fn alloc_id(&mut self) -> NodeId {
        // A file's saved `next_id` can be stale (behind ids already in use): skip taken ids.
        let mut id = self.next_id.alloc();
        while self.nodes.contains_key(&id) {
            let next = self.next_id.alloc();
            if next == id {
                break;
            }
            id = next;
        }
        id
    }

    /// The id [`alloc_id`](Self::alloc_id) returns next.
    pub fn next_id(&self) -> NodeId {
        NodeId(self.next_id.next)
    }

    /// The collaboration client ids are allocated for (0: none, a single-user document).
    pub fn client_id(&self) -> u32 {
        self.next_id.client
    }

    /// Allocate new ids as `client << 32 | counter`, continuing after the highest id of that space
    /// already in the document, so participants of a live session never make the same id. 0 goes
    /// back to single-user allocation (after the highest id of any space).
    pub fn set_client_id(&mut self, client: u32) {
        let (lo, hi) = if client == 0 { (0, u64::MAX) } else { (u64::from(client) << 32, (u64::from(client) << 32) | u64::from(u32::MAX)) };
        let highest = self.nodes.range(NodeId(lo)..=NodeId(hi)).next_back().map(|(k, _)| *k);
        // Re-setting the same client keeps its counter (ids handed out stay handed out).
        let keep = if self.next_id.client == client { self.next_id.next } else { 0 };
        let start = if client == 0 { keep.max(1) } else { keep.max(lo | 1) };
        self.next_id = IdAlloc { next: start, client };
        if let Some(h) = highest {
            self.next_id.reserve(h);
        }
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id).map(Arc::as_ref)
    }

    pub fn node_arc(&self, id: NodeId) -> Option<&Arc<Node>> {
        self.nodes.get(&id)
    }

    /// Mutable access (copy-on-write when a snapshot shares the node).
    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        let n = self.nodes.get_mut(&id).map(Arc::make_mut);
        if n.is_some() {
            self.dirty.touch(id);
        }
        n
    }

    /// Undo a write that changed nothing: when node `old.id` equals `old` by value, `old` goes back
    /// in its place (shared with the snapshots holding it again) and, unless `was_dirty`, the node
    /// leaves the dirty set. Returns whether it did.
    pub fn keep_if_unchanged(&mut self, old: Arc<Node>, was_dirty: bool) -> bool {
        let id = old.id;
        let same = self.nodes.get(&id).is_some_and(|n| Arc::ptr_eq(n, &old) || **n == *old);
        if same {
            self.nodes.insert(id, old);
            if !was_dirty {
                self.dirty.0.remove(&id);
            }
        }
        same
    }

    pub fn get(&self, id: NodeId) -> DocResult<&Node> {
        self.node(id).ok_or(DocError::NoNode(id))
    }

    pub fn get_mut(&mut self, id: NodeId) -> DocResult<&mut Node> {
        self.node_mut(id).ok_or(DocError::NoNode(id))
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(&id)
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Ids of the nodes added, removed or changed (by value) since `before`, ascending, or `None`
    /// when more than `limit` differ. Structure the two versions share is skipped, so comparing an
    /// edit with the version it came from costs about the size of the edit, not of the document.
    pub fn changed_ids(&self, before: &Document, limit: usize) -> Option<Vec<NodeId>> {
        let mut out = Vec::new();
        for d in before.nodes.diff(&self.nodes) {
            if out.len() >= limit {
                return None;
            }
            out.push(match d {
                imbl::ordmap::DiffItem::Add(k, _) | imbl::ordmap::DiffItem::Remove(k, _) => *k,
                imbl::ordmap::DiffItem::Update { new: (k, _), .. } => *k,
            });
        }
        Some(out)
    }

    /// Every node id (pages included), ascending.
    pub fn ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes.keys().copied()
    }

    pub fn nodes(&self) -> impl Iterator<Item = &Node> + '_ {
        self.nodes.values().map(Arc::as_ref)
    }

    /// Add a page at the end.
    pub fn add_page(&mut self, name: &str) -> NodeId {
        let id = self.alloc_id();
        let node =
            Node::new(id, NodeKind::Page { background: Color::DEFAULT_PAGE, flows: Vec::new(), prototype: Default::default() }, name, 0.0, 0.0);
        self.nodes.insert(id, Arc::new(node));
        self.dirty.touch(id);
        self.pages.push(id);
        id
    }

    /// The page a node is on (a page is on itself).
    pub fn page_of(&self, id: NodeId) -> Option<NodeId> {
        let mut cur = id;
        for _ in 0..10_000 {
            let n = self.node(cur)?;
            if n.is_page() {
                return Some(cur);
            }
            cur = n.parent?;
        }
        None
    }

    pub fn first_page(&self) -> Option<NodeId> {
        self.pages.first().copied()
    }

    /// Whether page `id` is a divider in the pages list: an empty page whose name starts with a
    /// hyphen. Dividers separate pages; they aren't opened.
    pub fn is_divider(&self, id: NodeId) -> bool {
        self.node(id).is_some_and(|n| n.is_page() && n.children.is_empty() && n.name.starts_with('-'))
    }

    /// The first page that isn't a divider.
    pub fn first_open_page(&self) -> Option<NodeId> {
        self.pages.iter().copied().find(|p| !self.is_divider(*p))
    }

    /// Insert `node` (its id must come from [`alloc_id`](Self::alloc_id)) as a child of `parent` at
    /// `index` (None = on top).
    pub fn insert(&mut self, mut node: Node, parent: NodeId, index: Option<usize>) -> DocResult<NodeId> {
        let p = self.get(parent)?;
        if !p.kind.is_container() {
            return Err(DocError::NotContainer(parent));
        }
        let id = node.id;
        if self.nodes.contains_key(&id) {
            return Err(DocError::Invalid(format!("node {id} already exists")));
        }
        self.reserve_ids(id);
        node.parent = Some(parent);
        self.nodes.insert(id, Arc::new(node));
        self.dirty.touch(id);
        let p = self.get_mut(parent)?;
        let at = index.unwrap_or(p.children.len()).min(p.children.len());
        p.children.insert(at, id);
        Ok(id)
    }

    /// Create a node of `kind` under `parent`, positioned at `(x, y)` in the parent's space.
    pub fn create(&mut self, parent: NodeId, kind: NodeKind, name: &str, x: f64, y: f64, w: f64, h: f64) -> DocResult<NodeId> {
        let id = self.alloc_id();
        let mut node = Node::new(id, kind, name, w, h);
        node.transform = Affine::translate((x, y));
        self.insert(node, parent, None)
    }

    /// Whether `ancestor` is `id` or one of its ancestors.
    pub fn is_ancestor(&self, ancestor: NodeId, id: NodeId) -> bool {
        let mut cur = Some(id);
        let mut guard = 0;
        while let Some(c) = cur {
            if c == ancestor {
                return true;
            }
            guard += 1;
            // More hops than nodes means the parent chain loops; fail closed so callers reject the move.
            if guard > self.nodes.len() {
                return true;
            }
            cur = self.node(c).and_then(|n| n.parent);
        }
        false
    }

    /// Ancestors from the parent up to (and including) the page.
    pub fn ancestors(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut cur = self.node(id).and_then(|n| n.parent);
        while let Some(c) = cur {
            if out.len() > 10_000 {
                break;
            }
            out.push(c);
            cur = self.node(c).and_then(|n| n.parent);
        }
        out
    }

    /// `id` and all its descendants, pre-order.
    pub fn subtree(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(c) = stack.pop() {
            if out.len() > 5_000_000 {
                break;
            }
            let Some(n) = self.node(c) else { continue };
            out.push(c);
            stack.extend(n.children.iter().rev());
        }
        out
    }

    /// Remove a node and its subtree. Pages can only be removed while another page remains.
    pub fn remove(&mut self, id: NodeId) -> DocResult<()> {
        let node = self.get(id)?;
        let parent = node.parent;
        if node.is_page() {
            if self.pages.len() <= 1 {
                return Err(DocError::Invalid("a file needs at least one page".into()));
            }
            self.pages.retain(|p| *p != id);
            if self.current_page == Some(id) {
                self.current_page = self.pages.first().copied();
            }
        }
        if let Some(parent) = parent
            && let Some(p) = self.node_mut(parent)
        {
            p.children.retain(|c| *c != id);
        }
        for d in self.subtree(id) {
            self.nodes.remove(&d);
            self.dirty.touch(d);
        }
        Ok(())
    }

    /// Whether sections may go in `parent`: a page or another section (never a frame or a group).
    pub fn takes_sections(&self, parent: NodeId) -> bool {
        self.node(parent).is_some_and(|p| p.is_page() || matches!(p.kind, NodeKind::Section { .. }))
    }

    /// `id`'s transform in the space of the frame it is in (or of the page): groups and boolean
    /// groups have no coordinate space of their own, so theirs fold in. X and Y are measured there.
    pub fn frame_relative(&self, id: NodeId) -> Affine {
        let Some(n) = self.node(id) else { return Affine::IDENTITY };
        let mut t = n.transform;
        let mut parent = n.parent;
        while let Some(p) = parent.and_then(|p| self.node(p)).filter(|p| p.kind.bounds_follow_children()) {
            t = p.transform * t;
            parent = p.parent;
        }
        t
    }

    /// The children of `n` that make up its box (a group's bounds, an export's area): all of them,
    /// except layers a visible mask below them masks, whose part is the mask's.
    pub fn box_children<'a>(&'a self, n: &'a Node) -> impl Iterator<Item = (NodeId, &'a Node)> + 'a {
        let mut masked = false;
        n.children.iter().filter_map(|c| self.node(*c).map(|k| (*c, k))).filter(move |(_, k)| {
            if k.is_mask {
                masked = k.visible;
                return true;
            }
            !masked
        })
    }

    /// Index of `id` among its siblings.
    pub fn index_in_parent(&self, id: NodeId) -> Option<usize> {
        let parent = self.node(id)?.parent?;
        self.node(parent)?.children.iter().position(|c| *c == id)
    }

    /// Move `id` under `new_parent` at `index` (None = top), keeping its world position.
    pub fn reparent(&mut self, id: NodeId, new_parent: NodeId, index: Option<usize>) -> DocResult<()> {
        if self.is_ancestor(id, new_parent) {
            return Err(DocError::Cycle(id, new_parent));
        }
        let np = self.get(new_parent)?;
        if !np.kind.is_container() {
            return Err(DocError::NotContainer(new_parent));
        }
        let node = self.get(id)?;
        if node.is_page() {
            return Err(DocError::Invalid("pages can't be moved into nodes".into()));
        }
        if matches!(node.kind, NodeKind::Section { .. }) && !self.takes_sections(new_parent) {
            return Err(DocError::Invalid("sections go on the page or in other sections, not in frames or groups".into()));
        }
        let world = self.world_transform(id);
        let old_parent = node.parent;
        let old_index = self.index_in_parent(id);
        if let Some(op) = old_parent
            && let Some(p) = self.node_mut(op)
        {
            p.children.retain(|c| *c != id);
        }
        let parent_world = self.world_transform(new_parent);
        let np = self.get_mut(new_parent)?;
        let mut at = index.unwrap_or(np.children.len());
        // Moving within the same parent: the removal shifted later indices.
        if old_parent == Some(new_parent)
            && let (Some(oi), Some(_)) = (old_index, index)
            && oi < at
        {
            at = at.saturating_sub(1);
        }
        let at = at.min(np.children.len());
        np.children.insert(at, id);
        let n = self.get_mut(id)?;
        n.parent = Some(new_parent);
        n.transform = parent_world.inverse() * world;
        Ok(())
    }

    /// Add page node `node` (its id unused; no parent, no children) at the end of the page list.
    pub fn insert_page(&mut self, mut node: Node) -> DocResult<NodeId> {
        let id = node.id;
        if !node.is_page() {
            return Err(DocError::Invalid(format!("{id} is not a page")));
        }
        if self.nodes.contains_key(&id) {
            return Err(DocError::Invalid(format!("node {id} already exists")));
        }
        self.reserve_ids(id);
        node.parent = None;
        node.children.clear();
        self.nodes.insert(id, Arc::new(node));
        self.dirty.touch(id);
        self.pages.push(id);
        Ok(id)
    }

    /// Move `id` under `parent` at `index` (clamped) keeping its local transform, unlike
    /// [`reparent`](Self::reparent), which keeps its place on the page.
    pub fn place(&mut self, id: NodeId, parent: NodeId, index: usize) -> DocResult<()> {
        if self.get(id)?.is_page() {
            return Err(DocError::Invalid("pages can't be moved into nodes".into()));
        }
        if !self.get(parent)?.kind.is_container() {
            return Err(DocError::NotContainer(parent));
        }
        if self.is_ancestor(id, parent) {
            return Err(DocError::Cycle(id, parent));
        }
        if let Some(old) = self.get(id)?.parent
            && let Some(p) = self.node_mut(old)
        {
            p.children.retain(|c| *c != id);
        }
        let p = self.get_mut(parent)?;
        let at = index.min(p.children.len());
        p.children.insert(at, id);
        self.get_mut(id)?.parent = Some(parent);
        Ok(())
    }

    /// Reorder `id` within its parent to `index`.
    pub fn reorder(&mut self, id: NodeId, index: usize) -> DocResult<()> {
        let parent = self.get(id)?.parent.ok_or(DocError::Invalid("page".into()))?;
        let p = self.get_mut(parent)?;
        p.children.retain(|c| *c != id);
        let at = index.min(p.children.len());
        p.children.insert(at, id);
        Ok(())
    }

    /// Transform from `id`'s local space to its page.
    pub fn world_transform(&self, id: NodeId) -> Affine {
        let mut xf = Affine::IDENTITY;
        let mut cur = Some(id);
        let mut guard = 0;
        while let Some(c) = cur {
            let Some(n) = self.node(c) else { break };
            if n.is_page() {
                break;
            }
            xf = n.transform * xf;
            cur = n.parent;
            guard += 1;
            if guard > 10_000 {
                break;
            }
        }
        xf
    }

    /// Set `id`'s transform so that its world transform becomes `world`.
    pub fn set_world_transform(&mut self, id: NodeId, world: Affine) -> DocResult<()> {
        let parent = self.get(id)?.parent;
        let pw = parent.map(|p| self.world_transform(p)).unwrap_or(Affine::IDENTITY);
        self.get_mut(id)?.transform = pw.inverse() * world;
        Ok(())
    }

    /// Axis-aligned bounds of the node's box in page coordinates.
    pub fn world_bounds(&self, id: NodeId) -> Option<Rect> {
        let n = self.node(id)?;
        if n.is_page() {
            return self.page_content_bounds(id);
        }
        Some(figcraft_geom::transform_rect_bbox(self.world_transform(id), n.local_rect()))
    }

    /// Bounds of everything painted by a node, effects and strokes included (page coordinates):
    /// a clipping frame's own area, else that and what its visible children show (masked
    /// children only within their mask). A group is what its children show, grown by its effects.
    pub fn world_render_bounds(&self, id: NodeId) -> Option<Rect> {
        self.render_bounds_at(id, 0)
    }

    fn render_bounds_at(&self, id: NodeId, depth: usize) -> Option<Rect> {
        let n = self.node(id)?;
        let own = n.own_paint_bounds(self.world_transform(id));
        if (n.clip && n.kind.is_frame_like() && !n.kind.bounds_follow_children()) || depth >= MAX_RENDER_DEPTH {
            return Some(own);
        }
        Some(match (self.children_render_bounds(n, depth + 1), &n.kind) {
            (Some(kids), NodeKind::Group) => n.grow_by_effects(kids),
            (Some(kids), _) => own.union(kids),
            (None, _) => own,
        })
    }

    /// What `n`'s visible children show, together: each mask's run within the mask's bounds.
    fn children_render_bounds(&self, n: &Node, depth: usize) -> Option<Rect> {
        let shown = |i: &NodeId| self.node(*i).is_some_and(|c| c.visible && !c.is_mask);
        let mut out: Option<Rect> = None;
        for run in self.paint_runs(n) {
            let ids = n.children.get(run).unwrap_or(&[]);
            let mask = ids.first().copied().filter(|m| self.node(*m).is_some_and(|c| c.is_mask && c.visible));
            let r = ids.iter().filter(|i| shown(i)).filter_map(|i| self.render_bounds_at(*i, depth)).reduce(|a, b| a.union(b));
            let r = match mask {
                Some(m) => r.zip(self.render_bounds_at(m, depth)).map(|(a, b)| a.intersect(b)).filter(|x| x.width() > 0.0 && x.height() > 0.0),
                None => r,
            };
            out = match (out, r) {
                (Some(a), Some(b)) => Some(a.union(b)),
                (a, b) => a.or(b),
            };
        }
        out
    }

    /// Union of the top-level nodes' bounds on a page.
    pub fn page_content_bounds(&self, page: NodeId) -> Option<Rect> {
        let p = self.node(page)?;
        p.children.iter().filter_map(|c| self.world_bounds(*c)).reduce(|a, b| a.union(b))
    }

    /// Union of several nodes' world bounds.
    pub fn bounds_of(&self, ids: &[NodeId]) -> Option<Rect> {
        ids.iter().filter_map(|i| self.world_bounds(*i)).reduce(|a, b| a.union(b))
    }

    /// Resize a group/boolean so its box tightly wraps its children (children keep world position).
    pub fn fit_to_children(&mut self, id: NodeId) -> DocResult<()> {
        let n = self.get(id)?;
        if n.children.is_empty() {
            return Ok(());
        }
        let world = self.world_transform(id);
        let inv = world.inverse();
        // Children bounds in the group's current local space (a mask stands for what it masks).
        let mut local: Option<Rect> = None;
        for (c, cn) in self.box_children(n) {
            let r = figcraft_geom::transform_rect_bbox(inv * self.world_transform(c), cn.local_rect());
            local = Some(local.map_or(r, |l| l.union(r)));
        }
        let Some(local) = local else { return Ok(()) };
        let children = n.children.clone();
        let child_worlds: Vec<(NodeId, Affine)> = children.iter().map(|c| (*c, self.world_transform(*c))).collect();
        let shift = Affine::translate((local.x0, local.y0));
        let nw = world * shift;
        {
            let g = self.get_mut(id)?;
            g.transform *= shift;
            g.width = local.width();
            g.height = local.height();
        }
        let ninv = nw.inverse();
        for (c, w) in child_worlds {
            if let Some(cn) = self.node_mut(c) {
                cn.transform = ninv * w;
            }
        }
        Ok(())
    }

    /// Fit every group/boolean ancestor of `id` (innermost first).
    pub fn fit_ancestors(&mut self, id: NodeId) {
        for a in self.ancestors(id) {
            let follows = self.node(a).is_some_and(|n| n.kind.bounds_follow_children());
            if follows {
                let _ = self.fit_to_children(a);
            } else {
                break;
            }
        }
    }

    /// Deep-copy `id`'s subtree under `parent` (at `index`), returning the new root id. The copy
    /// keeps the original's local transform; ids are fresh.
    pub fn duplicate_into(&mut self, id: NodeId, parent: NodeId, index: Option<usize>) -> DocResult<NodeId> {
        let mut map = BTreeMap::new();
        let src_nodes: Vec<Node> = self.subtree(id).into_iter().filter_map(|i| self.node(i).cloned()).collect();
        for n in &src_nodes {
            let nid = self.alloc_id();
            map.insert(n.id, nid);
        }
        self.paste_nodes(&src_nodes, &map, id, parent, index)
    }

    /// Deep-copy each of `ids`' subtrees to the end of `parent`, in order, returning the new root
    /// ids. One id map covers every copy, so references between the copied subtrees (a navigation
    /// from one copied frame to another) point at the copies.
    pub fn duplicate_all_into(&mut self, ids: &[NodeId], parent: NodeId) -> DocResult<Vec<NodeId>> {
        let mut map = BTreeMap::new();
        let mut trees = Vec::with_capacity(ids.len());
        for &id in ids {
            let src_nodes: Vec<Node> = self.subtree(id).into_iter().filter_map(|i| self.node(i).cloned()).collect();
            for n in &src_nodes {
                let nid = self.alloc_id();
                map.insert(n.id, nid);
            }
            trees.push((id, src_nodes));
        }
        trees.into_iter().map(|(id, src_nodes)| self.paste_nodes(&src_nodes, &map, id, parent, None)).collect()
    }

    /// Insert copies of `nodes` (a subtree rooted at `root`, from this or another document) with ids
    /// remapped by `map`. Interactions, instance references and pattern sources pointing inside the
    /// subtree are remapped too.
    pub fn paste_nodes(
        &mut self,
        nodes: &[Node],
        map: &BTreeMap<NodeId, NodeId>,
        root: NodeId,
        parent: NodeId,
        index: Option<usize>,
    ) -> DocResult<NodeId> {
        let p = self.get(parent)?;
        if !p.kind.is_container() {
            return Err(DocError::NotContainer(parent));
        }
        let new_root = *map.get(&root).ok_or(DocError::NoNode(root))?;
        // Never overwrite an existing node, or another copy, with a copy.
        let mut copied = std::collections::BTreeSet::new();
        if let Some(taken) = nodes.iter().filter_map(|n| map.get(&n.id)).find(|i| self.nodes.contains_key(*i) || !copied.insert(**i)) {
            return Err(DocError::Invalid(format!("node {taken} already exists")));
        }
        for n in nodes {
            let Some(new_id) = map.get(&n.id) else { continue };
            let mut c = n.clone();
            c.id = *new_id;
            c.children = n.children.iter().filter_map(|k| map.get(k).copied()).collect();
            c.parent = if n.id == root { Some(parent) } else { n.parent.and_then(|p| map.get(&p).copied()) };
            for it in &mut c.interactions {
                proto::walk_mut(&mut it.actions, &mut |a| remap_action(a, map));
            }
            if let NodeKind::Instance { component, .. } = &mut c.kind
                && let Some(m) = map.get(component)
            {
                *component = *m;
            }
            // A measurement copied with the layer it measures to measures to the copy; one to a
            // layer left behind isn't copied.
            c.measurements.retain_mut(|m| match m.to {
                Some(t) => map.get(&t).map(|n| m.to = Some(*n)).is_some(),
                None => true,
            });
            // A connector (or label) copied with what it joins joins the copies.
            if let Some(b) = c.board.as_mut() {
                b.remap(map);
            }
            // A pattern tiling a layer of the copy tiles the copy's layer.
            for p in c.fills.iter_mut().chain(c.strokes.iter_mut()) {
                if let figcraft_color::PaintKind::Pattern(pt) = &mut p.kind
                    && let Some(m) = map.get(&NodeId(pt.source))
                {
                    pt.source = m.0;
                }
            }
            if let NodeKind::Vector { network } = &mut c.kind {
                for p in network.regions.iter_mut().filter_map(|r| r.fills.as_mut()).flatten() {
                    if let figcraft_color::PaintKind::Pattern(pt) = &mut p.kind
                        && let Some(m) = map.get(&NodeId(pt.source))
                    {
                        pt.source = m.0;
                    }
                }
            }
            self.reserve_ids(*new_id);
            self.nodes.insert(*new_id, Arc::new(c));
            self.dirty.touch(*new_id);
        }
        let p = self.get_mut(parent)?;
        let at = index.unwrap_or(p.children.len()).min(p.children.len());
        p.children.insert(at, new_root);
        Ok(new_root)
    }

    /// Find nodes on `page` (or all pages) whose name contains `query` (case-insensitive).
    pub fn find_by_name(&self, query: &str, page: Option<NodeId>) -> Vec<NodeId> {
        let q = query.to_lowercase();
        let roots: Vec<NodeId> = page.map(|p| vec![p]).unwrap_or_else(|| self.pages.clone());
        roots
            .into_iter()
            .flat_map(|r| self.subtree(r))
            .filter(|i| self.node(*i).is_some_and(|n| !n.is_page() && n.name.to_lowercase().contains(&q)))
            .collect()
    }

    /// The nearest frame-like ancestor (excluding `id` itself).
    pub fn containing_frame(&self, id: NodeId) -> Option<NodeId> {
        self.ancestors(id).into_iter().find(|a| self.node(*a).is_some_and(|n| n.kind.is_frame_like() || matches!(n.kind, NodeKind::Section { .. })))
    }

    /// The top-level frame containing `id` (or `id` if it is one): the outermost frame, on the
    /// page or in a section (sections nest and hold top-level frames).
    pub fn top_level_frame(&self, id: NodeId) -> Option<NodeId> {
        let mut chain = vec![id];
        chain.extend(self.ancestors(id));
        // chain ends with the page; below it, past any sections, is the top-level node.
        let page_pos = chain.iter().position(|c| self.node(*c).is_some_and(Node::is_page))?;
        let top = chain.iter().take(page_pos).rev().copied().find(|c| !self.node(*c).is_some_and(|n| matches!(n.kind, NodeKind::Section { .. })))?;
        self.node(top).filter(|n| n.kind.is_frame_like()).map(|_| top)
    }

    /// Top-level frames of a page (also those in sections) in reading order (rows top to bottom,
    /// then left to right).
    pub fn top_frames(&self, page: NodeId) -> Vec<NodeId> {
        self.frames_in(page)
    }

    /// The top-level frames inside page or section `container` (and its sections, at any depth), in
    /// reading order.
    pub fn frames_in(&self, container: NodeId) -> Vec<NodeId> {
        fn walk(doc: &Document, id: NodeId, depth: usize, out: &mut Vec<(NodeId, Rect)>) {
            let Some(n) = doc.node(id) else { return };
            for c in &n.children {
                let Some(cn) = doc.node(*c) else { continue };
                if cn.kind.is_frame_like() {
                    if let Some(b) = doc.world_bounds(*c) {
                        out.push((*c, b));
                    }
                } else if matches!(cn.kind, NodeKind::Section { .. }) && depth < 64 {
                    walk(doc, *c, depth + 1, out);
                }
            }
        }
        let mut v = Vec::new();
        walk(self, container, 0, &mut v);
        v.sort_by(|a, b| a.1.y0.total_cmp(&b.1.y0).then(a.1.x0.total_cmp(&b.1.x0)));
        v.into_iter().map(|(i, _)| i).collect()
    }

    /// The sections on page `page` (nested ones after their section), in layer order.
    pub fn sections_in(&self, page: NodeId) -> Vec<NodeId> {
        fn walk(doc: &Document, id: NodeId, depth: usize, out: &mut Vec<NodeId>) {
            let Some(n) = doc.node(id) else { return };
            for c in &n.children {
                if doc.node(*c).is_some_and(|cn| matches!(cn.kind, NodeKind::Section { .. })) && depth < 64 {
                    out.push(*c);
                    walk(doc, *c, depth + 1, out);
                }
            }
        }
        let mut v = Vec::new();
        walk(self, page, 0, &mut v);
        v
    }

    /// A unique name like "Frame 3" for a new node of a kind.
    pub fn next_name(&self, base: &str) -> String {
        let mut max = 0u64;
        let mut taken = std::collections::BTreeSet::new();
        for n in self.nodes() {
            if let Some(rest) = n.name.strip_prefix(base)
                && let Some(num) = rest.strip_prefix(' ')
                && let Ok(k) = num.parse::<u64>()
            {
                max = max.max(k);
                taken.insert(k);
            }
        }
        let next = match max.checked_add(1) {
            Some(next) => next,
            // The counter is spent: the lowest free number instead of wrapping onto a name.
            None => (1u64..).find(|k| !taken.contains(k)).unwrap_or(1),
        };
        format!("{base} {next}")
    }

    pub fn style(&self, id: &str) -> Option<&Style> {
        self.styles.iter().find(|s| s.id == id || s.name == id)
    }

    /// The effective variable modes for a node: the ones it sets over [`Self::inherited_modes`].
    pub fn modes_for(&self, id: NodeId) -> BTreeMap<CollectionId, ModeId> {
        let mut out = self.inherited_modes(id);
        if let Some(n) = self.node(id) {
            out.extend(n.var_modes.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        out
    }

    /// The modes a node is on where it sets none (Auto): its ancestors', and for an instance the
    /// ones its main component sets, which are closer than any container's.
    pub fn inherited_modes(&self, id: NodeId) -> BTreeMap<CollectionId, ModeId> {
        let mut out = BTreeMap::new();
        for c in self.ancestors(id).into_iter().rev() {
            if let Some(n) = self.node(c) {
                out.extend(self.main_modes(n).chain(&n.var_modes).map(|(k, v)| (k.clone(), v.clone())));
            }
        }
        if let Some(n) = self.node(id) {
            out.extend(self.main_modes(n).map(|(k, v)| (k.clone(), v.clone())));
        }
        out
    }

    /// The modes an instance's main component sets (none for other layers).
    fn main_modes<'a>(&'a self, n: &Node) -> impl Iterator<Item = (&'a CollectionId, &'a ModeId)> {
        let main = match n.kind {
            NodeKind::Instance { component, .. } => self.node(component),
            _ => None,
        };
        main.into_iter().flat_map(|m| &m.var_modes)
    }

    /// Check structural invariants (used by tests and after loading untrusted files).
    pub fn validate(&self) -> Vec<String> {
        // Hostile files can break an invariant per child; keep only the first few reports.
        const MAX_ISSUES: usize = 100;
        fn note(issues: &mut Vec<String>, msg: impl FnOnce() -> String) {
            if issues.len() < MAX_ISSUES {
                issues.push(msg());
            }
        }
        let mut issues = Vec::new();
        // Every (parent, child) edge and the page list, indexed once so each node's checks are
        // lookups: a wide parent or a long page list doesn't make validation quadratic.
        let edges: std::collections::BTreeSet<(NodeId, NodeId)> = self.nodes.iter().flat_map(|(p, n)| n.children.iter().map(|c| (*p, *c))).collect();
        let page_ids: std::collections::BTreeSet<NodeId> = self.pages.iter().copied().collect();
        for (id, n) in self.nodes.iter() {
            if n.id != *id {
                note(&mut issues, || format!("node key {id} has id {}", n.id));
            }
            let mut seen = std::collections::BTreeSet::new();
            for c in &n.children {
                if !seen.insert(*c) {
                    note(&mut issues, || format!("child {c} listed twice in {id}"));
                }
                match self.node(*c) {
                    Some(cn) if cn.parent == Some(*id) => {}
                    Some(_) => note(&mut issues, || format!("child {c} of {id} has another parent")),
                    None => note(&mut issues, || format!("child {c} of {id} missing")),
                }
            }
            if let Some(p) = n.parent
                && !(self.node(p).is_some() && edges.contains(&(p, *id)))
            {
                note(&mut issues, || format!("{id} not listed in parent {p}"));
            }
            // The library store's root is a page that is never listed.
            let listed = page_ids.contains(id) || (n.is_page() && self.libraries.store == Some(*id));
            if n.is_page() != listed {
                note(&mut issues, || format!("page list mismatch for {id}"));
            }
            if n.is_page() && n.parent.is_some() {
                note(&mut issues, || format!("page {id} has a parent"));
            }
        }
        let reachable = self.reachable();
        for id in self.nodes.keys().filter(|k| !reachable.contains(*k)) {
            note(&mut issues, || format!("{id} is not under a page (cycle or detached)"));
        }
        issues
    }

    /// Every node reachable from a page through reciprocal parent/child links, each once (cycles
    /// and repeated children are walked once, so this is linear in the node count).
    fn reachable(&self) -> std::collections::BTreeSet<NodeId> {
        let mut seen = std::collections::BTreeSet::new();
        let mut stack: Vec<NodeId> = self.nodes.iter().filter(|(_, n)| n.is_page() && n.parent.is_none()).map(|(k, _)| *k).collect();
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(n) = self.nodes.get(&id) else { continue };
            stack.extend(n.children.iter().filter(|c| !seen.contains(*c) && self.nodes.get(*c).is_some_and(|cn| cn.parent == Some(id))));
        }
        seen
    }

    /// Repair what [`validate`](Self::validate) reports (lenient loading of untrusted files).
    pub fn repair(&mut self) {
        let ids: Vec<NodeId> = self.nodes.keys().copied().collect();
        for id in &ids {
            if let Some(n) = self.nodes.get_mut(id) {
                let n = Arc::make_mut(n);
                n.id = *id;
                // Pages are roots.
                if n.is_page() {
                    n.parent = None;
                }
            }
        }
        // Drop dangling children, children claimed by another parent and repeats.
        let parents: BTreeMap<NodeId, Option<NodeId>> = self.nodes.iter().map(|(k, n)| (*k, n.parent)).collect();
        for id in &ids {
            let Some(n) = self.nodes.get(id) else { continue };
            let mut seen = std::collections::BTreeSet::new();
            let keep: Vec<NodeId> =
                n.children.iter().copied().filter(|c| parents.get(c).copied().flatten() == Some(*id) && seen.insert(*c)).collect();
            if keep != n.children
                && let Some(n) = self.nodes.get_mut(id)
            {
                Arc::make_mut(n).children = keep;
            }
        }
        // Nodes not under a page (orphans, detached subtrees, cycles) are removed.
        let reachable = self.reachable();
        let gone: Vec<NodeId> = self.nodes.keys().filter(|k| !reachable.contains(*k)).copied().collect();
        for k in gone {
            self.nodes.remove(&k);
        }
        // Snapshots of layers that are gone or no longer changed since marked ready for dev, or that
        // don't start with their layer, are dropped.
        self.dev_snapshots.retain(|k, snap| {
            self.nodes.get(k).is_some_and(|n| n.dev_status == DevStatus::ReadyForDev && n.dev_changed)
                && snap.nodes.first().is_some_and(|n| n.id == *k)
        });
        let store = self.libraries.store.filter(|s| self.nodes.get(s).is_some_and(|n| n.is_page()));
        if store != self.libraries.store {
            Arc::make_mut(&mut self.libraries).store = store;
        }
        self.pages.retain(|p| self.nodes.get(p).is_some_and(|n| n.is_page()) && Some(*p) != store);
        // Unlisted pages join the list; a set keeps many pages from making this quadratic (#1078).
        let mut listed: std::collections::BTreeSet<NodeId> = self.pages.iter().copied().collect();
        for (k, n) in self.nodes.iter() {
            if n.is_page() && Some(*k) != store && listed.insert(*k) {
                self.pages.push(*k);
            }
        }
        if self.pages.is_empty() {
            let p = self.add_page("Page 1");
            self.current_page = Some(p);
        }
        let client = self.client_id();
        self.set_client_id(client);
        if self.current_page.is_none_or(|p| !self.pages.contains(&p)) {
            self.current_page = self.pages.first().copied();
        }
        self.link_layer_text_styles();
    }

    /// Text layers linked to a text style whose chars carry no link (files from before per-char
    /// links, importers that link layers): the chars showing the style link to it, and the layer
    /// stays linked only if they all do.
    pub fn link_layer_text_styles(&mut self) {
        let defs: BTreeMap<String, TextStyle> = self
            .styles
            .iter()
            .filter_map(|s| match &s.kind {
                StyleKind::Text { style } => Some((s.id.clone(), style.clone())),
                _ => None,
            })
            .collect();
        let legacy = |n: &Node| match &n.kind {
            NodeKind::Text(t) => n.styles.text.is_some() && t.spans().iter().all(|(_, _, s)| s.style_id.is_none()),
            _ => false,
        };
        let ids: Vec<NodeId> = self.nodes.iter().filter(|(_, n)| legacy(n)).map(|(k, _)| *k).collect();
        for id in ids {
            let Some(n) = self.nodes.get_mut(&id) else { continue };
            let n = Arc::make_mut(n);
            let sid = n.styles.text.clone().unwrap_or_default();
            if let NodeKind::Text(t) = &mut n.kind {
                if let Some(def) = defs.get(&sid) {
                    t.each_style(|s| {
                        if s.shows_text_style(def) {
                            s.style_id = Some(sid.clone());
                        }
                    });
                }
                n.styles.text = t.text_style_link().flatten().map(str::to_string);
            }
        }
    }

    /// Hit test: the topmost visible, unlocked node under `pt` on `page` for a click.
    /// `deep` selects the innermost leaf (Cmd-click); otherwise top-level children of frames
    /// resolve like Figma's click-through rules (see [`geometry::select_target`]). Boolean groups
    /// are hit where their `result` is (see [`geometry::BoolResult`]).
    pub fn hit_test(&self, page: NodeId, pt: Point, tolerance: f64, result: geometry::BoolResult) -> Option<NodeId> {
        geometry::hit_leaf(self, page, pt, tolerance, result)
    }

    /// `n`'s children as paint runs (index ranges into `n.children`), bottom to top: a visible
    /// mask with the layers above it up to the next mask, or a single layer. An auto layout frame
    /// whose first child paints on top paints the runs in reverse (see [`Node::paint_order`]).
    pub fn paint_runs(&self, n: &Node) -> Vec<std::ops::Range<usize>> {
        let kids: Vec<Option<&Node>> = n.children.iter().map(|k| self.node(*k)).collect();
        Self::paint_runs_of(&kids, n.layout.as_ref().is_some_and(|l| l.reverse_z))
    }

    /// [`Self::paint_runs`] of children already looked up (`None`: not in the document), last run
    /// first when `reverse_z`.
    pub fn paint_runs_of(kids: &[Option<&Node>], reverse_z: bool) -> Vec<std::ops::Range<usize>> {
        let node = |i: usize| kids.get(i).copied().flatten();
        let mut runs = Vec::new();
        let mut i = 0;
        while i < kids.len() {
            let mut j = i + 1;
            if node(i).is_some_and(|k| k.is_mask && k.visible) {
                while j < kids.len() && !node(j).is_some_and(|k| k.is_mask) {
                    j += 1;
                }
            }
            runs.push(i..j);
            i = j;
        }
        if reverse_z {
            runs.reverse();
        }
        runs
    }
}

fn remap_action(a: &mut proto::Action, map: &BTreeMap<NodeId, NodeId>) {
    use proto::Action::*;
    match a {
        Navigate { dest, .. } | ScrollTo { dest, .. } | OpenOverlay { dest, .. } | SwapOverlay { dest } | ChangeTo { dest } => {
            if let Some(m) = map.get(dest) {
                *dest = *m;
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod blur_tests;
#[cfg(test)]
mod boolean_hit_tests;
#[cfg(test)]
mod client_id_tests;
#[cfg(test)]
mod colors_tests;
#[cfg(test)]
mod diff_tests;
#[cfg(test)]
mod effects_tests;
#[cfg(test)]
mod guides_tests;
#[cfg(test)]
mod handoff_tests;
#[cfg(test)]
mod ids_tests;
#[cfg(test)]
mod print_tests;
#[cfg(test)]
mod proto_expr_mode_tests;
#[cfg(test)]
mod proto_expr_parse_tests;
#[cfg(test)]
mod proto_expr_tests;
#[cfg(test)]
mod proto_overlay_anchor_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod vector_regions_tests;
#[cfg(test)]
mod width_sides_tests;
#[cfg(test)]
mod width_tests;
