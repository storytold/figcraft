//! Topological ordering, vector interning and deflate framing for `CatchUp::DeltaSnapshot`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufWriter, Write};

use figcraft_doc::{Document, Node, NodeId, NodeKind, VectorNetwork};
use sha2::{Digest as _, Sha256};

use super::decode::{est_deltas_extra_bytes, est_meta_bytes, est_node_bytes, est_vector_bytes, materialized_budget};
use super::fields::{diff_mirror_fields, exact_vector_network_eq};
use super::{DEFAULT_MAX_MATERIALIZED_BYTES, DELTA_MAGIC, DELTA_VERSION, DeltaWireRef, MirrorWire, WireImage, WireMeta, WireRecord};
use crate::Error;
use crate::replica::Replica;

/// Content-addressed deduplicator for `VectorNetwork`s within one snapshot.
#[derive(Default)]
pub(super) struct VectorInterner {
    by_hash: HashMap<[u8; 32], Vec<u32>>,
    table: Vec<VectorNetwork>,
}

impl VectorInterner {
    pub(super) fn intern(&mut self, net: &VectorNetwork) -> Result<u32, Error> {
        let raw = serde_json::to_vec(net).map_err(|e| Error::Encode(e.to_string()))?;
        let digest: [u8; 32] = Sha256::digest(&raw).into();
        if let Some(bucket) = self.by_hash.get(&digest) {
            for &idx in bucket {
                if usize::try_from(idx).ok().and_then(|i| self.table.get(i)).is_some_and(|existing| exact_vector_network_eq(existing, net)) {
                    return Ok(idx);
                }
            }
        }
        let idx = u32::try_from(self.table.len()).map_err(|_| Error::Encode("too many distinct vector networks".into()))?;
        self.table.push(net.clone());
        self.by_hash.entry(digest).or_default().push(idx);
        Ok(idx)
    }
}

/// Kahn topological order over `(parent, source)` edges (in-degree at most 2 per node).
///
/// Every parent precedes its children and every acyclic source precedes its mirrors. Nodes
/// caught in a `source` cycle are appended in parent-first order and emitted as `Full`.
fn topo_order(doc: &Document) -> Vec<NodeId> {
    let mut roots = doc.pages.clone();
    roots.extend(doc.nodes().filter(|n| n.parent.is_none() && !doc.pages.contains(&n.id)).map(|n| n.id));
    let mut parent_order = Vec::with_capacity(doc.node_count());
    for r in roots {
        parent_order.extend(doc.subtree(r));
    }

    let present: HashSet<NodeId> = parent_order.iter().copied().collect();
    let mut indeg: HashMap<NodeId, u8> = HashMap::with_capacity(present.len());
    let mut deps_of: HashMap<NodeId, Vec<NodeId>> = HashMap::with_capacity(present.len());

    for &id in &parent_order {
        let Some(n) = doc.node(id) else { continue };
        let mut d: u8 = 0;
        let p = n.parent.filter(|p| *p != id && present.contains(p));
        let s = n.source.filter(|s| *s != id && Some(*s) != p && present.contains(s));
        if let Some(pid) = p {
            deps_of.entry(pid).or_default().push(id);
            d += 1;
        }
        if let Some(sid) = s {
            deps_of.entry(sid).or_default().push(id);
            d += 1;
        }
        indeg.insert(id, d);
    }

    let mut q: VecDeque<NodeId> = parent_order.iter().copied().filter(|id| indeg.get(id).copied() == Some(0)).collect();
    let mut ordered = Vec::with_capacity(parent_order.len());
    let mut placed: HashSet<NodeId> = HashSet::with_capacity(parent_order.len());

    while let Some(u) = q.pop_front() {
        if !placed.insert(u) {
            continue;
        }
        ordered.push(u);
        if let Some(nexts) = deps_of.get(&u) {
            for &v in nexts {
                if let Some(deg) = indeg.get_mut(&v)
                    && *deg > 0
                {
                    *deg -= 1;
                    if *deg == 0 {
                        q.push_back(v);
                    }
                }
            }
        }
    }

    if ordered.len() < parent_order.len() {
        for id in parent_order {
            if placed.insert(id) {
                ordered.push(id);
            }
        }
    }
    ordered
}

/// Encode `replica` as a compressed `CatchUp::DeltaSnapshot` payload.
///
/// Returns `Error::TooLarge` if the payload would exceed the client decoder's materialized-memory
/// budget, allowing callers such as `Server::join_as` to fall back to `CatchUp::Snapshot`.
pub fn encode_delta_snapshot(replica: &Replica) -> Result<Vec<u8>, Error> {
    let doc = replica.document();
    let order = topo_order(doc);
    let mut interner = VectorInterner::default();
    let mut seen: HashMap<NodeId, &Node> = HashMap::with_capacity(order.len());
    let mut records = Vec::with_capacity(order.len());

    for id in order {
        let Some(node) = doc.node(id) else { continue };
        let pos = replica.key(id).to_string();
        let rec = encode_one_node(node, pos, &seen, &mut interner)?;
        records.push(rec);
        seen.insert(id, node);
    }

    let meta = build_meta(doc);
    let est_materialized = estimate_decode_charge(&meta, &interner.table, &records);
    let wire = DeltaWireRef { meta: &meta, vectors: &interner.table, records: &records };
    let (out, written) = compress_wire(&wire)?;
    let budget = materialized_budget(written, DEFAULT_MAX_MATERIALIZED_BYTES);
    if est_materialized > budget {
        return Err(Error::TooLarge(budget));
    }
    Ok(out)
}

/// Mirror the decoder's materialized-byte accounting so the server never emits a delta snapshot
/// that a compliant client would reject with `Error::TooLarge`.
fn estimate_decode_charge(meta: &WireMeta, vectors: &[VectorNetwork], records: &[WireRecord]) -> usize {
    let vector_sizes: Vec<usize> = vectors.iter().map(est_vector_bytes).collect();
    let mut total = est_meta_bytes(meta);
    for &vs in &vector_sizes {
        total = total.saturating_add(vs);
    }
    let mut node_sizes: HashMap<NodeId, usize> = HashMap::with_capacity(records.len());
    for rec in records {
        match rec {
            WireRecord::Full { pos, node, vref } => {
                let vsize = vref.and_then(|idx| usize::try_from(idx).ok()).and_then(|i| vector_sizes.get(i).copied()).unwrap_or(0);
                let est = est_node_bytes(node).saturating_add(pos.len()).saturating_add(vsize);
                total = total.saturating_add(est);
                node_sizes.insert(node.id, est);
            }
            WireRecord::Mirror(m) => {
                let src_size = node_sizes.get(&m.source).copied().unwrap_or(0);
                let est = src_size.saturating_add(est_deltas_extra_bytes(&m.deltas, &vector_sizes)).saturating_add(m.pos.len());
                let overrides = m.overrides.iter().fold(0usize, |a, s| a.saturating_add(s.len()).saturating_add(24));
                total = total.saturating_add(est);
                node_sizes.insert(m.id, est.saturating_add(overrides));
            }
        }
    }
    total
}

fn encode_one_node(node: &Node, pos: String, seen: &HashMap<NodeId, &Node>, interner: &mut VectorInterner) -> Result<WireRecord, Error> {
    if let Some(src_id) = node.source
        && let Some(src) = seen.get(&src_id)
    {
        let deltas = diff_mirror_fields(src, node, interner)?;
        return Ok(WireRecord::Mirror(Box::new(MirrorWire {
            id: node.id,
            parent: node.parent,
            pos,
            source: src_id,
            overrides: node.overrides.clone(),
            deltas,
        })));
    }

    let mut stripped = node.clone();
    // Children are rebuilt from `(parent, pos)` on decode.
    stripped.children.clear();
    let vref = match &mut stripped.kind {
        NodeKind::Vector { network } if !network.vertices.is_empty() || !network.segments.is_empty() || !network.regions.is_empty() => {
            let idx = interner.intern(network)?;
            *network = figcraft_doc::Shared::default();
            Some(idx)
        }
        _ => None,
    };
    Ok(WireRecord::Full { pos, node: Box::new(stripped), vref })
}

fn build_meta(doc: &Document) -> WireMeta {
    use base64::Engine as _;
    let images = doc
        .images
        .iter()
        .map(|(key, img)| WireImage {
            key: key.clone(),
            width: img.width,
            height: img.height,
            bytes: base64::engine::general_purpose::STANDARD.encode(img.bytes.as_slice()),
        })
        .collect();
    WireMeta {
        name: doc.name.clone(),
        kind: doc.kind,
        styles: doc.styles.clone(),
        variables: doc.variables.clone(),
        current_page: doc.current_page,
        document_colors: doc.document_colors.clone(),
        comments: doc.comments.clone(),
        libraries: (*doc.libraries).clone(),
        color_profile: doc.color_profile,
        image_presets: doc.image_presets.clone(),
        design_review: doc.design_review.clone(),
        annotation_categories: doc.annotation_categories.clone(),
        dev_snapshots: doc.dev_snapshots.iter().map(|(k, v)| (*k, (**v).clone())).collect(),
        timer: doc.timer,
        vote: doc.vote.clone(),
        votes: doc.votes.clone(),
        vote_history: doc.vote_history.clone(),
        images,
    }
}

struct CountingWriter<W> {
    inner: W,
    written: usize,
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.written = self.written.saturating_add(n);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn compress_wire(wire: &DeltaWireRef<'_>) -> Result<(Vec<u8>, usize), Error> {
    let mut out = Vec::with_capacity(4096);
    out.extend_from_slice(DELTA_MAGIC);
    out.push(DELTA_VERSION);
    out.extend_from_slice(&0u32.to_le_bytes());

    // Buffer writes into 64 KiB chunks so serde_json's small token writes do not enter deflate individually.
    let encoder = flate2::write::DeflateEncoder::new(out, flate2::Compression::fast());
    let mut buffered = BufWriter::with_capacity(64 * 1024, CountingWriter { inner: encoder, written: 0 });
    serde_json::to_writer(&mut buffered, wire).map_err(|e| Error::Encode(e.to_string()))?;
    let counting = buffered.into_inner().map_err(|e| Error::Encode(e.into_error().to_string()))?;
    let written = counting.written;
    let len_u32 = u32::try_from(written).map_err(|_| Error::TooLarge(u32::MAX as usize))?;
    let mut out = counting.inner.finish().map_err(|e| Error::Encode(e.to_string()))?;
    if let Some(slot) = out.get_mut(5..9) {
        slot.copy_from_slice(&len_u32.to_le_bytes());
    }
    Ok((out, written))
}
