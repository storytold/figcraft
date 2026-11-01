//! The Dash cap through the session: `node.set`, the saved file, inspection and copied properties.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use figcraft_doc::{NodeId, StrokeCap};

/// A 200-long vector with a dashed 10 px black stroke.
fn line(s: &mut Session) -> NodeId {
    let v = id_of(&s.execute("vector.create", &json!({"path": "M0 0 L200 0", "x": 0, "y": 0})).unwrap()["id"]).unwrap();
    s.execute("node.set", &json!({"id": v, "props": {"strokes": ["#000000"], "strokeWeight": 10, "strokeAlign": "center", "dashes": [20, 20]}}))
        .unwrap();
    v
}

fn set(s: &mut Session, id: NodeId, cap: Value) -> crate::Result<Value> {
    s.execute("node.set", &json!({"id": id, "props": {"dashCap": cap}}))
}

fn cap(s: &Session, id: NodeId) -> Option<StrokeCap> {
    s.doc().unwrap().node(id).unwrap().stroke.dash_cap
}

#[test]
fn node_set_takes_the_three_caps_and_null_clears_it() {
    let mut s = Session::new();
    let l = line(&mut s);
    assert_eq!(cap(&s, l), None, "unset by default");
    for (name, want) in [("round", StrokeCap::Round), ("square", StrokeCap::Square), ("none", StrokeCap::None)] {
        set(&mut s, l, json!(name)).unwrap();
        assert_eq!(cap(&s, l), Some(want));
    }
    for junk in [json!("lineArrow"), json!("triangleArrow"), json!("wobbly"), json!(3)] {
        assert!(set(&mut s, l, junk).is_err());
    }
    assert_eq!(cap(&s, l), Some(StrokeCap::None), "a rejected value changes nothing");
    set(&mut s, l, Value::Null).unwrap();
    assert_eq!(cap(&s, l), None);
}

#[test]
fn it_saves_only_when_set_and_older_files_open_unset() {
    let mut s = Session::new();
    let l = line(&mut s);
    let bytes = |s: &Session| figcraft_format::save_bytes(s.doc().unwrap(), None).unwrap();
    let unset = figcraft_format::to_json(s.doc().unwrap()).unwrap().to_string();
    assert!(!unset.contains("dash_cap") && !unset.contains("dashCap"), "not written when unset");
    let (open, _) = figcraft_format::load_bytes(&bytes(&s)).unwrap();
    assert_eq!(open.node(l).unwrap().stroke.dash_cap, None);
    set(&mut s, l, json!("round")).unwrap();
    let (open, _) = figcraft_format::load_bytes(&bytes(&s)).unwrap();
    assert_eq!(open.node(l).unwrap().stroke.dash_cap, Some(StrokeCap::Round));
}

#[test]
fn inspect_shows_it_and_copied_properties_carry_it() {
    let mut s = Session::new();
    let (a, b) = (line(&mut s), line(&mut s));
    set(&mut s, a, json!("square")).unwrap();
    assert_eq!(s.execute("node.get", &json!({"id": a})).unwrap()["dashCap"], "square");
    assert!(s.execute("node.get", &json!({"id": b})).unwrap().get("dashCap").is_none(), "only when set");
    s.execute("edit.copyProperties", &json!({"id": a})).unwrap();
    s.execute("edit.pasteProperties", &json!({"ids": [b]})).unwrap();
    assert_eq!(cap(&s, b), Some(StrokeCap::Square));
    // Copying an unset stroke clears what it pastes over.
    set(&mut s, a, Value::Null).unwrap();
    s.execute("edit.copyProperties", &json!({"id": a})).unwrap();
    s.execute("edit.pasteProperties", &json!({"ids": [b]})).unwrap();
    assert_eq!(cap(&s, b), None);
}
