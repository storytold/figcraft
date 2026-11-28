//! Parity, exact-equality and hostile-input tests for `CatchUp::DeltaSnapshot`.

use std::collections::BTreeMap;
use std::io::Write;
use std::sync::Arc;

use figcraft_color::{Color, Paint};
use figcraft_doc::{
    BoardTimer, Comment, DesignReview, DevSnapshot, Document, ImageData, ImagePreset, Libraries, NodeId, NodeKind, Segment, TextData, VectorNetwork,
    Vertex, Vote, VoteRecord,
};
use figcraft_geom::Affine;

use crate::delta_snapshot::{DELTA_MAGIC, DELTA_VERSION, decode_delta_snapshot, decode_delta_snapshot_with_budget, encode_delta_snapshot};
use crate::message::{self, CatchUp, ClientMsg, ServerMsg};
use crate::{Client, Error, Limits, Outgoing, Replica, Server};

pub(crate) const LEGACY_CLIENT_VERSION: &str = "0.18.1 signals hosting comments packed design-review branches";

/// Compare two documents via serialized JSON (excluding per-client `next_id`) so
/// non-structural `PartialEq` cannot hide silent data loss.
pub(crate) fn assert_docs_json_eq(left: &Document, right: &Document, context: &str) {
    let mut lj = serde_json::to_value(left).expect("serialize left doc");
    let mut rj = serde_json::to_value(right).expect("serialize right doc");
    if let Some(m) = lj.as_object_mut() {
        m.remove("next_id");
    }
    if let Some(m) = rj.as_object_mut() {
        m.remove("next_id");
    }
    assert_eq!(lj, rj, "{context}: serialized JSON mismatch");
}

fn make_raw_delta_payload(json: &str, declared_override: Option<u32>) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(DELTA_MAGIC);
    out.push(DELTA_VERSION);
    let len = declared_override.unwrap_or(u32::try_from(json.len()).unwrap());
    out.extend_from_slice(&len.to_le_bytes());
    let mut z = flate2::write::DeflateEncoder::new(out, flate2::Compression::fast());
    z.write_all(json.as_bytes()).unwrap();
    z.finish().unwrap()
}

pub(crate) fn sample_vector_icon(seed: usize, vertex_count: usize) -> VectorNetwork {
    let mut vertices = Vec::with_capacity(vertex_count);
    let mut segments = Vec::with_capacity(vertex_count);
    for i in 0..vertex_count {
        let a = (i as f64) * std::f64::consts::TAU / (vertex_count as f64) + (seed as f64) * 0.1;
        vertices.push(Vertex::new(16.0 + 12.0 * a.cos(), 16.0 + 12.0 * a.sin()));
        segments.push(Segment { a: i, b: (i + 1) % vertex_count, ta: (1.5, -0.5), tb: (-1.5, 0.5) });
    }
    VectorNetwork { vertices, segments, regions: Vec::new() }
}

/// Build a document with cross-page instances (main component on page 2, instances on page 1),
/// depth-2 mirror chains, repeated vector icons and stale empty `overrides` (#1819).
pub(crate) fn design_system_doc(num_instances: usize) -> (Document, NodeId, Vec<NodeId>) {
    let mut doc = Document::new("DesignSystemDST");
    let page1 = doc.first_page().unwrap();
    let page2 = doc.add_page("Components");

    let comp = doc
        .create(
            page2,
            NodeKind::Component { description: "Card".into(), props: BTreeMap::new(), prop_order: Vec::new() },
            "Card",
            0.0,
            0.0,
            320.0,
            200.0,
        )
        .unwrap();

    let icon_a = sample_vector_icon(1, 24);
    let icon_b = sample_vector_icon(2, 24);
    let mut main_children = Vec::new();
    for i in 0..6 {
        let kind = match i {
            0 | 1 => NodeKind::Text(TextData::new(format!("Title {i}")).into()),
            2 => NodeKind::Vector { network: icon_a.clone().into() },
            3 => NodeKind::Vector { network: icon_b.clone().into() },
            _ => NodeKind::Rectangle,
        };
        let cid = doc.create(comp, kind, &format!("Part {i}"), i as f64 * 24.0, 8.0, 32.0, 32.0).unwrap();
        if let Some(n) = doc.node_mut(cid) {
            n.fills = vec![Paint::solid(Color::from_rgb8_const(40, 100, (i * 30) as u8))];
        }
        main_children.push(cid);
    }

    let main_snapshots: Vec<_> = main_children.iter().map(|id| doc.node(*id).unwrap().clone()).collect();
    let mut prev_inst_children: Option<Vec<NodeId>> = None;

    for idx in 0..num_instances {
        let mut root = doc.node(comp).unwrap().clone();
        let inst_id = doc.alloc_id();
        root.id = inst_id;
        root.name = format!("Card #{idx}");
        root.kind = NodeKind::Instance { component: comp, props: BTreeMap::new() };
        root.children.clear();
        root.parent = None;
        root.source = Some(comp);
        root.transform = Affine::translate((idx as f64 * 340.0, 0.0));
        doc.insert(root, page1, None).unwrap();

        let mut this_children = Vec::new();
        for (j, src) in main_snapshots.iter().enumerate() {
            let mut child = src.clone();
            child.id = doc.alloc_id();
            child.parent = None;
            child.children.clear();
            let chain_src = prev_inst_children.as_ref().filter(|_| idx % 3 == 2).map_or(src.id, |prev| prev[j]);
            child.source = Some(chain_src);

            if j == 0 && idx % 2 == 0 {
                if let NodeKind::Text(t) = &mut child.kind {
                    t.characters = format!("Overridden {idx}");
                }
                if idx % 4 != 0 {
                    child.overrides.insert("characters".to_string());
                }
            }
            if j == 4 && idx % 5 == 0 {
                child.width = 99.0;
                child.transform = Affine::translate((12.0, 24.0));
            }
            let child_id = child.id;
            doc.insert(child, inst_id, None).unwrap();
            this_children.push(child_id);
        }
        prev_inst_children = Some(this_children);
    }

    doc.take_dirty();
    (doc, comp, main_children)
}

#[test]
fn mirror_delta_preserves_paint_color_alpha_and_signed_zero() {
    let mut doc = Document::new("ExactEq");
    let page = doc.first_page().unwrap();
    let src = doc.create(page, NodeKind::Rectangle, "Src", 0.0, 0.0, 100.0, 50.0).unwrap();
    let mut src_paint = Paint::solid(Color::from_rgb8_const(20, 80, 200));
    src_paint.opacity = 0.5;
    src_paint.color_alpha = 0.5;
    if let Some(n) = doc.node_mut(src) {
        n.fills = vec![src_paint.clone()];
        n.strokes = vec![src_paint];
    }

    let mut mirror = doc.node(src).unwrap().clone();
    mirror.id = doc.alloc_id();
    mirror.parent = None;
    mirror.source = Some(src);
    mirror.fills[0].color_alpha = 1.0;
    mirror.strokes[0].color_alpha = 0.25;
    mirror.width = -0.0;
    mirror.transform = Affine::translate((-0.0, 0.0));
    assert_eq!(doc.node(src).unwrap().fills, mirror.fills, "Paint PartialEq ignores color_alpha");
    doc.insert(mirror, page, None).unwrap();
    doc.take_dirty();

    let replica = Replica::new(doc);
    let decoded = decode_delta_snapshot(&encode_delta_snapshot(&replica).unwrap(), 16 << 20, Limits::default().max_nodes).unwrap();
    let legacy = Replica::from_snapshot(replica.snapshot().unwrap()).unwrap();
    assert_docs_json_eq(replica.document(), decoded.document(), "exact structural equality");
    assert_docs_json_eq(legacy.document(), decoded.document(), "legacy parity");
}

#[test]
fn corrupt_hash_image_matches_legacy_join_parity() {
    let mut doc = Document::new("BadImage");
    let page = doc.first_page().unwrap();
    doc.create(page, NodeKind::Rectangle, "Box", 0.0, 0.0, 40.0, 40.0).unwrap();
    let bad_key = "0".repeat(64);
    doc.images.insert(bad_key.clone(), ImageData { bytes: Arc::new(b"not-matching-sha256".to_vec()), width: 10, height: 10 });
    doc.images.insert("svg_icon_1".into(), ImageData { bytes: Arc::new(b"valid-bytes".to_vec()), width: 16, height: 16 });
    doc.take_dirty();

    let mut srv = Server::new(doc, "tok", Limits::default());
    let join_with = |srv: &mut Server, ver: Option<&str>| -> Client {
        let ClientMsg::Hello(mut h) = Client::hello("f", "tok", "u", "#111") else { panic!() };
        if let Some(v) = ver {
            h.client_version = v.into();
        }
        let (id, outs) = srv.join(h).unwrap();
        let w = outs
            .into_iter()
            .find_map(|o| match o {
                Outgoing::To(t, m @ ServerMsg::Welcome { .. }) if t == id => Some(m),
                _ => None,
            })
            .unwrap();
        Client::join(w).unwrap()
    };

    let legacy_client = join_with(&mut srv, Some(LEGACY_CLIENT_VERSION));
    let delta_client = join_with(&mut srv, None);
    assert!(!delta_client.document().images.contains_key(&bad_key));
    assert!(delta_client.document().images.contains_key("svg_icon_1"));
    assert_docs_json_eq(legacy_client.document(), delta_client.document(), "corrupt-image parity");
}

#[test]
fn forged_declared_length_cannot_bypass_unpack_limit() {
    let huge_json = format!(
        r#"{{"meta":{{"name":"Bomb"}},"records":[{{"r":"full","pos":"a0","node":{{"id":1,"name":"{}","type":"page","width":0,"height":0}}}}]}}"#,
        "A".repeat(4 << 20)
    );
    let mut welcome = ServerMsg::Welcome {
        session: 1,
        client_id: 1,
        seq: 0,
        catch_up: CatchUp::DeltaSnapshot(make_raw_delta_payload(&huge_json, Some(10))),
        processed: message::Processed::default(),
        peers: Vec::new(),
        access: message::Access::Edit,
        secret: 1,
    };
    message::unpack(&mut welcome, 1 << 20).unwrap();
    let err = Client::join(welcome).unwrap_err();
    assert!(matches!(err, Error::TooLarge(10)), "forged length must abort at declared+1, got {err:?}");
}

#[test]
fn mirror_and_vref_amplification_bombs_are_rejected_within_memory_budget() {
    let big_text = "X".repeat(256 * 1024);
    let mut mirror_recs = String::from(
        r#"{"meta":{"name":"MirrorBomb"},"records":[{"r":"full","pos":"a0","node":{"id":1,"name":"P","type":"page","width":0,"height":0}},"#,
    );
    mirror_recs.push_str(&format!(
        r#"{{"r":"full","pos":"a0","node":{{"id":2,"name":"Big","parent":1,"type":"text","characters":"{big_text}","width":10,"height":10}}}}"#
    ));
    for i in 3..2003u64 {
        mirror_recs.push_str(&format!(r#",{{"r":"mirror","id":{i},"parent":1,"pos":"a{i:04}1","source":2}}"#));
    }
    mirror_recs.push_str("]}");
    let err1 = decode_delta_snapshot(&make_raw_delta_payload(&mirror_recs, None), 16 << 20, Limits::default().max_nodes).unwrap_err();
    assert!(matches!(err1, Error::TooLarge(_)), "mirror bomb must fail TooLarge, got {err1:?}");

    let net_json = serde_json::to_string(&sample_vector_icon(1, 20_000)).unwrap();
    let mut vref_recs = format!(
        r#"{{"meta":{{"name":"VrefBomb"}},"vectors":[{net_json}],"records":[{{"r":"full","pos":"a0","node":{{"id":1,"name":"P","type":"page","width":0,"height":0}}}}"#
    );
    for i in 2..3002u64 {
        vref_recs.push_str(&format!(
            r#",{{"r":"full","pos":"a{i:04}1","vref":0,"node":{{"id":{i},"name":"V","parent":1,"type":"vector","network":{{"vertices":[],"segments":[]}},"width":10,"height":10}}}}"#
        ));
    }
    vref_recs.push_str("]}");
    let err2 = decode_delta_snapshot(&make_raw_delta_payload(&vref_recs, None), 16 << 20, Limits::default().max_nodes).unwrap_err();
    assert!(matches!(err2, Error::TooLarge(_)), "vref bomb must fail TooLarge, got {err2:?}");
}

#[test]
fn streaming_node_budget_and_hostile_inputs_are_rejected_cleanly() {
    let mut many =
        String::from(r#"{"meta":{"name":"Many"},"records":[{"r":"full","pos":"a0","node":{"id":1,"name":"P","type":"page","width":0,"height":0}}"#);
    for i in 2..502u64 {
        many.push_str(&format!(
            r#",{{"r":"full","pos":"a{i:04}1","node":{{"id":{i},"name":"R","parent":1,"type":"rectangle","width":1,"height":1}}}}"#
        ));
    }
    many.push_str("]}");
    let err = decode_delta_snapshot_with_budget(&make_raw_delta_payload(&many, None), 16 << 20, 10, 64 << 20).unwrap_err();
    assert!(matches!(&err, Error::Invalid(msg) if msg.contains("10-node limit")), "got {err:?}");

    let cyclic = r#"{"meta":{"name":"Evil"},"records":[
        {"r":"full","pos":"a0","node":{"id":1,"name":"P","type":"page","width":0,"height":0}},
        {"r":"mirror","id":2,"parent":1,"pos":"a0","source":3},
        {"r":"mirror","id":3,"parent":1,"pos":"a1","source":2}
    ]}"#;
    let self_cycle = r#"{"meta":{"name":"Self"},"records":[
        {"r":"full","pos":"a0","node":{"id":1,"name":"P","type":"page","width":0,"height":0}},
        {"r":"mirror","id":2,"parent":1,"pos":"a0","source":2}
    ]}"#;
    assert!(matches!(decode_delta_snapshot(&make_raw_delta_payload(cyclic, None), 1 << 20, 100), Err(Error::Invalid(_))));
    assert!(matches!(decode_delta_snapshot(&make_raw_delta_payload(self_cycle, None), 1 << 20, 100), Err(Error::Invalid(_))));

    for bad in [vec![], b"FCD".to_vec(), b"XXXX\x01\x00\x00\x00\x00".to_vec(), b"FCDS\x99\x00\x00\x00\x00".to_vec(), vec![0xFF; 64]] {
        assert!(decode_delta_snapshot(&bad, 1 << 20, 1000).is_err());
    }
}

#[test]
fn full_metadata_and_design_review_roundtrip_matches_legacy_json() {
    let (mut doc, _, _) = design_system_doc(4);
    let page = doc.first_page().unwrap();
    doc.comments.push(Comment {
        id: 7,
        page,
        x: 10.0,
        y: 20.0,
        anchor: None,
        first: figcraft_doc::Message::new("Alice", "Check contrast", 1_700_000_000),
        resolved: false,
        replies: Vec::new(),
        read_by: None,
    });
    doc.dev_snapshots.insert(page, Arc::new(DevSnapshot { nodes: vec![Arc::new(doc.node(page).unwrap().clone())] }));
    doc.timer = Some(BoardTimer::Paused { left: 120_000 });
    let vote = Vote { per_person: 3, scope: Default::default(), ended: false, by: "Alice".into() };
    doc.vote = Some(vote.clone());
    doc.votes.entry("alice".into()).or_default().push(page);
    doc.vote_history.push(VoteRecord { vote, ballots: doc.votes.clone(), names: BTreeMap::new(), ended: 1_700_000_000 });
    doc.libraries = Arc::new(Libraries::default());
    doc.image_presets.push(ImagePreset { name: "Warm".into(), filters: Default::default() });
    doc.images.insert("logo_1".into(), ImageData { bytes: Arc::new(b"png-bytes".to_vec()), width: 32, height: 32 });
    doc.design_review = DesignReview { radii: vec![4.0; figcraft_doc::MAX_RADII + 10], ..DesignReview::default() };
    doc.take_dirty();

    let replica = Replica::new(doc);
    let legacy = Replica::from_snapshot(replica.snapshot().unwrap()).unwrap();
    let decoded = decode_delta_snapshot(&encode_delta_snapshot(&replica).unwrap(), 64 << 20, Limits::default().max_nodes).unwrap();
    assert_eq!(decoded.document().design_review.radii.len(), figcraft_doc::MAX_RADII);
    assert_docs_json_eq(legacy.document(), decoded.document(), "full metadata parity");
}

#[test]
fn encoder_safely_roundtrips_cyclic_source_references_as_full_records() {
    let mut doc = Document::new("CyclicInMemory");
    let page = doc.first_page().unwrap();
    let a = doc.create(page, NodeKind::Rectangle, "A", 0.0, 0.0, 50.0, 50.0).unwrap();
    let b = doc.create(page, NodeKind::Rectangle, "B", 60.0, 0.0, 50.0, 50.0).unwrap();
    doc.node_mut(a).unwrap().source = Some(b);
    doc.node_mut(b).unwrap().source = Some(a);
    doc.take_dirty();

    let replica = Replica::new(doc);
    let back = decode_delta_snapshot(&encode_delta_snapshot(&replica).unwrap(), 1 << 20, 100).unwrap();
    assert_docs_json_eq(replica.document(), back.document(), "cyclic source roundtrip");
}

#[test]
fn delta_snapshot_never_panics_when_truncated_or_bit_flipped() {
    let (doc, _, _) = design_system_doc(2);
    let valid = encode_delta_snapshot(&Replica::new(doc)).unwrap();
    // Truncations across header and compressed body.
    for cut in [0, 1, 4, 8, 9, 10, valid.len() / 2, valid.len() - 1] {
        assert!(decode_delta_snapshot(&valid[..cut], 16 << 20, 10_000).is_err());
    }
    // Single-bit flips across header and body must either error cleanly or produce a valid Replica.
    let stride = (valid.len() / 64).max(1);
    for idx in (0..valid.len()).step_by(stride) {
        let mut mutated = valid.clone();
        mutated[idx] ^= 0x80;
        let _ = decode_delta_snapshot(&mutated, 16 << 20, 10_000);
    }
}

#[test]
fn bound_map_amplification_and_oversized_metadata_are_rejected_by_budget() {
    // Source node with a large `bound` map mirrored 1,000 times must trip the memory budget.
    let mut bound_entries = Vec::with_capacity(200);
    for i in 0..200 {
        bound_entries.push(format!(r#""k{i:03}":"{}""#, "v".repeat(512)));
    }
    let bound_json = bound_entries.join(",");
    let mut recs = format!(
        r#"{{"meta":{{"name":"BoundBomb"}},"records":[{{"r":"full","pos":"a0","node":{{"id":1,"name":"P","type":"page","width":0,"height":0}}}},{{"r":"full","pos":"a0","node":{{"id":2,"name":"Src","parent":1,"type":"rectangle","width":10,"height":10,"bound":{{{bound_json}}}}}}}"#
    );
    for i in 3..1003u64 {
        recs.push_str(&format!(r#",{{"r":"mirror","id":{i},"parent":1,"pos":"a{i:04}1","source":2}}"#));
    }
    recs.push_str("]}");
    let err = decode_delta_snapshot(&make_raw_delta_payload(&recs, None), 16 << 20, Limits::default().max_nodes).unwrap_err();
    assert!(matches!(err, Error::TooLarge(_)), "bound-map amplification must fail TooLarge, got {err:?}");

    // Oversized metadata (`name` of 2 MiB) under a 1 MiB materialized budget must fail TooLarge.
    let big_meta = format!(
        r#"{{"meta":{{"name":"{}"}},"records":[{{"r":"full","pos":"a0","node":{{"id":1,"name":"P","type":"page","width":0,"height":0}}}}]}}"#,
        "M".repeat(2 << 20)
    );
    let err2 = decode_delta_snapshot_with_budget(&make_raw_delta_payload(&big_meta, None), 16 << 20, 100, 1 << 20).unwrap_err();
    assert!(matches!(err2, Error::TooLarge(_)), "oversized metadata must fail TooLarge, got {err2:?}");
}

#[test]
fn server_falls_back_to_legacy_snapshot_when_delta_exceeds_decode_budget() {
    // 2,000 mirrors of a 256 KiB text node exceed the client's 64x materialized ceiling:
    // the server must fall back to `CatchUp::Snapshot` so capable clients still join cleanly.
    let mut doc = Document::new("HighAmplificationHonest");
    let page = doc.first_page().unwrap();
    let src = doc.create(page, NodeKind::Text(TextData::new("T".repeat(256 * 1024)).into()), "Big", 0.0, 0.0, 100.0, 20.0).unwrap();
    let src_snap = doc.node(src).unwrap().clone();
    for _ in 0..2000 {
        let mut m = src_snap.clone();
        m.id = doc.alloc_id();
        m.parent = None;
        m.source = Some(src);
        doc.insert(m, page, None).unwrap();
    }
    doc.take_dirty();

    assert!(matches!(encode_delta_snapshot(&Replica::new(doc.clone())), Err(Error::TooLarge(_))));
    let mut srv = Server::new(doc, "tok", Limits::default());
    let ClientMsg::Hello(h) = Client::hello("f", "tok", "u", "#222") else { panic!() };
    let (id, outs) = srv.join(h).unwrap();
    let welcome = outs
        .into_iter()
        .find_map(|o| match o {
            Outgoing::To(t, m @ ServerMsg::Welcome { .. }) if t == id => Some(m),
            _ => None,
        })
        .unwrap();
    assert!(matches!(welcome, ServerMsg::Welcome { catch_up: CatchUp::Snapshot(_), .. }));
}
