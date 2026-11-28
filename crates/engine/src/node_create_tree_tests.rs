//! `node.create {children}` builds a layer with the layers inside it in one command (#2185): one
//! undo step, a result that mirrors the tree, and a refused child that names its place and leaves
//! nothing behind.

use serde_json::{Value, json};

use crate::{EngineError, Session};

fn id(v: &Value) -> crate::NodeId {
    crate::params::id_of(v).unwrap()
}

fn card() -> Value {
    json!({"type": "frame", "name": "Card", "width": 320, "props": {"layout": "vertical", "gap": 8, "padding": 16}, "children": [
        {"type": "text", "name": "Title", "text": "Title"},
        {"type": "frame", "name": "Button", "width": 120, "height": 40, "props": {"layout": "horizontal"}, "children": [
            {"type": "text", "name": "Label", "text": "Go"}
        ]}
    ]})
}

#[test]
fn a_layer_is_made_with_its_children_in_one_undo_step() {
    let mut s = Session::new();
    let undo = s.ds().unwrap().history.undo.len();
    let out = s.execute("node.create", &card()).unwrap();
    let (card, title, button, label) =
        (id(&out["id"]), id(&out["children"][0]["id"]), id(&out["children"][1]["id"]), id(&out["children"][1]["children"][0]["id"]));
    let doc = s.doc().unwrap();
    assert_eq!(doc.node(card).unwrap().children, [title, button], "in the order listed");
    assert_eq!(doc.node(button).unwrap().children, [label]);
    assert_eq!(doc.node(label).unwrap().name, "Label");
    assert!(doc.node(card).unwrap().height > doc.node(button).unwrap().height, "the card's layout holds its children");
    assert_eq!(s.ds().unwrap().selection, [card], "the layer asked for is selected, not its children");
    assert_eq!(s.ds().unwrap().history.undo.len(), undo + 1);
    s.undo().unwrap();
    assert!(!s.doc().unwrap().contains(card) && !s.doc().unwrap().contains(label));
}

#[test]
fn a_refused_child_names_its_place_and_nothing_is_made() {
    let mut s = Session::new();
    let before = s.doc().unwrap().nodes().count();
    let mut tree = card();
    tree["children"][1]["children"][0]["colour"] = json!("#f00");
    let Err(EngineError::BadParams { msg, .. }) = s.execute("node.create", &tree) else { panic!("refused") };
    assert!(msg.starts_with("children[1].children[0]: ") && msg.contains("colour"), "{msg}");
    tree["children"][1]["children"][0] = json!({"type": "text", "parent": 1});
    let Err(EngineError::BadParams { msg, .. }) = s.execute("node.create", &tree) else { panic!("refused") };
    assert!(msg.starts_with("children[1].children[0]: "), "{msg}");
    assert_eq!(s.doc().unwrap().nodes().count(), before, "rolled back whole");
}
