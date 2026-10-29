//! Finding layers without changing the selection, page or edit history (#996).

use figcraft_doc::*;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::Session;
use crate::components::tests::{ex, idv};

fn add(s: &mut Session, parent: NodeId, kind: NodeKind, name: &str) -> NodeId {
    s.doc_mut().unwrap().create(parent, kind, name, 0.0, 0.0, 100.0, 100.0).unwrap()
}

fn found(s: &mut Session, params: Value, ids: &[NodeId]) {
    assert_eq!(ex(s, "node.find", params), json!({"ids": ids, "total": ids.len()}));
}

#[test]
fn node_find_filters_are_literal_case_insensitive_and_anded() {
    let mut s = Session::new();
    let page = s.page().unwrap();
    let frame = add(&mut s, page, NodeKind::Frame, "CAFÉ [Card]");
    let title = add(&mut s, frame, NodeKind::Text(TextData::new("HÉLLO 世界")), "Café Title");
    let other = add(&mut s, frame, NodeKind::Text(TextData::new("Other")), "Title");
    let rect = add(&mut s, page, NodeKind::Rectangle, "HÉLLO 世界");
    found(&mut s, json!({"name": "CAFÉ"}), &[frame, title]);
    found(&mut s, json!({"name": "[cArD]"}), &[frame]);
    found(&mut s, json!({"type": "tExT"}), &[title, other]);
    found(&mut s, json!({"text": "héLLo 世"}), &[title]);
    found(&mut s, json!({"name": "title", "type": "TEXT", "text": "世界"}), &[title]);
    found(&mut s, json!({"name": "café", "type": "RECTANGLE"}), &[]);
    found(&mut s, json!({"text": "héllo", "name": "HÉLLO"}), &[]);
    found(&mut s, json!({"text": ""}), &[title, other]);
    found(&mut s, json!({"name": ""}), &[frame, title, other, rect]);
    found(&mut s, json!({"name": ".*"}), &[]);
}

#[test]
fn node_find_matches_a_greek_final_sigma_both_ways() {
    let mut s = Session::new();
    let page = s.page().unwrap();
    let os = add(&mut s, page, NodeKind::Text(TextData::new("ΛΟΓΟΣ")), "ΟΣ");
    found(&mut s, json!({"name": "Σ"}), &[os]);
    found(&mut s, json!({"name": "ς"}), &[os]);
    found(&mut s, json!({"text": "λογοσ"}), &[os]);
}

#[test]
fn node_find_scope_is_preorder_strict_descendants_in_stored_order() {
    let mut s = Session::new();
    let page = s.page().unwrap();
    let root = add(&mut s, page, NodeKind::Frame, "Root");
    let first = add(&mut s, root, NodeKind::Frame, "First");
    let nested = add(&mut s, first, NodeKind::Rectangle, "Nested");
    let second = add(&mut s, root, NodeKind::Rectangle, "Second");
    let outside = add(&mut s, page, NodeKind::Rectangle, "Outside");
    s.doc_mut().unwrap().get_mut(first).unwrap().visible = false;
    s.doc_mut().unwrap().get_mut(second).unwrap().locked = true;
    found(&mut s, json!({}), &[root, first, nested, second, outside]);
    found(&mut s, json!({"root": root}), &[first, nested, second]);
    found(&mut s, json!({"root": root, "name": "root"}), &[]);
    found(&mut s, json!({"root": nested}), &[]);
    found(&mut s, json!({"root": page}), &[root, first, nested, second, outside]);
    s.doc_mut().unwrap().reorder(second, 0).unwrap();
    found(&mut s, json!({"root": root}), &[second, first, nested]);
}

#[test]
fn node_find_other_page_requires_root_and_does_not_switch_pages() {
    let mut s = Session::new();
    let page = s.page().unwrap();
    let here = add(&mut s, page, NodeKind::Frame, "Here");
    let other_page = s.doc_mut().unwrap().add_page("Other");
    let root = add(&mut s, other_page, NodeKind::Frame, "There");
    let child = add(&mut s, root, NodeKind::Rectangle, "Child");
    found(&mut s, json!({}), &[here]);
    found(&mut s, json!({"root": other_page}), &[root, child]);
    found(&mut s, json!({"root": root}), &[child]);
    assert_eq!(s.page().unwrap(), page);
}

#[test]
fn node_find_instances_match_main_name_and_visit_actual_children_only() {
    let mut s = Session::new();
    let page = s.page().unwrap();
    let main = add(&mut s, page, NodeKind::Component { description: String::new(), props: Default::default() }, "CAFÉ [Button]");
    let original = add(&mut s, main, NodeKind::Text(TextData::new("Original")), "Original");
    let inst = idv(&ex(&mut s, "instance.create", json!({"component": main})));
    let child = s.doc().unwrap().get(inst).unwrap().children[0];
    ex(&mut s, "node.set", json!({"ids": [inst], "props": {"name": "Copy"}}));
    ex(&mut s, "text.set", json!({"id": child, "text": "Changed"}));
    let broken = add(&mut s, page, NodeKind::Instance { component: NodeId(u64::MAX), props: Default::default() }, "Missing");
    found(&mut s, json!({"component": "café [bUt"}), &[inst]);
    found(&mut s, json!({"component": "", "type": "instance", "name": "copy"}), &[inst]);
    found(&mut s, json!({"component": "copy"}), &[]);
    found(&mut s, json!({"component": "button", "text": "Changed"}), &[]);
    found(&mut s, json!({"root": inst}), &[child]);
    found(&mut s, json!({"root": inst, "text": "original"}), &[]);
    found(&mut s, json!({"root": inst, "text": "changed"}), &[child]);
    found(&mut s, json!({}), &[main, original, inst, child, broken]);
}

#[test]
fn node_find_accepts_every_canonical_type() {
    let mut s = Session::new();
    let page = s.page().unwrap();
    let kinds = [
        NodeKind::Frame,
        NodeKind::Group,
        NodeKind::Section { contents_hidden: false },
        NodeKind::Rectangle,
        NodeKind::Ellipse { start: 0.0, sweep: 1.0, inner: 0.0 },
        NodeKind::Polygon { count: 3 },
        NodeKind::Star { count: 5, inner_ratio: 0.5 },
        NodeKind::Line,
        NodeKind::Vector { network: Default::default() },
        NodeKind::Text(TextData::new("Text")),
        NodeKind::BooleanOp { op: BoolOp::Union },
        NodeKind::Component { description: String::new(), props: Default::default() },
        NodeKind::ComponentSet { description: String::new() },
        NodeKind::Instance { component: NodeId(u64::MAX), props: Default::default() },
        NodeKind::Slice,
    ];
    for kind in kinds {
        let name = kind.type_name();
        let id = add(&mut s, page, kind, name);
        assert_eq!(ex(&mut s, "node.get", json!({"id": id}))["type"], name);
        found(&mut s, json!({"type": name.to_ascii_lowercase()}), &[id]);
    }
    found(&mut s, json!({"type": "PAGE"}), &[]);
}

#[test]
fn node_find_limit_caps_ids_not_total_and_zero_counts_only() {
    let mut s = Session::new();
    let page = s.page().unwrap();
    let ids: Vec<_> = (0..105).map(|_| add(&mut s, page, NodeKind::Rectangle, "Match")).collect();
    assert_eq!(ex(&mut s, "node.find", json!({})), json!({"ids": &ids[..100], "total": 105}));
    assert_eq!(ex(&mut s, "node.find", json!({"limit": 2})), json!({"ids": &ids[..2], "total": 105}));
    assert_eq!(ex(&mut s, "node.find", json!({"limit": 0})), json!({"ids": [], "total": 105}));
    found(&mut s, json!({"limit": 10000}), &ids);
    found(&mut s, json!({"name": "missing", "limit": 0}), &[]);
}

#[test]
fn node_find_rejects_bad_root_type_filters_and_limits() {
    let mut s = Session::new();
    for params in [
        json!({"root": 999999}),
        json!({"root": "nope"}),
        json!({"root": {"id": 1}}),
        json!({"type": "BOOLEANOP"}),
        json!({"type": "FRAME "}),
        json!({"type": ""}),
        json!({"type": 3}),
        json!({"name": []}),
        json!({"text": true}),
        json!({"component": {}}),
        json!({"limit": -1}),
        json!({"limit": 10001}),
        json!({"limit": 1.5}),
        json!({"limit": 1.0}),
        json!({"limit": "2"}),
        json!({"limit": true}),
        json!({"limit": []}),
        json!({"limit": 1e308}),
    ] {
        assert!(s.execute("node.find", &params).is_err(), "{params}");
    }
    let error = s.execute("node.find", &json!({"type": "x".repeat(10000)})).unwrap_err().to_string();
    assert!(error.contains("unknown type"), "{error}");
    assert!(error.len() < 300, "unknown types are not echoed whole");
}

#[test]
fn node_find_leaves_document_selection_page_and_history_untouched() {
    let mut s = Session::new();
    let layer = idv(&ex(&mut s, "shape.rectangle", json!({"width": 10, "height": 10})));
    ex(&mut s, "node.set", json!({"ids": [layer], "props": {"name": "Previous edit"}}));
    ex(&mut s, "node.set", json!({"ids": [layer], "props": {"name": "Undone edit"}}));
    ex(&mut s, "edit.undo", json!({}));
    ex(&mut s, "select.set", json!({"ids": [layer]}));
    let ds = s.ds().unwrap();
    let (doc, selection, page, undo, redo) = (ds.doc.clone(), ds.selection.clone(), ds.page, ds.history.undo.len(), ds.history.redo.len());
    assert!(undo > 0 && redo > 0);
    found(&mut s, json!({"name": "previous"}), &[layer]);
    found(&mut s, json!({"name": "missing"}), &[]);
    ex(&mut s, "node.find", json!({"limit": 0}));
    assert!(s.execute("node.find", &json!({"root": 999999})).is_err());
    let ds = s.ds().unwrap();
    assert!(Arc::ptr_eq(&ds.doc, &doc));
    assert_eq!((&ds.selection, ds.page, ds.history.undo.len(), ds.history.redo.len()), (&selection, page, undo, redo));
    assert_eq!(ex(&mut s, "edit.undo", json!({}))["undone"], true);
    assert_ne!(s.doc().unwrap().get(layer).unwrap().name, "Previous edit");
    assert_eq!(s.ds().unwrap().history.undo.len(), undo - 1);
}
