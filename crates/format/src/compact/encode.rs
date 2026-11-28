//! Compact `.figcraft` document serializer.
//!
//! Elides unchanged fields on cycle-free mirrored layers (`Node::source`) and deduplicates
//! repeated non-empty `VectorNetwork`s into a top-level `"vectors"` table.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use figcraft_doc::color::BlendMode;
use figcraft_doc::geom::Affine;
use figcraft_doc::{Document, MaskType, Node, NodeId, NodeKind, VectorNetwork};
use serde::ser::{SerializeMap, SerializeStruct};
use serde::{Serialize, Serializer};

use super::decode::materialized_budget;
use super::fields::{canonical_vector_key, est_node_bytes, est_vector_bytes, exact_kind_eq, serialize_mirror_diff_fields};

/// Conservative lower bound on encoded bytes for a compact mirror record.
const MIN_MIRROR_JSON_BYTES: usize = 36;
/// Conservative lower bound on encoded bytes for a full node record.
const MIN_FULL_JSON_BYTES: usize = 128;

/// Serializable view of a [`Document`] that writes mirrored layers as deltas against their
/// `source` and hoists repeated `VectorNetwork`s into `"vectors"`.
pub(crate) struct CompactDocument<'a> {
    doc: &'a Document,
    mirrors: BTreeSet<NodeId>,
    vectors: Vec<&'a VectorNetwork>,
    vector_refs: BTreeMap<NodeId, u32>,
}

impl<'a> CompactDocument<'a> {
    pub(crate) fn new(doc: &'a Document) -> Self {
        let mut mirrors = find_acyclic_mirrors(doc);
        let (mut vectors, mut vector_refs) = build_vector_table(doc, &mirrors);
        enforce_reload_budget(doc, &mut mirrors, &mut vectors, &mut vector_refs);
        Self { doc, mirrors, vectors, vector_refs }
    }
}

/// Identifies every node whose `source` pointer chain reaches a live non-cyclic root.
///
/// Uses iterative 3-color traversal so arbitrarily deep chains never overflow the stack.
fn find_acyclic_mirrors(doc: &Document) -> BTreeSet<NodeId> {
    // 0 = visiting, 1 = acyclic, 2 = cyclic/invalid
    let mut state: HashMap<NodeId, u8> = HashMap::with_capacity(doc.node_count());
    let mut mirrors = BTreeSet::new();
    let mut chain: Vec<NodeId> = Vec::new();
    for node in doc.nodes() {
        let Some(src) = node.source else { continue };
        if src == node.id || doc.node(src).is_none() {
            state.insert(node.id, 2);
            continue;
        }
        chain.clear();
        let mut cur = node.id;
        let outcome = loop {
            if let Some(&st) = state.get(&cur) {
                break if st == 1 { 1 } else { 2 };
            }
            state.insert(cur, 0);
            chain.push(cur);
            let Some(n) = doc.node(cur) else {
                break 2;
            };
            let Some(next) = n.source else {
                break 1;
            };
            if next == cur || doc.node(next).is_none() {
                break 2;
            }
            cur = next;
        };
        for id in &chain {
            state.insert(*id, outcome);
        }
        if outcome == 1 {
            mirrors.insert(node.id);
        }
    }
    mirrors
}

/// Collects finite non-empty `VectorNetwork`s that would actually be serialized at least twice,
/// assigning deterministic indices in `NodeId` order.
fn build_vector_table<'a>(doc: &'a Document, mirrors: &BTreeSet<NodeId>) -> (Vec<&'a VectorNetwork>, BTreeMap<NodeId, u32>) {
    let mut keys: Vec<(NodeId, Vec<u8>, &'a VectorNetwork)> = Vec::new();
    let mut counts: HashMap<Vec<u8>, usize> = HashMap::new();
    for node in doc.nodes() {
        let NodeKind::Vector { network } = &node.kind else { continue };
        if mirrors.contains(&node.id)
            && let Some(src) = node.source.and_then(|id| doc.node(id))
            && exact_kind_eq(&node.kind, &src.kind)
        {
            // Kind is inherited from source; no vector data is emitted for this mirror.
            continue;
        }
        let Some(key) = canonical_vector_key(network) else { continue };
        *counts.entry(key.clone()).or_insert(0) += 1;
        keys.push((node.id, key, network));
    }

    let mut vectors: Vec<&'a VectorNetwork> = Vec::new();
    let mut index_by_key: HashMap<Vec<u8>, u32> = HashMap::new();
    let mut vector_refs: BTreeMap<NodeId, u32> = BTreeMap::new();
    for (id, key, net) in keys {
        if counts.get(&key).copied().unwrap_or(0) < 2 {
            continue;
        }
        let idx = if let Some(&existing) = index_by_key.get(&key) {
            existing
        } else {
            let Ok(next) = u32::try_from(vectors.len()) else { continue };
            vectors.push(net);
            index_by_key.insert(key, next);
            next
        };
        vector_refs.insert(id, idx);
    }
    (vectors, vector_refs)
}

/// Ensures an honest document never compacts below the decoder's amplification budget.
///
/// If a pathological fan-out would exceed `materialized_budget(est_json_bytes)`, drops compact
/// encoding so the saved document always reloads cleanly.
fn enforce_reload_budget(doc: &Document, mirrors: &mut BTreeSet<NodeId>, vectors: &mut Vec<&VectorNetwork>, vector_refs: &mut BTreeMap<NodeId, u32>) {
    if mirrors.is_empty() && vectors.is_empty() {
        return;
    }
    let nodes_charge: usize = doc.nodes().map(est_node_bytes).fold(0usize, usize::saturating_add);
    let vecs_charge: usize = vectors.iter().map(|v| est_vector_bytes(v)).fold(0usize, usize::saturating_add);
    let total_materialized = nodes_charge.saturating_add(vecs_charge);
    let est_compact = doc
        .node_count()
        .saturating_sub(mirrors.len())
        .saturating_mul(MIN_FULL_JSON_BYTES)
        .saturating_add(mirrors.len().saturating_mul(MIN_MIRROR_JSON_BYTES));
    if total_materialized > materialized_budget(est_compact) {
        mirrors.clear();
        vectors.clear();
        vector_refs.clear();
    }
}

impl Serialize for CompactDocument<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let d = self.doc;
        let mut st = serializer.serialize_struct("Document", 20)?;
        st.serialize_field("name", &d.name)?;
        if !d.kind.is_design() {
            st.serialize_field("kind", &d.kind)?;
        }
        st.serialize_field("pages", &d.pages)?;
        if !self.vectors.is_empty() {
            st.serialize_field("vectors", &self.vectors)?;
        }
        st.serialize_field("nodes", &CompactNodesMap { doc: d, mirrors: &self.mirrors, vector_refs: &self.vector_refs })?;
        st.serialize_field("next_id", &d.next_id())?;
        st.serialize_field("styles", &d.styles)?;
        st.serialize_field("variables", &d.variables)?;
        st.serialize_field("current_page", &d.current_page)?;
        st.serialize_field("document_colors", &d.document_colors)?;
        st.serialize_field("comments", &d.comments)?;
        if !figcraft_doc::Libraries::is_unused(&d.libraries) {
            st.serialize_field("libraries", &d.libraries)?;
        }
        if !d.color_profile.is_srgb() {
            st.serialize_field("color_profile", &d.color_profile)?;
        }
        if !d.image_presets.is_empty() {
            st.serialize_field("image_presets", &d.image_presets)?;
        }
        if !d.design_review.is_default() {
            st.serialize_field("design_review", &d.design_review)?;
        }
        if d.annotation_categories.is_some() {
            st.serialize_field("annotation_categories", &d.annotation_categories)?;
        }
        if !d.dev_snapshots.is_empty() {
            st.serialize_field("dev_snapshots", &d.dev_snapshots)?;
        }
        if d.timer.is_some() {
            st.serialize_field("timer", &d.timer)?;
        }
        if d.vote.is_some() {
            st.serialize_field("vote", &d.vote)?;
        }
        if !d.votes.is_empty() {
            st.serialize_field("votes", &d.votes)?;
        }
        if !d.vote_history.is_empty() {
            st.serialize_field("vote_history", &d.vote_history)?;
        }
        st.end()
    }
}

struct CompactNodesMap<'a> {
    doc: &'a Document,
    mirrors: &'a BTreeSet<NodeId>,
    vector_refs: &'a BTreeMap<NodeId, u32>,
}

impl Serialize for CompactNodesMap<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.doc.node_count()))?;
        for node in self.doc.nodes() {
            if self.mirrors.contains(&node.id)
                && let Some(src_id) = node.source
                && let Some(src) = self.doc.node(src_id)
            {
                let vref = self.vector_refs.get(&node.id).copied();
                map.serialize_entry(&node.id, &CompactMirrorNode { node, src, vector_ref: vref })?;
            } else if let Some(&vref) = self.vector_refs.get(&node.id) {
                map.serialize_entry(&node.id, &FullVectorRefNode { node, vector_ref: vref })?;
            } else {
                map.serialize_entry(&node.id, node)?;
            }
        }
        map.end()
    }
}

struct CompactMirrorNode<'a> {
    node: &'a Node,
    src: &'a Node,
    vector_ref: Option<u32>,
}

impl Serialize for CompactMirrorNode<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("mirror", &true)?;
        map.serialize_entry("id", &self.node.id)?;
        if let Some(src_id) = self.node.source {
            map.serialize_entry("source", &src_id)?;
        }
        if let Some(parent) = self.node.parent {
            map.serialize_entry("parent", &parent)?;
        }
        if !self.node.children.is_empty() {
            map.serialize_entry("children", &self.node.children)?;
        }
        if !exact_kind_eq(&self.node.kind, &self.src.kind) {
            if let Some(idx) = self.vector_ref {
                map.serialize_entry("vectorRef", &idx)?;
            } else {
                map.serialize_entry("kind", &self.node.kind)?;
            }
        }
        serialize_mirror_diff_fields(&mut map, self.src, self.node)?;
        map.end()
    }
}

/// Full non-mirror `NodeKind::Vector` serialized with `"type": "vector"` and `"vectorRef": idx`
/// instead of an inline `"network"`.
struct FullVectorRefNode<'a> {
    node: &'a Node,
    vector_ref: u32,
}

impl Serialize for FullVectorRefNode<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let n = self.node;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("id", &n.id)?;
        map.serialize_entry("name", &n.name)?;
        map.serialize_entry("type", "vector")?;
        map.serialize_entry("vectorRef", &self.vector_ref)?;
        if let Some(parent) = n.parent {
            map.serialize_entry("parent", &parent)?;
        }
        if !n.children.is_empty() {
            map.serialize_entry("children", &n.children)?;
        }
        if !n.visible {
            map.serialize_entry("visible", &n.visible)?;
        }
        if n.locked {
            map.serialize_entry("locked", &n.locked)?;
        }
        if n.opacity != 1.0 {
            map.serialize_entry("opacity", &n.opacity)?;
        }
        if n.blend != BlendMode::PassThrough {
            map.serialize_entry("blend", &n.blend)?;
        }
        if n.transform != Affine::IDENTITY {
            map.serialize_entry("transform", &n.transform)?;
        }
        map.serialize_entry("width", &n.width)?;
        map.serialize_entry("height", &n.height)?;
        serialize_optional_full_fields(&mut map, n)?;
        map.end()
    }
}

fn emit_if<S: SerializeMap, T: Serialize + ?Sized>(map: &mut S, key: &str, cond: bool, val: &T) -> Result<(), S::Error> {
    if cond {
        map.serialize_entry(key, val)?;
    }
    Ok(())
}

fn serialize_optional_full_fields<S: SerializeMap>(map: &mut S, n: &Node) -> Result<(), S::Error> {
    serialize_paint_and_layout_fields(map, n)?;
    serialize_component_and_meta_fields(map, n)
}

fn serialize_paint_and_layout_fields<S: SerializeMap>(map: &mut S, n: &Node) -> Result<(), S::Error> {
    emit_if(map, "fills", !n.fills.is_empty(), &n.fills)?;
    emit_if(map, "strokes", !n.strokes.is_empty(), &n.strokes)?;
    emit_if(map, "stroke", n.stroke != Default::default(), &n.stroke)?;
    emit_if(map, "effects", !n.effects.is_empty(), &n.effects)?;
    emit_if(map, "corners", n.corners != Default::default(), &n.corners)?;
    emit_if(map, "constraints", n.constraints != Default::default(), &n.constraints)?;
    emit_if(map, "layout", n.layout.is_some(), &n.layout)?;
    emit_if(map, "child_layout", n.child_layout != Default::default(), &n.child_layout)?;
    emit_if(map, "clip", n.clip, &n.clip)?;
    emit_if(map, "is_mask", n.is_mask, &n.is_mask)?;
    emit_if(map, "mask_type", !MaskType::is_alpha(&n.mask_type), &n.mask_type)?;
    emit_if(map, "grids", !n.grids.is_empty(), &n.grids)?;
    emit_if(map, "guides", !n.guides.is_empty(), &n.guides)?;
    emit_if(map, "exports", !n.exports.is_empty(), &n.exports)?;
    emit_if(map, "interactions", !n.interactions.is_empty(), &n.interactions)?;
    emit_if(map, "overflow", n.overflow != Default::default(), &n.overflow)?;
    emit_if(map, "scroll_position", n.scroll_position != Default::default(), &n.scroll_position)?;
    emit_if(map, "styles", n.styles != Default::default(), &n.styles)
}

fn serialize_component_and_meta_fields<S: SerializeMap>(map: &mut S, n: &Node) -> Result<(), S::Error> {
    emit_if(map, "bound", !n.bound.is_empty(), &n.bound)?;
    emit_if(map, "var_modes", !n.var_modes.is_empty(), &n.var_modes)?;
    emit_if(map, "scale_factor", n.scale_factor != 1.0, &n.scale_factor)?;
    emit_if(map, "bool_op", n.bool_op.is_some(), &n.bool_op)?;
    emit_if(map, "constrain_proportions", n.constrain_proportions, &n.constrain_proportions)?;
    emit_if(map, "source", n.source.is_some(), &n.source)?;
    emit_if(map, "exposed", n.exposed, &n.exposed)?;
    emit_if(map, "dev_status", !n.dev_status.is_none(), &n.dev_status)?;
    emit_if(map, "dev_changed", n.dev_changed, &n.dev_changed)?;
    emit_if(map, "dev_note", !n.dev_note.is_empty(), &n.dev_note)?;
    emit_if(map, "overrides", !n.overrides.is_empty(), &n.overrides)?;
    emit_if(map, "prop_refs", !n.prop_refs.is_empty(), &n.prop_refs)?;
    emit_if(map, "plugin_data", !n.plugin_data.is_empty(), &n.plugin_data)?;
    emit_if(map, "annotations", !n.annotations.is_empty(), &n.annotations)?;
    emit_if(map, "code_links", !n.code_links.is_empty(), &n.code_links)?;
    emit_if(map, "dev_resources", !n.dev_resources.is_empty(), &n.dev_resources)?;
    emit_if(map, "measurements", !n.measurements.is_empty(), &n.measurements)?;
    emit_if(map, "board", n.board.is_some(), &n.board)
}
