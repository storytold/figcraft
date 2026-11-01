//! Page commands act on pages: a layer id (a rectangle, a frame) is refused before anything about
//! the layer changes, as `page.select` already does.

use figcraft_doc::{NodeId, NodeKind};
use serde_json::{Value, json};

use crate::Session;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn name(s: &Session, id: NodeId) -> String {
    s.doc().unwrap().get(id).unwrap().name.clone()
}

#[test]
fn page_commands_reject_a_rectangle() {
    let mut s = Session::new();
    let page = s.page().unwrap();
    let r = id_of(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})));
    let before = name(&s, r);
    assert!(s.execute("page.select", &json!({"id": r})).is_err());
    assert!(s.execute("page.rename", &json!({"id": r, "name": "Not a page"})).is_err(), "rename refuses a layer");
    assert_eq!(name(&s, r), before, "the layer keeps its name");
    assert!(s.execute("page.setBackground", &json!({"id": r, "color": "#123456"})).is_err(), "background refuses a layer");
    assert!(s.execute("page.delete", &json!({"id": r})).is_err(), "delete refuses a layer");
    assert!(s.doc().unwrap().contains(r), "the layer is still there");
    assert!(s.doc().unwrap().contains(page));
}

#[test]
fn page_delete_keeps_a_frame_and_its_child() {
    let mut s = Session::new();
    let f = id_of(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100})));
    let c = id_of(&ex(&mut s, "shape.rectangle", json!({"x": 5, "y": 5, "width": 10, "height": 10, "parent": f})));
    assert_eq!(s.doc().unwrap().get(c).unwrap().parent, Some(f));
    assert!(s.execute("page.delete", &json!({"id": f})).is_err());
    assert!(s.doc().unwrap().contains(f) && s.doc().unwrap().contains(c), "the frame and its child remain");
    assert!(s.execute("page.rename", &json!({"id": f, "name": "Not a page"})).is_err());
    assert_ne!(name(&s, f), "Not a page");
}

#[test]
fn page_commands_still_work_on_pages() {
    let mut s = Session::new();
    let first = s.page().unwrap();
    let second = id_of(&ex(&mut s, "page.add", json!({})));
    ex(&mut s, "page.rename", json!({"id": second, "name": "Flows"}));
    assert_eq!(name(&s, second), "Flows");
    ex(&mut s, "page.setBackground", json!({"id": second, "color": "#123456"}));
    match s.doc().unwrap().get(second).unwrap().kind {
        NodeKind::Page { background, .. } => assert_eq!(background.to_css_hex().to_uppercase(), "#123456"),
        _ => panic!("not a page"),
    }
    ex(&mut s, "page.delete", json!({"id": second}));
    assert!(!s.doc().unwrap().contains(second));
    assert!(s.execute("page.delete", &json!({"id": first})).is_err(), "the last ordinary page stays");
}

#[test]
fn page_duplicate_rejects_a_frame() {
    let mut s = Session::new();
    let f = id_of(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100})));
    let pages = s.doc().unwrap().pages.clone();
    let err = s.execute("page.duplicate", &json!({"id": f})).expect_err("duplicate refuses a layer");
    assert!(err.to_string().contains("is not a page"), "{err}");
    assert_eq!(s.doc().unwrap().pages, pages, "no page was added");
}
