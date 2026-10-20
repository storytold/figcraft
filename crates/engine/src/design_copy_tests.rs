//! Copy for other design apps: the copied layers alone as a `.fig` canvas in the clipboard's HTML
//! (`edit.copyDesignLayers`), read back through the same paste other apps' copies take.

use base64::Engine as _;
use serde_json::json;

use super::*;
use crate::cmd::edit::{META_CLOSE, META_OPEN, design_clipboard_buffer};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

fn id(v: &Value) -> NodeId {
    params::id_of(&v["id"]).unwrap()
}

fn ids(v: &Value) -> Vec<NodeId> {
    v["ids"].as_array().unwrap().iter().map(|x| params::id_of(x).unwrap()).collect()
}

fn names(s: &Session, ids: &[NodeId]) -> Vec<String> {
    let doc = s.doc().unwrap();
    ids.iter().map(|i| doc.node(*i).unwrap().name.clone()).collect()
}

/// Copy `ids` for other apps; the HTML.
fn copy_html(s: &mut Session, ids: &[NodeId]) -> String {
    let r = s.execute("edit.copyDesignLayers", &json!({ "ids": ids })).unwrap();
    r["html"].as_str().unwrap().to_string()
}

/// Paste `html` into a new file; the pasted layers and the session.
fn paste_elsewhere(html: &str) -> (Session, Vec<NodeId>) {
    let mut s = Session::new();
    let r = s.execute("edit.pasteDesignLayers", &json!({ "html": html })).unwrap();
    let pasted = ids(&r);
    (s, pasted)
}

#[test]
fn copied_layers_paste_back_with_their_tree_and_place() {
    let mut s = Session::new();
    let card = id(&s.execute("frame.create", &json!({"x": 40, "y": 30, "width": 200, "height": 100, "name": "Card"})).unwrap());
    s.execute("node.create", &json!({"type": "rectangle", "x": 50, "y": 40, "width": 40, "height": 20, "parent": card, "name": "Badge"})).unwrap();
    let dot = id(&s.execute("node.create", &json!({"type": "ellipse", "x": 300, "y": 0, "width": 50, "height": 50, "name": "Dot"})).unwrap());
    let html = copy_html(&mut s, &[card, dot]);
    let (t, pasted) = paste_elsewhere(&html);
    assert_eq!(names(&t, &pasted), ["Card", "Dot"]);
    let doc = t.doc().unwrap();
    let c = doc.node(pasted[0]).unwrap();
    assert_eq!(names(&t, &c.children), ["Badge"]);
    assert_eq!((c.width, c.height), (200.0, 100.0));
    let b = doc.world_bounds(pasted[0]).unwrap();
    assert_eq!((b.x0, b.y0), (40.0, 30.0));
    // Copying for other apps is a copy too: our own paste repeats it.
    let r = s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!(names(&s, &ids(&r)), ["Card", "Dot"]);
}

#[test]
fn only_the_copied_layers_travel() {
    let mut s = Session::new();
    let keep = id(&s.execute("node.create", &json!({"type": "rectangle", "x": 0, "y": 0, "width": 10, "height": 10, "name": "Keep"})).unwrap());
    s.execute("node.create", &json!({"type": "rectangle", "x": 20, "y": 0, "width": 10, "height": 10, "name": "Sibling"})).unwrap();
    let other = id(&s.execute("page.add", &json!({"name": "Other"})).unwrap());
    s.execute("node.create", &json!({"type": "ellipse", "x": 0, "y": 0, "width": 10, "height": 10, "parent": other, "name": "Elsewhere"})).unwrap();
    s.execute("page.select", &json!({"index": 0})).unwrap();
    let html = copy_html(&mut s, &[keep]);
    let (t, pasted) = paste_elsewhere(&html);
    assert_eq!(names(&t, &pasted), ["Keep"]);
    // The source file is untouched.
    let doc = s.doc().unwrap();
    assert_eq!(doc.pages.len(), 2);
    assert!(doc.nodes().any(|n| n.name == "Sibling"));
}

#[test]
fn a_nested_layer_comes_out_where_it_was_on_the_page() {
    let mut s = Session::new();
    let frame = id(&s.execute("frame.create", &json!({"x": 100, "y": 200, "width": 300, "height": 300})).unwrap());
    let inner = id(&s
        .execute("node.create", &json!({"type": "rectangle", "x": 120, "y": 230, "width": 30, "height": 30, "parent": frame, "name": "Inner"}))
        .unwrap());
    let was = s.doc().unwrap().world_bounds(inner).unwrap();
    let html = copy_html(&mut s, &[inner]);
    let (t, pasted) = paste_elsewhere(&html);
    assert_eq!(names(&t, &pasted), ["Inner"]);
    let b = t.doc().unwrap().world_bounds(pasted[0]).unwrap();
    assert_eq!((b.x0, b.y0), (was.x0, was.y0));
    assert_eq!((b.x0, b.y0), (220.0, 430.0));
}

#[test]
fn images_travel_inside_the_clipboard_data() {
    let mut s = Session::new();
    let png = figcraft_render::encode_png(&figcraft_render::Pixmap::new(2, 2).unwrap()).unwrap();
    let photo = id(&s.execute("file.placeImage", &json!({"data": B64.encode(png), "x": 0, "y": 0})).unwrap());
    let html = copy_html(&mut s, &[photo]);
    let (t, pasted) = paste_elsewhere(&html);
    let doc = t.doc().unwrap();
    let n = doc.node(pasted[0]).unwrap();
    let key = n
        .fills
        .iter()
        .find_map(|p| match &p.kind {
            figcraft_color::PaintKind::Image(img) => Some(img.image.clone()),
            _ => None,
        })
        .expect("an image fill");
    assert!(doc.images.contains_key(&key), "the image's bytes came along");
}

#[test]
fn instances_stay_instances_with_their_main_and_become_frames_without() {
    let mut s = Session::new();
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 80, "height": 40, "name": "Button"})).unwrap());
    let main = id(&s.execute("component.create", &json!({"ids": [f]})).unwrap());
    let inst = id(&s.execute("instance.create", &json!({"component": main, "x": 200, "y": 0})).unwrap());
    let is_instance = |t: &Session, i: NodeId| matches!(t.doc().unwrap().node(i).unwrap().kind, figcraft_doc::NodeKind::Instance { .. });
    let (t, pasted) = paste_elsewhere(&copy_html(&mut s, &[main, inst]));
    assert_eq!(pasted.len(), 2);
    assert!(is_instance(&t, pasted[1]));
    let (t, pasted) = paste_elsewhere(&copy_html(&mut s, &[inst]));
    assert_eq!(pasted.len(), 1);
    assert!(!is_instance(&t, pasted[0]));
    assert_eq!(t.doc().unwrap().node(pasted[0]).unwrap().width, 80.0);
}

#[test]
fn the_html_says_it_holds_layers() {
    let mut s = Session::new();
    let r = id(&s.execute("node.create", &json!({"type": "rectangle", "x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
    let html = copy_html(&mut s, &[r]);
    let buffer = design_clipboard_buffer(&html).expect("design data");
    // A bare canvas, as the clipboard carries it (no zip around it).
    assert!(buffer.starts_with(b"fig-kiwi"));
    let start = html.find(META_OPEN).unwrap() + META_OPEN.len();
    let end = html.find(META_CLOSE).unwrap();
    let meta: Value = serde_json::from_slice(&B64.decode(html.get(start..end).unwrap()).unwrap()).unwrap();
    assert_eq!(meta["dataType"], "scene");
    // The same copy gives the same HTML.
    assert_eq!(copy_html(&mut s, &[r]), html);
}

#[test]
fn nothing_to_copy_is_refused() {
    let mut s = Session::new();
    assert!(s.execute("edit.copyDesignLayers", &json!({})).is_err());
    let page = s.page().unwrap();
    assert!(s.execute("edit.copyDesignLayers", &json!({"ids": [page]})).is_err());
    assert!(crate::cmd::edit::design_clipboard_html(&s).is_none());
}
