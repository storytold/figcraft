//! Tests for compact `.figcraft` mirror-delta and vector-table encoding.

use figcraft_doc::color::{Color, Paint};
use figcraft_doc::geom::{Affine, Point};
use figcraft_doc::{Document, Guide, NodeKind, TextData, VectorNetwork};

use super::*;

fn star_vector(points: usize, radius: f64) -> VectorNetwork {
    let pts: Vec<Point> = (0..points)
        .map(|i| {
            let angle = (i as f64) * std::f64::consts::TAU / (points as f64);
            let r = if i % 2 == 0 { radius } else { radius * 0.5 };
            Point::new(angle.cos() * r, angle.sin() * r)
        })
        .collect();
    VectorNetwork::from_points(&pts, true)
}

fn assert_docs_exact(a: &Document, b: &Document) {
    assert_eq!(a.node_count(), b.node_count());
    assert_eq!(a.pages, b.pages);
    assert_eq!(a.name, b.name);
    for na in a.nodes() {
        let nb = b.node(na.id).expect("missing node");
        assert_eq!(na, nb, "node {} `{}`", na.id.0, na.name);
        assert_eq!(na.width.to_bits(), nb.width.to_bits(), "width bits on {}", na.id.0);
        assert_eq!(na.height.to_bits(), nb.height.to_bits(), "height bits on {}", na.id.0);
        assert_eq!(na.opacity.to_bits(), nb.opacity.to_bits(), "opacity bits on {}", na.id.0);
        for (pa, pb) in na.fills.iter().zip(&nb.fills) {
            assert_eq!(pa.color_alpha.to_bits(), pb.color_alpha.to_bits(), "color_alpha on {}", na.id.0);
        }
    }
}

fn build_component_instances_doc(instances: usize, layers_per_component: usize, icon_vertices: usize) -> Document {
    let mut doc = Document::new("DesignSystem");
    let page = doc.first_page().unwrap();
    let comp = doc
        .create(
            page,
            NodeKind::Component { description: "Card".into(), props: Default::default(), prop_order: Vec::new() },
            "Card",
            0.0,
            0.0,
            400.0,
            300.0,
        )
        .unwrap();
    let icon_net = star_vector(icon_vertices, 12.0);
    let mut src_children = Vec::with_capacity(layers_per_component);
    for i in 0..layers_per_component {
        let kind = match i % 4 {
            0 => NodeKind::Text(TextData::new(format!("Label {i}"))),
            1 => NodeKind::Vector { network: icon_net.clone() },
            _ => NodeKind::Rectangle,
        };
        let cid = doc.create(comp, kind, &format!("L{i}"), (i % 8) as f64 * 48.0, (i / 8) as f64 * 56.0, 40.0, 40.0).unwrap();
        src_children.push(cid);
    }

    for k in 0..instances {
        let inst = doc
            .create(
                page,
                NodeKind::Instance { component: comp, props: Default::default() },
                "Card",
                ((k % 32) as f64) * 420.0,
                ((k / 32) as f64) * 320.0,
                400.0,
                300.0,
            )
            .unwrap();
        doc.get_mut(inst).unwrap().source = Some(comp);
        for (idx, &src_id) in src_children.iter().enumerate() {
            let src_node = doc.node(src_id).unwrap().clone();
            let mut copy = src_node;
            copy.id = doc.alloc_id();
            copy.parent = None;
            copy.children.clear();
            copy.source = Some(src_id);
            // Override one label per instance to exercise partial mirror deltas too.
            if idx == 0 && k % 3 == 0 {
                if let NodeKind::Text(ref mut t) = copy.kind {
                    t.characters = format!("Custom {k}");
                }
                copy.overrides.insert("characters".into());
            }
            doc.insert(copy, inst, None).unwrap();
        }
    }
    doc
}

#[test]
fn mirrored_layers_are_elided_and_roundtrip_deterministically() {
    let doc = build_component_instances_doc(128, 20, 32);
    let mut compact_json = Vec::new();
    write_json(&doc, &mut compact_json).unwrap();
    let full_json = serde_json::to_vec(&doc).unwrap();
    assert!(
        compact_json.len() * 4 < full_json.len(),
        "expected >4x reduction in document.json: compact={} vs full={}",
        compact_json.len(),
        full_json.len()
    );

    let text = std::str::from_utf8(&compact_json).unwrap();
    let (back, issues) = from_json(text).unwrap();
    assert!(issues.is_empty(), "{issues:?}");
    assert_docs_exact(&doc, &back);

    // Resaving produces byte-identical document.json.
    let mut second_json = Vec::new();
    write_json(&back, &mut second_json).unwrap();
    assert_eq!(compact_json, second_json);

    // Round-tripping through serde_json::Value (alphabetical key order) also works.
    let val = to_json(&doc).unwrap();
    let (from_val, val_issues) = from_json(&val.to_string()).unwrap();
    assert!(val_issues.is_empty(), "{val_issues:?}");
    assert_docs_exact(&doc, &from_val);

    // Full `.figcraft` archive + preview still works cleanly.
    let archive = save_bytes(&doc, None).unwrap();
    let (loaded, load_issues) = load_bytes(&archive).unwrap();
    assert!(load_issues.is_empty(), "{load_issues:?}");
    assert_docs_exact(&doc, &loaded);
    let preview = thumbnail::archive_preview(std::io::Cursor::new(&archive), 64);
    assert!(preview.page.is_some());
}

#[test]
fn mirror_deltas_preserve_default_overrides_negative_zero_color_alpha_and_nested_chains() {
    let mut doc = Document::new("EdgeCases");
    let page = doc.first_page().unwrap();
    let comp = doc
        .create(
            page,
            NodeKind::Component { description: String::new(), props: Default::default(), prop_order: Vec::new() },
            "C",
            0.0,
            0.0,
            100.0,
            100.0,
        )
        .unwrap();
    let src = doc.create(comp, NodeKind::Frame, "Box", 0.0, 0.0, 50.0, 50.0).unwrap();
    {
        let s = doc.get_mut(src).unwrap();
        s.clip = true;
        s.visible = false;
        s.opacity = 0.5;
        s.guides.push(Guide { vertical: true, offset: 12.0 });
        let mut p = Paint::solid(Color::WHITE);
        p.color_alpha = 0.25;
        s.fills = vec![p];
    }

    // Mirror 1 resets non-default source fields back to defaults, flips -0.0 and color_alpha.
    let mut m1 = doc.node(src).unwrap().clone();
    m1.id = doc.alloc_id();
    m1.parent = None;
    m1.children.clear();
    m1.source = Some(src);
    m1.clip = false;
    m1.visible = true;
    m1.opacity = 1.0;
    m1.guides.clear();
    m1.width = -0.0;
    m1.fills[0].color_alpha = 0.75;
    let m1_id = doc.insert(m1, page, None).unwrap();

    // Mirror 2 chains onto Mirror 1 and changes transform only.
    let mut m2 = doc.node(m1_id).unwrap().clone();
    m2.id = doc.alloc_id();
    m2.parent = None;
    m2.children.clear();
    m2.source = Some(m1_id);
    m2.transform = Affine::translate((20.0, 30.0));
    doc.insert(m2, page, None).unwrap();

    let v = to_json(&doc).unwrap();
    assert_eq!(v["nodes"][m1_id.0.to_string()]["mirror"], json!(true));
    let (back, issues) = from_json(&v.to_string()).unwrap();
    assert!(issues.is_empty(), "{issues:?}");
    assert_docs_exact(&doc, &back);
}

#[test]
fn repeated_non_mirror_vector_networks_are_deduplicated_in_vectors_table() {
    let mut doc = Document::new("Icons");
    let page = doc.first_page().unwrap();
    let icon = star_vector(48, 24.0);
    for i in 0..8 {
        doc.create(page, NodeKind::Vector { network: icon.clone() }, &format!("Icon {i}"), i as f64 * 30.0, 0.0, 24.0, 24.0).unwrap();
    }
    let val = to_json(&doc).unwrap();
    assert_eq!(val["vectors"].as_array().map(Vec::len), Some(1));
    let (back, issues) = from_json(&val.to_string()).unwrap();
    assert!(issues.is_empty(), "{issues:?}");
    assert_docs_exact(&doc, &back);

    let mut a = Vec::new();
    let mut b = Vec::new();
    write_json(&doc, &mut a).unwrap();
    write_json(&back, &mut b).unwrap();
    assert_eq!(a, b);
}

#[test]
fn cyclic_source_pointers_fall_back_on_encode_and_are_rejected_on_decode() {
    let mut doc = Document::new("Cycle");
    let page = doc.first_page().unwrap();
    let a = doc.create(page, NodeKind::Rectangle, "A", 0.0, 0.0, 10.0, 10.0).unwrap();
    let b = doc.create(page, NodeKind::Rectangle, "B", 10.0, 0.0, 10.0, 10.0).unwrap();
    doc.get_mut(a).unwrap().source = Some(b);
    doc.get_mut(b).unwrap().source = Some(a);

    // Encoder falls back to full nodes rather than hanging or producing invalid mirrors.
    let v = to_json(&doc).unwrap();
    assert!(v["nodes"][a.0.to_string()].get("mirror").is_none());
    assert!(v["nodes"][b.0.to_string()].get("mirror").is_none());
    let (back, _) = from_json(&v.to_string()).unwrap();
    assert_docs_exact(&doc, &back);

    // Hand-crafted cyclic mirror JSON is rejected cleanly.
    let cyclic = r#"{"format":"figcraft","version":1,"imageIndex":[],"name":"C","pages":[1],"next_id":4,"nodes":{
        "1":{"id":1,"name":"Page","type":"page","width":0.0,"height":0.0,"children":[2,3]},
        "2":{"mirror":true,"id":2,"source":3,"parent":1},
        "3":{"mirror":true,"id":3,"source":2,"parent":1}
    }}"#;
    assert!(matches!(from_json(cyclic), Err(FormatError::Corrupt(_))));

    // Self-mirror and missing source are rejected cleanly.
    let self_mirror = r#"{"format":"figcraft","version":1,"imageIndex":[],"name":"C","pages":[1],"next_id":3,"nodes":{
        "1":{"id":1,"name":"Page","type":"page","width":0.0,"height":0.0,"children":[2]},
        "2":{"mirror":true,"id":2,"source":2,"parent":1}
    }}"#;
    assert!(matches!(from_json(self_mirror), Err(FormatError::Corrupt(_))));

    let missing_source = r#"{"format":"figcraft","version":1,"imageIndex":[],"name":"C","pages":[1],"next_id":3,"nodes":{
        "1":{"id":1,"name":"Page","type":"page","width":0.0,"height":0.0,"children":[2]},
        "2":{"mirror":true,"id":2,"source":999,"parent":1}
    }}"#;
    assert!(matches!(from_json(missing_source), Err(FormatError::Corrupt(_))));
}

#[test]
fn invalid_vector_refs_and_amplification_bombs_are_rejected() {
    let bad_ref = r#"{"format":"figcraft","version":1,"imageIndex":[],"name":"V","pages":[1],"next_id":3,"vectors":[],"nodes":{
        "1":{"id":1,"name":"Page","type":"page","width":0.0,"height":0.0,"children":[2]},
        "2":{"id":2,"name":"Bad","type":"vector","vectorRef":0,"parent":1,"width":10.0,"height":10.0}
    }}"#;
    assert!(matches!(from_json(bad_ref), Err(FormatError::Corrupt(_))));

    let both_net_and_ref = r#"{"format":"figcraft","version":1,"imageIndex":[],"name":"V","pages":[1],"next_id":3,"vectors":[{"vertices":[],"segments":[],"regions":[]}],"nodes":{
        "1":{"id":1,"name":"Page","type":"page","width":0.0,"height":0.0,"children":[2]},
        "2":{"id":2,"name":"Bad","type":"vector","vectorRef":0,"network":{"vertices":[],"segments":[],"regions":[]},"parent":1,"width":10.0,"height":10.0}
    }}"#;
    assert!(matches!(from_json(both_net_and_ref), Err(FormatError::Corrupt(_))));

    // Amplification bomb: 1 heavy vector source mirrored 1,000 times against a small budget.
    let doc = build_component_instances_doc(50, 4, 256);
    let mut json_bytes = Vec::new();
    write_json(&doc, &mut json_bytes).unwrap();
    let text = std::str::from_utf8(&json_bytes).unwrap();
    assert!(compact::materialized_budget(text.len()) > 0);
    assert!(matches!(compact::parse_compact_document_with_budget(text, 64 * 1024), Err(FormatError::Corrupt(_))));
}

#[test]
fn truncation_and_bit_flip_mutations_never_panic() {
    let doc = build_component_instances_doc(4, 6, 16);
    let mut raw = Vec::new();
    write_json(&doc, &mut raw).unwrap();
    // Truncation sweep across stride points.
    let step = (raw.len() / 64).max(1);
    for len in (0..raw.len()).step_by(step) {
        if let Ok(s) = std::str::from_utf8(&raw[..len]) {
            let _ = from_json(s);
        }
    }
    // Single-byte bit-flip sweep across stride points.
    for idx in (0..raw.len()).step_by(step) {
        let mut mutated = raw.clone();
        mutated[idx] ^= 0x55;
        if let Ok(s) = std::str::from_utf8(&mutated) {
            let _ = from_json(s);
        }
    }
}

#[test]
#[ignore = "benchmark: run with cargo test --release -p figcraft-format bench_compact_format -- --ignored --nocapture"]
fn bench_compact_format() {
    use std::time::Instant;

    let cases: [(&str, Document); 3] = [
        ("42k heavy instances (64-vtx icons)", build_component_instances_doc(1024, 40, 64)),
        ("42k light instances (no vectors)", {
            let mut doc = Document::new("Light");
            let page = doc.first_page().unwrap();
            let comp = doc
                .create(
                    page,
                    NodeKind::Component { description: String::new(), props: Default::default(), prop_order: Vec::new() },
                    "Card",
                    0.0,
                    0.0,
                    400.0,
                    300.0,
                )
                .unwrap();
            let mut kids = Vec::new();
            for i in 0..40 {
                let kind = if i % 4 == 0 { NodeKind::Text(TextData::new(format!("Label {i}"))) } else { NodeKind::Rectangle };
                kids.push(doc.create(comp, kind, &format!("L{i}"), (i % 8) as f64 * 48.0, (i / 8) as f64 * 56.0, 40.0, 40.0).unwrap());
            }
            for k in 0..1024 {
                let inst = doc
                    .create(
                        page,
                        NodeKind::Instance { component: comp, props: Default::default() },
                        "Card",
                        (k % 32) as f64 * 420.0,
                        (k / 32) as f64 * 320.0,
                        400.0,
                        300.0,
                    )
                    .unwrap();
                doc.get_mut(inst).unwrap().source = Some(comp);
                for &src_id in &kids {
                    let mut c = doc.node(src_id).unwrap().clone();
                    c.id = doc.alloc_id();
                    c.parent = None;
                    c.children.clear();
                    c.source = Some(src_id);
                    doc.insert(c, inst, None).unwrap();
                }
            }
            doc
        }),
        ("42k flat (no instances)", {
            let mut doc = Document::new("Flat");
            let page = doc.first_page().unwrap();
            for i in 0..42_000 {
                let kind = if i % 4 == 0 { NodeKind::Text(TextData::new(format!("Label {i}"))) } else { NodeKind::Rectangle };
                doc.create(page, kind, &format!("L{i}"), (i % 100) as f64 * 50.0, (i / 100) as f64 * 50.0, 40.0, 40.0).unwrap();
            }
            doc
        }),
    ];

    for (label, doc) in &cases {
        let t0 = Instant::now();
        let mut legacy_json = Vec::new();
        craft_figcraft_format::write_document_json(&FiniteValue(doc), &native_image_index(doc), &mut legacy_json).unwrap();
        let legacy_write = t0.elapsed();

        let t1 = Instant::now();
        let legacy_text = std::str::from_utf8(&legacy_json).unwrap();
        let (_legacy_back, _) = craft_figcraft_format::read_document_json::<Document>(legacy_text).unwrap();
        let legacy_read = t1.elapsed();

        let t2 = Instant::now();
        let mut compact_json = Vec::new();
        write_json(doc, &mut compact_json).unwrap();
        let compact_write = t2.elapsed();

        let t3 = Instant::now();
        let compact_text = std::str::from_utf8(&compact_json).unwrap();
        let (compact_back, issues) = from_json(compact_text).unwrap();
        let compact_read = t3.elapsed();
        assert!(issues.is_empty());
        assert_eq!(compact_back.node_count(), doc.node_count());

        let t4 = Instant::now();
        let zip_bytes = save_bytes(doc, None).unwrap();
        let zip_save = t4.elapsed();

        let t5 = Instant::now();
        let (_zip_back, zip_issues) = load_bytes(&zip_bytes).unwrap();
        let zip_load = t5.elapsed();
        assert!(zip_issues.is_empty());

        eprintln!(
            "[{label}] nodes={}\n  document.json: {:.2} MB -> {:.2} MB ({:.2}x)\n  write_json: {:.1} ms -> {:.1} ms ({:.2}x)\n  from_json:  {:.1} ms -> {:.1} ms ({:.2}x)\n  .figcraft zip: {:.2} MB (save {:.1} ms, load {:.1} ms)",
            doc.node_count(),
            legacy_json.len() as f64 / 1_048_576.0,
            compact_json.len() as f64 / 1_048_576.0,
            legacy_json.len() as f64 / compact_json.len() as f64,
            legacy_write.as_secs_f64() * 1000.0,
            compact_write.as_secs_f64() * 1000.0,
            legacy_write.as_secs_f64() / compact_write.as_secs_f64(),
            legacy_read.as_secs_f64() * 1000.0,
            compact_read.as_secs_f64() * 1000.0,
            legacy_read.as_secs_f64() / compact_read.as_secs_f64(),
            zip_bytes.len() as f64 / 1_048_576.0,
            zip_save.as_secs_f64() * 1000.0,
            zip_load.as_secs_f64() * 1000.0,
        );
    }
}
