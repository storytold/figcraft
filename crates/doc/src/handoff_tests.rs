//! Handoff snapshots: a layer's subtree kept, and a copy of the document showing it.

use crate::{Document, NodeKind};
use std::sync::Arc;

#[test]
fn a_snapshot_shows_the_layer_as_it_was() {
    let mut doc = Document::new("d");
    let page = doc.first_page().unwrap();
    let f = doc.create(page, NodeKind::Frame, "Screen", 0.0, 0.0, 100.0, 100.0).unwrap();
    let kept = doc.create(f, NodeKind::Rectangle, "Kept", 0.0, 0.0, 10.0, 10.0).unwrap();
    let gone = doc.create(f, NodeKind::Rectangle, "Gone", 20.0, 0.0, 10.0, 10.0).unwrap();
    let snap = doc.snapshot_subtree(f).unwrap();
    assert_eq!(snap.nodes.iter().map(|n| n.id).collect::<Vec<_>>(), vec![f, kept, gone], "pre-order, the layer first");
    assert!(Arc::ptr_eq(&snap.nodes[1], doc.node_arc(kept).unwrap()), "shared, not copied");
    assert!(doc.snapshot_subtree(crate::NodeId(999)).is_none());
    doc.dev_snapshots.insert(f, Arc::new(snap));
    // Later: one layer edited, one removed, one added.
    doc.get_mut(kept).unwrap().width = 50.0;
    doc.remove(gone).unwrap();
    let new = doc.create(f, NodeKind::Ellipse { start: 0.0, sweep: 1.0, inner: 0.0 }, "New", 0.0, 0.0, 5.0, 5.0).unwrap();
    let before = doc.with_dev_snapshot(f).unwrap();
    assert_eq!(before.node(kept).unwrap().width, 10.0);
    assert!(before.node(gone).is_some());
    assert!(before.node(new).is_none());
    assert_eq!(before.subtree(f), vec![f, kept, gone]);
    assert!(before.dev_snapshots.is_empty());
    assert!(doc.with_dev_snapshot(kept).is_none(), "no snapshot of that layer");
    // Saved compactly: only when there is one.
    let json = serde_json::to_value(&doc).unwrap();
    assert_eq!(json["dev_snapshots"][f.0.to_string()]["nodes"].as_array().map(Vec::len), Some(3));
    let back: Document = serde_json::from_value(json).unwrap();
    assert_eq!(back.dev_snapshots, doc.dev_snapshots);
    doc.dev_snapshots.clear();
    assert!(serde_json::to_value(&doc).unwrap().get("dev_snapshots").is_none());
}

#[test]
fn a_snapshot_with_a_layer_inside_itself_is_refused() {
    let mut doc = Document::new("d");
    let page = doc.first_page().unwrap();
    let f = doc.create(page, NodeKind::Frame, "Screen", 0.0, 0.0, 100.0, 100.0).unwrap();
    let mut snap = doc.snapshot_subtree(f).unwrap();
    // A damaged file: the historical frame lists itself as a child.
    let mut bad = (*snap.nodes[0]).clone();
    bad.children.push(f);
    snap.nodes[0] = Arc::new(bad);
    doc.dev_snapshots.insert(f, Arc::new(snap));
    assert!(doc.with_dev_snapshot(f).is_none());
}

#[test]
fn a_snapshot_with_missing_or_nonreciprocal_child_is_refused() {
    use crate::NodeId;
    let mut doc = Document::new("d");
    let page = doc.first_page().unwrap();
    let f = doc.create(page, NodeKind::Frame, "Screen", 0.0, 0.0, 100.0, 100.0).unwrap();
    let _c1 = doc.create(f, NodeKind::Rectangle, "R1", 0.0, 0.0, 10.0, 10.0).unwrap();
    let snap = doc.snapshot_subtree(f).unwrap();

    let mut missing = snap.clone();
    let mut bad_root = (*missing.nodes[0]).clone();
    bad_root.children.push(NodeId(99999));
    missing.nodes[0] = Arc::new(bad_root);
    doc.dev_snapshots.insert(f, Arc::new(missing));
    assert!(doc.with_dev_snapshot(f).is_none());

    let mut nonrecip = snap.clone();
    let mut bad_child = (*nonrecip.nodes[1]).clone();
    bad_child.parent = Some(NodeId(88888));
    nonrecip.nodes[1] = Arc::new(bad_child);
    doc.dev_snapshots.insert(f, Arc::new(nonrecip));
    assert!(doc.with_dev_snapshot(f).is_none());
}

#[test]
fn validate_is_linear_in_a_wide_parent() {
    let mut doc = Document::new("d");
    let page = doc.first_page().unwrap();
    for i in 0..5_000 {
        doc.create(page, NodeKind::Rectangle, "R", f64::from(i), 0.0, 1.0, 1.0).unwrap();
    }
    assert!(doc.validate().is_empty());
    let kid = doc.node(page).unwrap().children[0];
    doc.get_mut(page).unwrap().children.retain(|c| *c != kid);
    assert!(doc.validate().iter().any(|m| m.contains("not listed in parent")));
}
