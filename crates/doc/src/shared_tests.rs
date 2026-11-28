//! Copy-on-write structural sharing tests for [`Shared`] and `NodeKind` payloads.

use figcraft_geom::Point;

use crate::*;

#[test]
fn node_kind_stays_compact_with_shared_text_and_vector_payloads() {
    use std::mem::size_of;
    // Before wrapping TextData (352 B) and VectorNetwork (72 B) in Shared<T>, NodeKind was 352 B
    // and Node was 1608 B.
    assert!(size_of::<NodeKind>() <= 144, "NodeKind grew unexpectedly: {} bytes", size_of::<NodeKind>());
    assert!(size_of::<Node>() <= 1400, "Node grew unexpectedly: {} bytes", size_of::<Node>());
}

#[test]
fn shared_clone_shares_pointer_and_deref_mut_detaches_without_mutating_source() {
    let mut net = VectorNetwork::from_points(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0)], true);
    let a = Shared::new(net.clone());
    let mut b = a.clone();
    assert!(a.ptr_eq(&b));
    assert_eq!(a.strong_count(), 2);

    // Mutating through DerefMut detaches b and leaves a untouched.
    b.vertices[0].x = 42.0;
    assert!(!a.ptr_eq(&b));
    assert_eq!(a.strong_count(), 1);
    assert_eq!(b.strong_count(), 1);
    assert_eq!(a.vertices[0].x, 0.0);
    assert_eq!(b.vertices[0].x, 42.0);

    // Non-mutating reads never detach.
    net.vertices[0].x = 42.0;
    assert_eq!(*b, net);
    let c = b.clone();
    let _len = c.vertices.len();
    assert!(b.ptr_eq(&c));
}

#[test]
fn duplicated_text_and_vector_layers_share_payloads_until_edited() {
    let mut doc = Document::new("CoW");
    let page = doc.first_page().unwrap();
    let group = doc.create(page, NodeKind::Frame, "Card", 0.0, 0.0, 200.0, 100.0).unwrap();
    let net = VectorNetwork::from_points(&[Point::new(0.0, 0.0), Point::new(24.0, 0.0), Point::new(12.0, 24.0)], true);
    let vec_id = doc.create(group, NodeKind::Vector { network: net.into() }, "Icon", 0.0, 0.0, 24.0, 24.0).unwrap();
    let txt_id = doc.create(group, NodeKind::Text(TextData::new("Hello").into()), "Title", 32.0, 0.0, 100.0, 20.0).unwrap();

    let copy = doc.duplicate_into(group, page, None).unwrap();
    let copy_children = doc.node(copy).unwrap().children.clone();
    let (copy_vec, copy_txt) = (copy_children[0], copy_children[1]);

    // Both vector and text payloads share the exact same Arc right after duplication.
    match (&doc.node(vec_id).unwrap().kind, &doc.node(copy_vec).unwrap().kind) {
        (NodeKind::Vector { network: a }, NodeKind::Vector { network: b }) => assert!(a.ptr_eq(b)),
        _ => panic!("expected vector nodes"),
    }
    match (&doc.node(txt_id).unwrap().kind, &doc.node(copy_txt).unwrap().kind) {
        (NodeKind::Text(a), NodeKind::Text(b)) => assert!(a.ptr_eq(b)),
        _ => panic!("expected text nodes"),
    }

    // Editing the copy's text detaches only the copy's TextData; the vector stays shared.
    if let Some(NodeKind::Text(t)) = doc.node_mut(copy_txt).map(|n| &mut n.kind) {
        t.characters = "Edited".into();
    }
    match (&doc.node(txt_id).unwrap().kind, &doc.node(copy_txt).unwrap().kind) {
        (NodeKind::Text(a), NodeKind::Text(b)) => {
            assert!(!a.ptr_eq(b));
            assert_eq!(a.characters, "Hello");
            assert_eq!(b.characters, "Edited");
        }
        _ => panic!("expected text nodes"),
    }
    match (&doc.node(vec_id).unwrap().kind, &doc.node(copy_vec).unwrap().kind) {
        (NodeKind::Vector { network: a }, NodeKind::Vector { network: b }) => assert!(a.ptr_eq(b)),
        _ => panic!("expected vector nodes"),
    }
}

#[test]
fn shared_serialization_is_transparent_in_json() {
    let td = TextData::new("Transparent");
    let raw = serde_json::to_string(&td).unwrap();
    let wrapped = serde_json::to_string(&Shared::new(td.clone())).unwrap();
    assert_eq!(raw, wrapped);
    let back: Shared<TextData> = serde_json::from_str(&wrapped).unwrap();
    assert_eq!(*back, td);
}

#[test]
#[ignore = "benchmark: run with cargo test --release -p figcraft-doc bench_cow_sharing -- --ignored --nocapture"]
fn bench_cow_sharing() {
    use std::collections::HashSet;
    use std::mem::size_of;
    use std::time::Instant;

    let pts: Vec<Point> = (0..64)
        .map(|i| {
            let a = (i as f64) * std::f64::consts::TAU / 64.0;
            let r = if i % 2 == 0 { 16.0 } else { 8.0 };
            Point::new(a.cos() * r, a.sin() * r)
        })
        .collect();
    let icon = VectorNetwork::from_points(&pts, true);

    let mut doc = Document::new("BenchCoW");
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
    let mut kids = Vec::new();
    for i in 0..40 {
        let kind = match i % 4 {
            0 => NodeKind::Text(TextData::new(format!("Label {i}")).into()),
            1 => NodeKind::Vector { network: icon.clone().into() },
            _ => NodeKind::Rectangle,
        };
        kids.push(doc.create(comp, kind, &format!("L{i}"), (i % 8) as f64 * 48.0, (i / 8) as f64 * 56.0, 40.0, 40.0).unwrap());
    }

    let t0 = Instant::now();
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
    let build_ms = t0.elapsed().as_secs_f64() * 1000.0;

    let mut unique_vec_ptrs = HashSet::new();
    let mut unique_txt_ptrs = HashSet::new();
    let mut total_vecs = 0usize;
    let mut total_txts = 0usize;
    for n in doc.nodes() {
        match &n.kind {
            NodeKind::Vector { network } => {
                total_vecs += 1;
                unique_vec_ptrs.insert(std::sync::Arc::as_ptr(network.as_arc()) as usize);
            }
            NodeKind::Text(t) => {
                total_txts += 1;
                unique_txt_ptrs.insert(std::sync::Arc::as_ptr(t.as_arc()) as usize);
            }
            _ => {}
        }
    }

    let vec_heap_one = icon.vertices.len() * size_of::<Vertex>()
        + icon.segments.len() * size_of::<Segment>()
        + icon.regions.len() * size_of::<Region>()
        + icon.regions.iter().map(|r| r.loops.iter().map(|l| l.len() * size_of::<usize>()).sum::<usize>()).sum::<usize>();
    let vec_heap_naive = total_vecs * vec_heap_one;
    let vec_heap_shared = unique_vec_ptrs.len() * vec_heap_one;

    eprintln!(
        "Node={} B (was 1608 B), NodeKind={} B (was 352 B)\n42k build: {:.1} ms, nodes={}\nVectors: {} total -> {} unique Arcs (heap {:.2} MB -> {:.3} MB, {:.1}x)\nTexts:   {} total -> {} unique Arcs ({:.1}x)",
        size_of::<Node>(),
        size_of::<NodeKind>(),
        build_ms,
        doc.node_count(),
        total_vecs,
        unique_vec_ptrs.len(),
        vec_heap_naive as f64 / 1_048_576.0,
        vec_heap_shared as f64 / 1_048_576.0,
        vec_heap_naive as f64 / vec_heap_shared.max(1) as f64,
        total_txts,
        unique_txt_ptrs.len(),
        total_txts as f64 / unique_txt_ptrs.len().max(1) as f64,
    );
}
