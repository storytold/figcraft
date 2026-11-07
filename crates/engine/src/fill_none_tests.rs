//! `props.fill` and `props.stroke` accept `"none"` (either case) and `null` as no paint, the
//! same as an empty `fills`/`strokes` list (#1036): an outline-only shape in one call.

use figcraft_doc::NodeId;
use serde_json::{Value, json};

use crate::Session;

fn id(v: Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn create_rect(s: &mut Session, fill: Value) -> NodeId {
    id(ex(
        s,
        "node.create",
        json!({
            "type": "rectangle", "x": 0, "y": 0, "width": 10, "height": 10,
            "props": {"fill": fill, "stroke": "#000000", "strokeWeight": 1}
        }),
    ))
}

#[test]
fn fill_none_or_null_makes_an_outline_only_shape_in_one_call() {
    for none in [json!("none"), json!("None"), json!("NONE"), json!(null)] {
        let mut s = Session::new();
        let r = create_rect(&mut s, none.clone());
        let n = s.doc().unwrap().node(r).unwrap().clone();
        assert!(n.fills.is_empty(), "{none:?}: fills not empty");
        assert_eq!(n.strokes.len(), 1, "{none:?}: stroke kept");
    }
}

#[test]
fn fill_none_clears_an_existing_fill_as_one_undo_step() {
    let mut s = Session::new();
    let r = create_rect(&mut s, json!("#FF0000"));
    assert_eq!(s.doc().unwrap().node(r).unwrap().fills.len(), 1);
    ex(&mut s, "node.set", json!({"id": r, "props": {"fill": "none"}}));
    assert!(s.doc().unwrap().node(r).unwrap().fills.is_empty());
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().unwrap().node(r).unwrap().fills.len(), 1);
}

#[test]
fn stroke_none_or_null_clears_the_strokes() {
    for none in [json!("none"), json!(null)] {
        let mut s = Session::new();
        let r = create_rect(&mut s, json!("#FF0000"));
        ex(&mut s, "node.set", json!({"id": r, "props": {"stroke": none}}));
        assert!(s.doc().unwrap().node(r).unwrap().strokes.is_empty(), "{none:?}");
        assert_eq!(s.doc().unwrap().node(r).unwrap().fills.len(), 1, "{none:?}");
    }
}

#[test]
fn text_with_fill_none_gets_no_automatic_colour() {
    let mut s = Session::new();
    let t = id(ex(
        &mut s,
        "node.create",
        json!({
            "type": "text", "x": 0, "y": 0, "text": "Hi", "props": {"fill": "none"}
        }),
    ));
    assert!(s.doc().unwrap().node(t).unwrap().fills.is_empty());
}

#[test]
fn an_unknown_string_still_names_the_key() {
    let mut s = Session::new();
    let e = s
        .execute(
            "node.create",
            &json!({
                "type": "rectangle", "x": 0, "y": 0, "width": 10, "height": 10,
                "props": {"fill": "nope"}
            }),
        )
        .unwrap_err();
    let msg = e.to_string();
    assert!(msg.contains("`fill`") && msg.contains("none"), "{msg}");
}
