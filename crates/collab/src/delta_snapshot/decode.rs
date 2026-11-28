//! Streaming decode and memory-budget enforcement for `CatchUp::DeltaSnapshot`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::io::Read;
use std::mem::size_of;
use std::sync::Arc;

use figcraft_doc::color::Paint;
use figcraft_doc::proto::Interaction;
use figcraft_doc::{
    Annotation, CodeLink, DevResource, Document, Effect, ExportSetting, Guide, LayoutGrid, Measurement, Node, NodeId, NodeKind, Region, Segment,
    VectorNetwork, Vertex,
};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};

use super::fields::{FieldDelta, apply_field_delta};
use super::{DEFAULT_MAX_MATERIALIZED_BYTES, HEADER_LEN, WireMeta, WireRecord, parse_header};
use crate::Error;
use crate::docprops::{clamp_design_review_config, validate_image_base64};
use crate::frac;
use crate::replica::{Positions, Replica};

/// Maximum ratio of estimated materialized bytes to declared uncompressed JSON bytes.
/// Heavy design-system files sit around 15x to 35x; amplification bombs exceed 1,000x.
const MAX_AMPLIFICATION_FACTOR: usize = 64;
const MIN_MATERIALIZED_FLOOR: usize = 16 << 20;

/// Decode a compressed `CatchUp::DeltaSnapshot` payload into a validated [`Replica`].
pub fn decode_delta_snapshot(bytes: &[u8], max_bytes: usize, max_nodes: usize) -> Result<Replica, Error> {
    decode_delta_snapshot_with_budget(bytes, max_bytes, max_nodes, DEFAULT_MAX_MATERIALIZED_BYTES)
}

/// Decode a compressed `CatchUp::DeltaSnapshot` with an explicit ceiling on estimated
/// materialized memory, also capped at `max(16 MiB, 64 * declared_bytes)`.
pub fn decode_delta_snapshot_with_budget(bytes: &[u8], max_bytes: usize, max_nodes: usize, max_materialized_bytes: usize) -> Result<Replica, Error> {
    let declared = parse_header(bytes)?;
    if declared > max_bytes {
        return Err(Error::TooLarge(max_bytes));
    }
    let body = bytes.get(HEADER_LEN..).ok_or_else(|| Error::Decode("delta snapshot body missing".into()))?;
    // Stop inflating at `declared + 1` so a forged small length aborts immediately.
    let limit = u64::try_from(declared).unwrap_or(u64::MAX).saturating_add(1);
    let prealloc = declared.min(bytes.len().saturating_mul(8)).min(1 << 20);
    let mut raw = Vec::with_capacity(prealloc);
    flate2::read::DeflateDecoder::new(body).take(limit).read_to_end(&mut raw).map_err(|e| Error::Decode(format!("delta snapshot deflate: {e}")))?;
    if raw.len() > declared {
        return Err(Error::TooLarge(declared));
    }
    if raw.len() != declared {
        return Err(Error::Decode(format!("delta snapshot truncated: header declared {declared} bytes, inflated {}", raw.len())));
    }

    let cap = declared.saturating_mul(MAX_AMPLIFICATION_FACTOR).max(MIN_MATERIALIZED_FLOOR);
    stream_decode_wire(&raw, max_nodes, max_materialized_bytes.min(cap))
}

fn est_str_map(m: &BTreeMap<String, String>) -> usize {
    m.iter().fold(0usize, |a, (k, v)| a.saturating_add(k.len()).saturating_add(v.len()).saturating_add(48))
}

fn est_vector_bytes(net: &VectorNetwork) -> usize {
    let regs = net.regions.iter().fold(0usize, |acc, r| {
        let loops = r.loops.iter().fold(0usize, |a, l| a.saturating_add(l.len().saturating_mul(size_of::<usize>())));
        let fills = r.fills.as_ref().map_or(0, |f| f.len().saturating_mul(size_of::<Paint>()));
        acc.saturating_add(size_of::<Region>()).saturating_add(loops).saturating_add(fills)
    });
    size_of::<VectorNetwork>()
        .saturating_add(net.vertices.len().saturating_mul(size_of::<Vertex>()))
        .saturating_add(net.segments.len().saturating_mul(size_of::<Segment>()))
        .saturating_add(regs)
}

fn est_kind_bytes(kind: &NodeKind) -> usize {
    match kind {
        NodeKind::Vector { network } => est_vector_bytes(network),
        NodeKind::Text(t) => t.characters.len().saturating_add(t.runs.len().saturating_mul(64)),
        NodeKind::Component { description, props, prop_order } => {
            description.len().saturating_add(props.len().saturating_mul(96)).saturating_add(prop_order.len().saturating_mul(32))
        }
        NodeKind::Instance { props, .. } => props.len().saturating_mul(96),
        _ => 0,
    }
}

fn est_node_bytes(n: &Node) -> usize {
    let ov: usize = n.overrides.iter().fold(0, |a, s| a.saturating_add(s.len()).saturating_add(24));
    size_of::<Node>()
        .saturating_add(n.name.len())
        .saturating_add(n.dev_note.len())
        .saturating_add(n.fills.len().saturating_mul(size_of::<Paint>()))
        .saturating_add(n.strokes.len().saturating_mul(size_of::<Paint>()))
        .saturating_add(n.effects.len().saturating_mul(size_of::<Effect>()))
        .saturating_add(n.grids.len().saturating_mul(size_of::<LayoutGrid>()))
        .saturating_add(n.guides.len().saturating_mul(size_of::<Guide>()))
        .saturating_add(n.exports.len().saturating_mul(size_of::<ExportSetting>()))
        .saturating_add(n.interactions.len().saturating_mul(size_of::<Interaction>()))
        .saturating_add(n.annotations.len().saturating_mul(size_of::<Annotation>()))
        .saturating_add(n.code_links.len().saturating_mul(size_of::<CodeLink>()))
        .saturating_add(n.dev_resources.len().saturating_mul(size_of::<DevResource>()))
        .saturating_add(n.measurements.len().saturating_mul(size_of::<Measurement>()))
        .saturating_add(est_kind_bytes(&n.kind))
        .saturating_add(ov)
        .saturating_add(est_str_map(&n.plugin_data))
        .saturating_add(est_str_map(&n.prop_refs))
}

fn est_deltas_extra_bytes(deltas: &[FieldDelta], vector_sizes: &[usize]) -> usize {
    deltas.iter().fold(0usize, |acc, d| {
        let extra = match d {
            FieldDelta::Name(s) | FieldDelta::DevNote(s) => s.len(),
            FieldDelta::Kind(k) => est_kind_bytes(k),
            FieldDelta::VectorRef(idx) => usize::try_from(*idx).ok().and_then(|i| vector_sizes.get(i).copied()).unwrap_or(0),
            FieldDelta::Fills(v) | FieldDelta::Strokes(v) => v.len().saturating_mul(size_of::<Paint>()),
            FieldDelta::Effects(v) => v.len().saturating_mul(size_of::<Effect>()),
            FieldDelta::PluginData(m) | FieldDelta::PropRefs(m) => est_str_map(m),
            FieldDelta::Annotations(v) => v.len().saturating_mul(size_of::<Annotation>()),
            _ => 0,
        };
        acc.saturating_add(extra)
    })
}

struct DecodeState {
    doc: Document,
    positions: Positions,
    touched: BTreeSet<NodeId>,
    vectors: Vec<figcraft_doc::Shared<VectorNetwork>>,
    vector_sizes: Vec<usize>,
    node_sizes: HashMap<NodeId, usize>,
    meta: Option<WireMeta>,
    records_seen: usize,
    max_nodes: usize,
    materialized_bytes: usize,
    max_materialized_bytes: usize,
    abort: Option<Error>,
}

impl DecodeState {
    fn new(max_nodes: usize, max_materialized_bytes: usize) -> Self {
        Self {
            doc: Document::without_pages(""),
            positions: Positions::new(),
            touched: BTreeSet::new(),
            vectors: Vec::new(),
            vector_sizes: Vec::new(),
            node_sizes: HashMap::new(),
            meta: None,
            records_seen: 0,
            max_nodes,
            materialized_bytes: 0,
            max_materialized_bytes,
            abort: None,
        }
    }

    fn charge(&mut self, bytes: usize) -> Result<(), Error> {
        self.materialized_bytes = self.materialized_bytes.saturating_add(bytes);
        if self.materialized_bytes > self.max_materialized_bytes {
            return Err(Error::TooLarge(self.max_materialized_bytes));
        }
        Ok(())
    }

    fn fail<E: de::Error>(&mut self, err: Error) -> E {
        let msg = err.to_string();
        self.abort = Some(err);
        E::custom(msg)
    }

    fn ingest_record(&mut self, rec: WireRecord) -> Result<(), Error> {
        let (node, pos, size) = match rec {
            WireRecord::Full { pos, mut node, vref } => {
                frac::validate(&pos).map_err(|e| Error::Invalid(e.to_string()))?;
                if self.doc.contains(node.id) {
                    return Err(Error::Invalid(format!("node {} already exists", node.id)));
                }
                node.children.clear();
                let mut est = est_node_bytes(&node).saturating_add(pos.len());
                if let Some(idx) = vref {
                    let i = usize::try_from(idx).map_err(|_| Error::Decode(format!("invalid vector ref {idx}")))?;
                    let vsize = self.vector_sizes.get(i).copied().ok_or_else(|| Error::Decode(format!("invalid vector ref {idx}")))?;
                    est = est.saturating_add(vsize);
                    self.charge(est)?;
                    let net = self.vectors.get(i).ok_or_else(|| Error::Decode(format!("invalid vector ref {idx}")))?;
                    node.kind = NodeKind::Vector { network: net.clone() };
                } else {
                    self.charge(est)?;
                }
                (*node, pos, est)
            }
            WireRecord::Mirror(m) => {
                frac::validate(&m.pos).map_err(|e| Error::Invalid(e.to_string()))?;
                if self.doc.contains(m.id) {
                    return Err(Error::Invalid(format!("node {} already exists", m.id)));
                }
                let missing = || Error::Invalid(format!("mirror {} references missing or cyclic source {}", m.id, m.source));
                let src_size = self.node_sizes.get(&m.source).copied().ok_or_else(missing)?;
                let est = src_size.saturating_add(est_deltas_extra_bytes(&m.deltas, &self.vector_sizes)).saturating_add(m.pos.len());
                self.charge(est)?;
                let src = self.doc.node(m.source).ok_or_else(missing)?;
                let mut node = src.clone();
                node.id = m.id;
                node.parent = m.parent;
                node.children.clear();
                node.source = Some(m.source);
                node.overrides = m.overrides;
                for delta in m.deltas {
                    apply_field_delta(&mut node, delta, &self.vectors)?;
                }
                let actual = est_node_bytes(&node).saturating_add(m.pos.len());
                (node, m.pos, actual)
            }
        };

        let id = node.id;
        insert_decoded_node(&mut self.doc, &mut self.positions, node, pos)?;
        self.touched.insert(id);
        self.node_sizes.insert(id, size);
        Ok(())
    }
}

struct VectorsSeed<'a>(&'a mut DecodeState);
impl<'de> DeserializeSeed<'de> for VectorsSeed<'_> {
    type Value = ();
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_seq(self)
    }
}
impl<'de> Visitor<'de> for VectorsSeed<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("vector networks")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        while let Some(net) = seq.next_element::<VectorNetwork>()? {
            let size = est_vector_bytes(&net);
            if let Err(err) = self.0.charge(size) {
                return Err(self.0.fail(err));
            }
            self.0.vector_sizes.push(size);
            self.0.vectors.push(figcraft_doc::Shared::new(net));
        }
        Ok(())
    }
}

struct RecordsSeed<'a>(&'a mut DecodeState);
impl<'de> DeserializeSeed<'de> for RecordsSeed<'_> {
    type Value = ();
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_seq(self)
    }
}
impl<'de> Visitor<'de> for RecordsSeed<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("delta snapshot records")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        loop {
            if self.0.records_seen >= self.0.max_nodes {
                if seq.next_element::<de::IgnoredAny>()?.is_some() {
                    let max = self.0.max_nodes;
                    return Err(self.0.fail(Error::Invalid(format!("delta snapshot exceeds the {max}-node limit"))));
                }
                break;
            }
            let Some(rec) = seq.next_element::<WireRecord>()? else {
                break;
            };
            self.0.records_seen += 1;
            if let Err(err) = self.0.ingest_record(rec) {
                return Err(self.0.fail(err));
            }
        }
        Ok(())
    }
}

struct DeltaWireSeed<'a>(&'a mut DecodeState);
impl<'de> DeserializeSeed<'de> for DeltaWireSeed<'_> {
    type Value = ();
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for DeltaWireSeed<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("delta snapshot payload")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "meta" => {
                    let meta: WireMeta = map.next_value()?;
                    let img_bytes = meta.images.iter().fold(0usize, |a, i| a.saturating_add(i.bytes.len()));
                    if let Err(err) = self.0.charge(img_bytes) {
                        return Err(self.0.fail(err));
                    }
                    self.0.meta = Some(meta);
                }
                "vectors" => map.next_value_seed(VectorsSeed(self.0))?,
                "records" => map.next_value_seed(RecordsSeed(self.0))?,
                _ => {
                    let _ = map.next_value::<de::IgnoredAny>()?;
                }
            }
        }
        Ok(())
    }
}

fn stream_decode_wire(raw: &[u8], max_nodes: usize, max_materialized_bytes: usize) -> Result<Replica, Error> {
    let mut state = DecodeState::new(max_nodes, max_materialized_bytes);
    let mut de = serde_json::Deserializer::from_slice(raw);
    let res = DeltaWireSeed(&mut state).deserialize(&mut de);
    if let Some(err) = state.abort {
        return Err(err);
    }
    res.map_err(|e| Error::Decode(format!("delta snapshot json: {e}")))?;
    de.end().map_err(|e| Error::Decode(format!("delta snapshot trailing data: {e}")))?;

    let meta = state.meta.ok_or_else(|| Error::Decode("delta snapshot missing meta".into()))?;
    apply_meta(&mut state.doc, meta);
    relist_pages(&mut state.doc, &state.positions);
    if state.doc.pages.is_empty() {
        return Err(Error::Invalid("a snapshot needs a page".into()));
    }
    state.doc.published = None;
    state.doc.take_dirty();
    let replica = Replica::from_parts(state.doc, state.positions);
    replica.check(&state.touched)?;
    Ok(replica)
}

fn insert_decoded_node(doc: &mut Document, positions: &mut Positions, node: Node, pos: String) -> Result<(), Error> {
    let (id, is_page, parent) = (node.id, node.is_page(), node.parent);
    match (is_page, parent) {
        (true, None) => {
            doc.insert_page(node).map_err(|e| Error::Invalid(e.to_string()))?;
        }
        (false, Some(p)) => {
            let siblings = &doc.get(p).map_err(|e| Error::Invalid(e.to_string()))?.children;
            let at = siblings.partition_point(|c| (positions.get(c).map_or("", String::as_str), *c) < (pos.as_str(), id));
            doc.insert(node, p, Some(at)).map_err(|e| Error::Invalid(e.to_string()))?;
        }
        _ => return Err(Error::Invalid("pages, and only pages, have no parent".into())),
    }
    positions.insert(id, pos);
    Ok(())
}

fn apply_meta(doc: &mut Document, mut meta: WireMeta) {
    doc.name = meta.name;
    doc.kind = meta.kind;
    doc.styles = meta.styles;
    doc.variables = meta.variables;
    doc.current_page = meta.current_page;
    doc.document_colors = meta.document_colors;
    doc.comments = meta.comments;
    doc.libraries = Arc::new(meta.libraries);
    doc.color_profile = meta.color_profile;
    doc.image_presets = meta.image_presets;
    clamp_design_review_config(&mut meta.design_review);
    doc.design_review = meta.design_review;
    doc.annotation_categories = meta.annotation_categories;
    doc.dev_snapshots = meta.dev_snapshots.into_iter().map(|(k, v)| (k, Arc::new(v))).collect();
    doc.timer = meta.timer;
    doc.vote = meta.vote;
    doc.votes = meta.votes;
    doc.vote_history = meta.vote_history;
    for wi in meta.images {
        // Leave out images that fail validation or attempt to replace bytes under an existing
        // key, matching legacy `Replica::from_snapshot_leaving_out`.
        let Ok(img) = validate_image_base64(&wi.key, wi.width, wi.height, &wi.bytes) else {
            continue;
        };
        if doc.images.get(&wi.key).is_some_and(|old| old.bytes != img.bytes) {
            continue;
        }
        doc.images.insert(wi.key, img);
    }
}

fn relist_pages(doc: &mut Document, positions: &Positions) {
    let store = doc.libraries.store;
    let mut pages: Vec<NodeId> = doc.nodes().filter(|n| n.is_page() && Some(n.id) != store).map(|n| n.id).collect();
    pages.sort_by(|a, b| (positions.get(a).map_or("", String::as_str), *a).cmp(&(positions.get(b).map_or("", String::as_str), *b)));
    doc.pages = pages;
}
