//! Streaming compact `.figcraft` document deserializer and mirror/vector resolver.
//!
//! Reads legacy v1 `document.json` and compact mirror/vector-table `document.json` in a single
//! streaming pass, resolving mirror chains iteratively with cycle detection and a materialized
//! memory budget against amplification bombs.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::Arc;

use craft_figcraft_format::{Header, MAX_DOCUMENT};
use figcraft_doc::color::{BlendMode, Color, ColorProfile};
use figcraft_doc::geom::Affine;
use figcraft_doc::{
    AnnotationCategory, Ballots, BoardTimer, Comment, DesignReview, DevSnapshot, Document, FileKind, ImagePreset, Libraries, Node, NodeId, NodeKind,
    Style, Variables, VectorNetwork, Vote, VoteRecord,
};
use serde::de::{DeserializeSeed, Error as DeError, IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use super::fields::{MirrorPatch, apply_full_patch, apply_mirror_patch, deserialize_patch_field, est_node_bytes, est_vector_bytes};
use crate::FormatError;

/// Maximum number of nodes accepted in one `document.json`.
const MAX_NODES: usize = 5_000_000;
/// Minimum materialized memory budget (bytes) allowed for compact expansion.
const MIN_MATERIALIZED_BUDGET: usize = 16 * 1024 * 1024;
/// Maximum allowed materialized expansion ratio relative to input JSON byte length.
const MAX_EXPANSION_RATIO: usize = 256;

/// Materialized memory ceiling for resolving mirrors and `vectorRef` entries.
pub(crate) fn materialized_budget(input_bytes: usize) -> usize {
    input_bytes.saturating_mul(MAX_EXPANSION_RATIO).clamp(MIN_MATERIALIZED_BUDGET, MAX_DOCUMENT as usize)
}

/// Parse a `document.json` string (legacy v1 or compact mirrors/vectors), checking the native
/// header and version first.
pub(crate) fn parse_compact_document(text: &str) -> Result<(Document, Header), FormatError> {
    if !text.contains("\"mirror\"") && !text.contains("\"vectorRef\"") {
        return craft_figcraft_format::read_document_json::<Document>(text).map_err(|e| match e {
            craft_figcraft_format::ReadDocumentError::Json(error) => FormatError::Corrupt(error.to_string()),
            craft_figcraft_format::ReadDocumentError::NewerVersion(error) => FormatError::Newer(error.0),
        });
    }
    parse_compact_document_with_budget(text, materialized_budget(text.len()))
}

/// [`parse_compact_document`] with an explicit materialized memory budget.
pub(crate) fn parse_compact_document_with_budget(text: &str, budget: usize) -> Result<(Document, Header), FormatError> {
    let header = craft_figcraft_format::read_header(text).map_err(|e| FormatError::Corrupt(e.to_string()))?;
    craft_figcraft_format::check_version(&header).map_err(|e| FormatError::Newer(e.0))?;
    let mut de = serde_json::Deserializer::from_str(text);
    let wire = CompactDocSeed { budget }.deserialize(&mut de).map_err(|e| FormatError::Corrupt(e.to_string()))?;
    de.end().map_err(|e| FormatError::Corrupt(e.to_string()))?;
    let doc = materialize(wire, budget)?;
    Ok((doc, header))
}

struct CompactDocWire {
    name: String,
    kind: FileKind,
    pages: Vec<NodeId>,
    nodes: BTreeMap<NodeId, CompactNodeEntry>,
    vectors: Vec<VectorNetwork>,
    next_id: NodeId,
    styles: Vec<Style>,
    variables: Variables,
    current_page: Option<NodeId>,
    document_colors: Vec<Color>,
    comments: Vec<Comment>,
    libraries: Arc<Libraries>,
    color_profile: ColorProfile,
    image_presets: Vec<ImagePreset>,
    design_review: DesignReview,
    annotation_categories: Option<Vec<AnnotationCategory>>,
    dev_snapshots: BTreeMap<NodeId, Arc<DevSnapshot>>,
    timer: Option<BoardTimer>,
    vote: Option<Vote>,
    votes: Ballots,
    vote_history: Vec<VoteRecord>,
}

enum CompactNodeEntry {
    Full { node: Box<Node>, vector_ref: Option<u32> },
    Mirror(Box<CompactMirror>),
}

struct CompactMirror {
    id: NodeId,
    source: NodeId,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    vector_ref: Option<u32>,
    explicit_kind: Option<NodeKind>,
    patch: MirrorPatch,
}

struct CompactDocSeed {
    budget: usize,
}

impl<'de> DeserializeSeed<'de> for CompactDocSeed {
    type Value = CompactDocWire;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_map(CompactDocVisitor { budget: self.budget })
    }
}

struct CompactDocVisitor {
    budget: usize,
}

impl<'de> Visitor<'de> for CompactDocVisitor {
    type Value = CompactDocWire;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a FigCraft document object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut state = DocParseState::default();
        while let Some(key) = map.next_key::<Cow<'de, str>>()? {
            read_doc_field(&key, &mut map, &mut state, self.budget)?;
        }
        state.finish()
    }
}

#[derive(Default)]
struct DocParseState {
    name: Option<String>,
    kind: Option<FileKind>,
    pages: Option<Vec<NodeId>>,
    nodes: Option<BTreeMap<NodeId, CompactNodeEntry>>,
    vectors: Option<Vec<VectorNetwork>>,
    next_id: Option<NodeId>,
    styles: Option<Vec<Style>>,
    variables: Option<Variables>,
    current_page: Option<Option<NodeId>>,
    document_colors: Option<Vec<Color>>,
    comments: Option<Vec<Comment>>,
    libraries: Option<Arc<Libraries>>,
    color_profile: Option<ColorProfile>,
    image_presets: Option<Vec<ImagePreset>>,
    design_review: Option<DesignReview>,
    annotation_categories: Option<Option<Vec<AnnotationCategory>>>,
    dev_snapshots: Option<BTreeMap<NodeId, Arc<DevSnapshot>>>,
    timer: Option<Option<BoardTimer>>,
    vote: Option<Option<Vote>>,
    votes: Option<Ballots>,
    vote_history: Option<Vec<VoteRecord>>,
}

impl DocParseState {
    fn finish<E: DeError>(self) -> Result<CompactDocWire, E> {
        Ok(CompactDocWire {
            name: self.name.ok_or_else(|| E::missing_field("name"))?,
            kind: self.kind.unwrap_or_default(),
            pages: self.pages.ok_or_else(|| E::missing_field("pages"))?,
            nodes: self.nodes.ok_or_else(|| E::missing_field("nodes"))?,
            vectors: self.vectors.unwrap_or_default(),
            next_id: self.next_id.ok_or_else(|| E::missing_field("next_id"))?,
            styles: self.styles.unwrap_or_default(),
            variables: self.variables.unwrap_or_default(),
            current_page: self.current_page.unwrap_or(None),
            document_colors: self.document_colors.unwrap_or_default(),
            comments: self.comments.unwrap_or_default(),
            libraries: self.libraries.unwrap_or_default(),
            color_profile: self.color_profile.unwrap_or_default(),
            image_presets: self.image_presets.unwrap_or_default(),
            design_review: self.design_review.unwrap_or_default(),
            annotation_categories: self.annotation_categories.unwrap_or(None),
            dev_snapshots: self.dev_snapshots.unwrap_or_default(),
            timer: self.timer.unwrap_or(None),
            vote: self.vote.unwrap_or(None),
            votes: self.votes.unwrap_or_default(),
            vote_history: self.vote_history.unwrap_or_default(),
        })
    }
}

fn read_doc_field<'de, A: MapAccess<'de>>(key: &str, map: &mut A, s: &mut DocParseState, budget: usize) -> Result<(), A::Error> {
    match key {
        "name" => s.name = Some(map.next_value()?),
        "kind" => s.kind = Some(map.next_value()?),
        "pages" => s.pages = Some(map.next_value()?),
        "nodes" => s.nodes = Some(map.next_value_seed(NodesMapSeed { budget })?),
        "vectors" => s.vectors = Some(map.next_value()?),
        "next_id" => s.next_id = Some(map.next_value()?),
        "styles" => s.styles = Some(map.next_value()?),
        "variables" => s.variables = Some(map.next_value()?),
        "current_page" => s.current_page = Some(map.next_value()?),
        "document_colors" => s.document_colors = Some(map.next_value()?),
        "comments" => s.comments = Some(map.next_value()?),
        "libraries" => s.libraries = Some(map.next_value()?),
        "color_profile" => s.color_profile = Some(map.next_value()?),
        "image_presets" => s.image_presets = Some(map.next_value()?),
        "design_review" => s.design_review = Some(map.next_value()?),
        "annotation_categories" => s.annotation_categories = Some(map.next_value()?),
        "dev_snapshots" => s.dev_snapshots = Some(map.next_value()?),
        "timer" => s.timer = Some(map.next_value()?),
        "vote" => s.vote = Some(map.next_value()?),
        "votes" => s.votes = Some(map.next_value()?),
        "vote_history" => s.vote_history = Some(map.next_value()?),
        _ => {
            let _ = map.next_value::<IgnoredAny>()?;
        }
    }
    Ok(())
}

struct NodesMapSeed {
    budget: usize,
}

impl<'de> DeserializeSeed<'de> for NodesMapSeed {
    type Value = BTreeMap<NodeId, CompactNodeEntry>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_map(NodesMapVisitor { budget: self.budget })
    }
}

struct NodesMapVisitor {
    budget: usize,
}

impl<'de> Visitor<'de> for NodesMapVisitor {
    type Value = BTreeMap<NodeId, CompactNodeEntry>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a map of node ids to nodes")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut nodes = BTreeMap::new();
        let mut charged = 0usize;
        while let Some(id) = map.next_key::<NodeId>()? {
            if nodes.len() >= MAX_NODES {
                return Err(A::Error::custom(format!("document exceeds maximum of {MAX_NODES} nodes")));
            }
            let entry = map.next_value::<CompactNodeEntry>()?;
            let charge = match &entry {
                CompactNodeEntry::Full { node, .. } => est_node_bytes(node),
                CompactNodeEntry::Mirror(_) => std::mem::size_of::<Node>(),
            };
            charged = charged.saturating_add(charge);
            if charged > self.budget {
                return Err(A::Error::custom("document exceeds materialized memory budget"));
            }
            nodes.insert(id, entry);
        }
        Ok(nodes)
    }
}

impl<'de> Deserialize<'de> for CompactNodeEntry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(CompactNodeVisitor)
    }
}

struct CompactNodeVisitor;

impl<'de> Visitor<'de> for CompactNodeVisitor {
    type Value = CompactNodeEntry;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a layer object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut acc = NodeAccumulator::default();
        while let Some(key) = map.next_key::<Cow<'de, str>>()? {
            read_node_key(&key, &mut map, &mut acc)?;
        }
        acc.finish()
    }
}

#[derive(Default)]
struct NodeAccumulator {
    is_mirror: bool,
    id: Option<NodeId>,
    source: Option<Option<NodeId>>,
    parent: Option<Option<NodeId>>,
    children: Option<Vec<NodeId>>,
    vector_ref: Option<u32>,
    explicit_kind: Option<NodeKind>,
    kind_type: Option<String>,
    kind_network: Option<VectorNetwork>,
    kind_extra: Option<serde_json::Map<String, Value>>,
    patch: MirrorPatch,
}

fn is_kind_field(key: &str) -> bool {
    matches!(
        key,
        "background"
            | "flows"
            | "prototype"
            | "opening"
            | "contents_hidden"
            | "start"
            | "sweep"
            | "inner"
            | "count"
            | "inner_ratio"
            | "characters"
            | "style"
            | "runs"
            | "auto_resize"
            | "align_h"
            | "align_v"
            | "paragraph_spacing"
            | "paragraph_indent"
            | "list"
            | "paragraphs"
            | "list_spacing"
            | "hanging_punctuation"
            | "hanging_list"
            | "leading_trim"
            | "max_lines"
            | "wrap_style"
            | "features"
            | "path"
            | "saved_breaks"
            | "op"
            | "description"
            | "props"
            | "prop_order"
            | "component"
    )
}

fn read_node_key<'de, A: MapAccess<'de>>(key: &str, map: &mut A, acc: &mut NodeAccumulator) -> Result<(), A::Error> {
    match key {
        "mirror" => acc.is_mirror = map.next_value()?,
        "id" => acc.id = Some(map.next_value()?),
        "source" => acc.source = Some(map.next_value()?),
        "parent" => acc.parent = Some(map.next_value()?),
        "children" => acc.children = Some(map.next_value()?),
        "vectorRef" => acc.vector_ref = Some(map.next_value()?),
        "kind" => acc.explicit_kind = Some(map.next_value()?),
        "type" => acc.kind_type = Some(map.next_value()?),
        "network" => acc.kind_network = Some(map.next_value()?),
        other => {
            if deserialize_patch_field(other, map, &mut acc.patch)? {
                return Ok(());
            }
            if is_kind_field(other) {
                let val = map.next_value::<Value>()?;
                acc.kind_extra.get_or_insert_with(serde_json::Map::new).insert(other.to_owned(), val);
            } else {
                let _ = map.next_value::<IgnoredAny>()?;
            }
        }
    }
    Ok(())
}

impl NodeAccumulator {
    fn finish<E: DeError>(mut self) -> Result<CompactNodeEntry, E> {
        let id = self.id.ok_or_else(|| E::missing_field("id"))?;
        if self.is_mirror {
            if self.explicit_kind.is_some() && self.vector_ref.is_some() {
                return Err(E::custom("mirror node cannot set both `kind` and `vectorRef`"));
            }
            if self.kind_type.is_some() || self.kind_network.is_some() {
                return Err(E::custom("mirror node must use `kind` or `vectorRef`, not `type`"));
            }
            let source = self.source.flatten().ok_or_else(|| E::missing_field("source"))?;
            return Ok(CompactNodeEntry::Mirror(Box::new(CompactMirror {
                id,
                source,
                parent: self.parent.flatten(),
                children: self.children.unwrap_or_default(),
                vector_ref: self.vector_ref,
                explicit_kind: self.explicit_kind,
                patch: self.patch,
            })));
        }

        let name = self.patch.name.take().ok_or_else(|| E::missing_field("name"))?;
        let width = self.patch.width.take().ok_or_else(|| E::missing_field("width"))?;
        let height = self.patch.height.take().ok_or_else(|| E::missing_field("height"))?;
        let (kind, vector_ref) = build_full_kind(self.kind_type, self.kind_network, self.vector_ref, self.kind_extra)?;
        let mut node = Node::new(id, kind, name, 0.0, 0.0);
        // Reset kind-specific Node::new presets back to Node's serde defaults before applying fields.
        node.width = width;
        node.height = height;
        node.parent = self.parent.flatten();
        node.children = self.children.unwrap_or_default();
        node.source = self.source.flatten();
        node.fills.clear();
        node.strokes.clear();
        node.stroke = Default::default();
        node.corners = Default::default();
        node.blend = BlendMode::PassThrough;
        node.clip = false;
        node.transform = Affine::IDENTITY;
        node.visible = true;
        node.opacity = 1.0;
        node.scale_factor = 1.0;
        apply_full_patch(&mut node, self.patch);
        Ok(CompactNodeEntry::Full { node: Box::new(node), vector_ref })
    }
}

fn build_full_kind<E: DeError>(
    kind_type: Option<String>,
    kind_network: Option<VectorNetwork>,
    vector_ref: Option<u32>,
    kind_extra: Option<serde_json::Map<String, Value>>,
) -> Result<(NodeKind, Option<u32>), E> {
    let ty = kind_type.ok_or_else(|| E::missing_field("type"))?;
    if vector_ref.is_some() && ty != "vector" {
        return Err(E::custom("`vectorRef` is only valid on vector layers"));
    }
    match ty.as_str() {
        "frame" => Ok((NodeKind::Frame, None)),
        "group" => Ok((NodeKind::Group, None)),
        "rectangle" => Ok((NodeKind::Rectangle, None)),
        "line" => Ok((NodeKind::Line, None)),
        "slice" => Ok((NodeKind::Slice, None)),
        "vector" => match (kind_network, vector_ref) {
            (Some(_), Some(_)) => Err(E::custom("vector layer cannot set both `network` and `vectorRef`")),
            (Some(network), None) => Ok((NodeKind::Vector { network }, None)),
            (None, Some(idx)) => Ok((NodeKind::Vector { network: VectorNetwork::default() }, Some(idx))),
            (None, None) => Err(E::missing_field("network")),
        },
        _ => {
            let mut map = kind_extra.unwrap_or_default();
            map.insert("type".to_owned(), Value::String(ty));
            let kind = serde_json::from_value::<NodeKind>(Value::Object(map)).map_err(E::custom)?;
            Ok((kind, None))
        }
    }
}

fn materialize(wire: CompactDocWire, budget: usize) -> Result<Document, FormatError> {
    let vectors = wire.vectors;
    let mut charged: usize = vectors.iter().map(est_vector_bytes).fold(0usize, usize::saturating_add);
    if charged > budget {
        return Err(FormatError::Corrupt("document vectors exceed materialized memory budget".into()));
    }

    let mut raw = wire.nodes;
    let ids: Vec<NodeId> = raw.keys().copied().collect();
    let mut resolved: HashMap<NodeId, Arc<Node>> = HashMap::with_capacity(ids.len());
    let mut resolving: Vec<NodeId> = Vec::new();
    let mut in_stack: HashMap<NodeId, bool> = HashMap::new();

    for root_id in ids {
        if resolved.contains_key(&root_id) {
            continue;
        }
        resolve_chain(root_id, &mut raw, &vectors, &mut resolved, &mut resolving, &mut in_stack, &mut charged, budget)?;
    }

    let mut doc = Document::without_pages(wire.name);
    doc.kind = wire.kind;
    doc.pages = wire.pages;
    doc.styles = wire.styles;
    doc.variables = wire.variables;
    doc.current_page = wire.current_page;
    doc.document_colors = wire.document_colors;
    doc.comments = wire.comments;
    doc.libraries = wire.libraries;
    doc.color_profile = wire.color_profile;
    doc.image_presets = wire.image_presets;
    doc.design_review = wire.design_review;
    doc.annotation_categories = wire.annotation_categories;
    doc.dev_snapshots = wire.dev_snapshots;
    doc.timer = wire.timer;
    doc.vote = wire.vote;
    doc.votes = wire.votes;
    doc.vote_history = wire.vote_history;
    doc.set_raw_nodes(resolved, wire.next_id);
    Ok(doc)
}

#[allow(clippy::too_many_arguments)]
fn resolve_chain(
    start: NodeId,
    raw: &mut BTreeMap<NodeId, CompactNodeEntry>,
    vectors: &[VectorNetwork],
    resolved: &mut HashMap<NodeId, Arc<Node>>,
    stack: &mut Vec<NodeId>,
    in_stack: &mut HashMap<NodeId, bool>,
    charged: &mut usize,
    budget: usize,
) -> Result<(), FormatError> {
    stack.clear();
    let mut cur = start;
    loop {
        if resolved.contains_key(&cur) {
            break;
        }
        if in_stack.get(&cur).copied().unwrap_or(false) {
            return Err(FormatError::Corrupt(format!("mirror cycle involving node {}", cur.0)));
        }
        let next = match raw.get(&cur) {
            Some(CompactNodeEntry::Full { .. }) => {
                stack.push(cur);
                in_stack.insert(cur, true);
                break;
            }
            Some(CompactNodeEntry::Mirror(m)) => {
                if m.source == cur {
                    return Err(FormatError::Corrupt(format!("mirror node {} references itself", cur.0)));
                }
                stack.push(cur);
                in_stack.insert(cur, true);
                m.source
            }
            None => {
                return Err(FormatError::Corrupt(format!("mirror references missing source node {}", cur.0)));
            }
        };
        cur = next;
    }

    while let Some(id) = stack.pop() {
        in_stack.remove(&id);
        let Some(entry) = raw.remove(&id) else { continue };
        let node = match entry {
            CompactNodeEntry::Full { mut node, vector_ref } => {
                if let Some(idx) = vector_ref {
                    let net = lookup_vector(vectors, idx)?;
                    let base_charge = est_node_bytes(&node);
                    let net_charge = est_vector_bytes(net);
                    charge_budget(charged, base_charge.saturating_add(net_charge), budget)?;
                    node.kind = NodeKind::Vector { network: net.clone() };
                } else {
                    charge_budget(charged, est_node_bytes(&node), budget)?;
                }
                *node
            }
            CompactNodeEntry::Mirror(m) => {
                let src =
                    resolved.get(&m.source).ok_or_else(|| FormatError::Corrupt(format!("mirror references missing source node {}", m.source.0)))?;
                let vref_net = match m.vector_ref {
                    Some(idx) => Some(lookup_vector(vectors, idx)?),
                    None => None,
                };
                // Charge before cloning the source node or vector network to prevent OOM spikes.
                let pre_charge = est_node_bytes(src).saturating_add(vref_net.map_or(0, est_vector_bytes));
                charge_budget(charged, pre_charge, budget)?;
                let mut node = (**src).clone();
                node.id = m.id;
                node.source = Some(m.source);
                node.parent = m.parent;
                node.children = m.children;
                if let Some(kind) = m.explicit_kind {
                    node.kind = kind;
                } else if let Some(net) = vref_net {
                    node.kind = NodeKind::Vector { network: net.clone() };
                }
                apply_mirror_patch(&mut node, m.patch);
                let extra = est_node_bytes(&node).saturating_sub(pre_charge);
                charge_budget(charged, extra, budget)?;
                node
            }
        };
        resolved.insert(id, Arc::new(node));
    }
    Ok(())
}

fn lookup_vector(vectors: &[VectorNetwork], idx: u32) -> Result<&VectorNetwork, FormatError> {
    usize::try_from(idx).ok().and_then(|i| vectors.get(i)).ok_or_else(|| FormatError::Corrupt(format!("invalid vectorRef {idx}")))
}

fn charge_budget(charged: &mut usize, amount: usize, budget: usize) -> Result<(), FormatError> {
    *charged = charged.saturating_add(amount);
    if *charged > budget {
        return Err(FormatError::Corrupt("document exceeds materialized memory budget".into()));
    }
    Ok(())
}
