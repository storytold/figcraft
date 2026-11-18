//! `data.fill`: a frame repeated per row with that row's texts, fills and visibility, in one undo
//! step, optionally exported.

use crate::Session;
use crate::components::tests::{ex, idv};
use figcraft_doc::{NodeId, NodeKind};
use serde_json::json;

/// A 1080×1350 "Post" with "Title" and "Price" texts and a "Badge" rectangle. Returns the frame.
fn post(s: &mut Session) -> NodeId {
    let f = idv(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 1080, "height": 1350, "name": "Post"})));
    ex(s, "text.create", json!({"text": "Title", "x": 80, "y": 80, "parent": f, "props": {"name": "Title"}}));
    ex(s, "text.create", json!({"text": "R$ 0", "x": 80, "y": 1200, "parent": f, "props": {"name": "Price"}}));
    ex(s, "shape.rectangle", json!({"x": 80, "y": 40, "width": 200, "height": 32, "parent": f, "props": {"name": "Badge", "fill": "#3DDC97"}}));
    f
}

fn text(s: &Session, frame: NodeId, name: &str) -> String {
    let doc = s.doc().unwrap();
    doc.subtree(frame)
        .into_iter()
        .filter_map(|i| doc.node(i))
        .find(|n| n.name == name)
        .and_then(|n| match &n.kind {
            NodeKind::Text(t) => Some(t.characters.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

#[test]
fn each_row_fills_a_copy_in_one_undo_step() {
    let mut s = Session::new();
    let f = post(&mut s);
    let r = ex(
        &mut s,
        "data.fill",
        json!({"id": f, "rows": [
            {"name": "Slide 1", "Title": "Site em 7 dias", "Price": 5500},
            {"name": "Slide 2", "Title": "WhatsApp com IA", "Badge": {"visible": false}},
            {"Title": "Agenda sozinha", "Badge": {"fill": "#7C5CFF"}, "Subtitle": "nope"}
        ]}),
    );
    let ids: Vec<NodeId> = r["ids"].as_array().unwrap().iter().map(|v| NodeId(v.as_u64().unwrap())).collect();
    assert_eq!(ids.len(), 3);
    assert_eq!(r["missing"], json!(["Subtitle"]));
    assert_eq!(text(&s, ids[0], "Title"), "Site em 7 dias");
    assert_eq!(text(&s, ids[0], "Price"), "5500");
    assert_eq!(text(&s, ids[2], "Title"), "Agenda sozinha");
    assert_eq!(text(&s, f, "Title"), "Title", "the source frame is untouched");
    let doc = s.doc().unwrap();
    assert_eq!(doc.node(ids[0]).unwrap().name, "Slide 1");
    assert_eq!(doc.node(ids[2]).unwrap().name, "Post 3");
    // Copies sit in a row to the right, 100 apart.
    let x = |i: NodeId| doc.node(i).unwrap().transform.as_coeffs()[4];
    assert_eq!((x(ids[0]), x(ids[1])), (1180.0, 2360.0));
    let badge = |i: NodeId| doc.subtree(i).into_iter().filter_map(|k| doc.node(k)).find(|n| n.name == "Badge").cloned().unwrap();
    assert!(!badge(ids[1]).visible && badge(ids[0]).visible);
    assert_eq!(badge(ids[2]).fills[0].solid_color().map(|c| c.to_hex()), Some("7C5CFF".into()));
    // One undo removes every copy.
    ex(&mut s, "edit.undo", json!({}));
    assert!(ids.iter().all(|i| !s.doc().unwrap().contains(*i)));
}

#[test]
fn copies_export_to_a_folder_and_bad_input_is_refused() {
    let mut s = Session::new();
    let f = post(&mut s);
    let dir = std::env::temp_dir().join(format!("figcraft-datafill-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let r = ex(
        &mut s,
        "data.fill",
        json!({"id": f, "rows": [{"name": "Post/1: hero", "Title": "A"}, {"Title": "B"}], "dir": dir.to_string_lossy(), "scale": 0.25}),
    );
    let files: Vec<String> = r["files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    assert_eq!(files.len(), 2);
    assert!(files[0].ends_with("Post-1- hero.png"), "{files:?}");
    assert!(files.iter().all(|p| std::fs::metadata(p).is_ok_and(|m| m.len() > 0)));
    let _ = std::fs::remove_dir_all(&dir);
    for bad in [json!({"id": f}), json!({"id": f, "rows": [1, 2]}), json!({"id": f, "rows": [], "format": "gif"}), json!({"id": 999_999, "rows": []})]
    {
        assert!(s.execute("data.fill", &bad).is_err(), "{bad}");
    }
}

#[test]
fn each_row_comes_in_every_format() {
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 1080, "height": 1350, "name": "Post"})));
    ex(&mut s, "layout.add", json!({"ids": [f], "direction": "vertical"}));
    ex(&mut s, "node.set", json!({"id": f, "props": {"sizingV": "fixed", "padding": 80}}));
    let t = idv(&ex(&mut s, "text.create", json!({"text": "Título", "x": 0, "y": 0, "parent": f, "props": {"name": "Title", "fontSize": 96}})));
    ex(&mut s, "node.set", json!({"id": t, "props": {"sizingH": "fill", "textAutoResize": "height"}}));
    let r = ex(
        &mut s,
        "data.fill",
        json!({"id": f, "formats": [
        {"name": "feed", "width": 1080, "height": 1350}, {"name": "story", "width": 1080, "height": 1920}, {"width": 1080, "height": 1080}
    ], "rows": [{"name": "capa", "Title": "Site em 7 dias"}, {"name": "prova", "Title": "Lighthouse 100"}]}),
    );
    let ids: Vec<NodeId> = r["ids"].as_array().unwrap().iter().map(|v| NodeId(v.as_u64().unwrap())).collect();
    assert_eq!(ids.len(), 6);
    let doc = s.doc().unwrap();
    let names: Vec<String> = ids.iter().map(|i| doc.node(*i).unwrap().name.clone()).collect();
    assert_eq!(names, ["capa-feed", "capa-story", "capa-1080x1080", "prova-feed", "prova-story", "prova-1080x1080"]);
    let size = |i: NodeId| (doc.node(i).unwrap().width, doc.node(i).unwrap().height);
    assert_eq!(size(ids[1]), (1080.0, 1920.0));
    assert_eq!(size(ids[2]), (1080.0, 1080.0));
    assert_eq!(text(&s, ids[4], "Title"), "Lighthouse 100");
    // Formats of a row stack downwards; rows go right without overlapping.
    let at = |i: NodeId| (doc.node(i).unwrap().transform.as_coeffs()[4], doc.node(i).unwrap().transform.as_coeffs()[5]);
    assert_eq!(at(ids[0]).0, at(ids[1]).0);
    assert!(at(ids[1]).1 >= at(ids[0]).1 + 1350.0);
    assert!(at(ids[3]).0 >= at(ids[0]).0 + 1080.0);
    for bad in [json!([]), json!([{"width": 0, "height": 10}]), json!("feed")] {
        assert!(s.execute("data.fill", &json!({"id": f, "rows": [{}], "formats": bad})).is_err(), "{bad}");
    }
}

#[test]
fn review_export_names_are_unique_and_failures_are_reported() {
    let mut s = Session::new();
    let f = post(&mut s);
    let dir = std::env::temp_dir().join(format!("figcraft-datafill-names-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let r = ex(
        &mut s,
        "data.fill",
        json!({"id": f, "rows": [{"name": "Post"}, {"name": "post"}, {"name": "a/b"}, {"name": "a:b"}], "dir": dir.to_string_lossy(), "scale": 0.1}),
    );
    let files: Vec<String> = r["files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    let lower: std::collections::HashSet<String> = files.iter().map(|f| f.to_lowercase()).collect();
    assert_eq!(lower.len(), 4, "one file per copy: {files:?}");
    assert!(files.iter().all(|p| std::fs::metadata(p).is_ok()), "{files:?}");
    let _ = std::fs::remove_dir_all(&dir);
    // A folder that can't be written: the copies stay and every failure is reported.
    let r = ex(&mut s, "data.fill", json!({"id": f, "rows": [{"name": "x"}], "dir": "/nonexistent-figcraft-dir/out"}));
    assert_eq!(r["ids"].as_array().unwrap().len(), 1);
    assert_eq!(r["errors"].as_array().map(Vec::len), Some(1), "{r}");
    assert!(s.doc().unwrap().contains(NodeId(r["ids"][0].as_u64().unwrap())));
}
