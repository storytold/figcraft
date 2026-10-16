use serde_json::json;

use crate::{EngineError, Session};

fn id(value: &serde_json::Value) -> crate::NodeId {
    crate::params::id_of(&value["id"]).unwrap()
}

#[test]
fn later_steps_target_layers_created_by_earlier_steps() {
    let mut s = Session::new();
    let results = s
        .execute(
            "command.batch",
            &json!({
                "label": "Card",
                "commands": [
                    {"command": "node.create", "params": {"type": "frame", "x": 0, "y": 0, "width": 200, "height": 100}},
                    {"command": "node.create", "params": {"type": "rectangle", "parent": {"$step": 0}, "x": 10, "y": 10, "width": 20, "height": 20}},
                    {"command": "node.set", "params": {"ids": [{"$step": 1}], "props": {"name": "Swatch"}}}
                ]
            }),
        )
        .unwrap();

    let frame = id(&results[0]);
    let rect = id(&results[1]);
    let doc = s.doc().unwrap();
    assert_eq!(doc.node(rect).unwrap().parent, Some(frame));
    assert_eq!(doc.node(rect).unwrap().name, "Swatch");
    assert_eq!(s.ds().unwrap().history.undo.len(), 1, "still one undo step");
}

#[test]
fn field_picks_another_part_of_the_result() {
    let mut s = Session::new();
    let results = s
        .execute(
            "command.batch",
            &json!({"commands": [
                {"command": "node.create", "params": {"type": "frame", "width": 50, "height": 50}},
                {"command": "node.create", "params": {"type": "text", "text": "hi", "parent": {"$step": 0, "field": "id"}}}
            ]}),
        )
        .unwrap();
    assert_eq!(s.doc().unwrap().node(id(&results[1])).unwrap().parent, Some(id(&results[0])));
}

#[test]
fn bad_references_fail_and_roll_back() {
    let mut s = Session::new();
    let before = s.doc().unwrap().node(s.page().unwrap()).unwrap().children.len();
    let frame = json!({"command": "node.create", "params": {"type": "frame", "width": 10, "height": 10}});
    for (bad, needle) in [
        (json!({"$step": 1}), "earlier step"),
        (json!({"$step": 5}), "earlier step"),
        (json!({"$step": -1}), "step index"),
        (json!({"$step": "0"}), "step index"),
        (json!({"$step": 0, "field": "nope"}), "no `nope`"),
        (json!({"$step": 0, "field": 3}), "`field` must be a string"),
        (json!({"$step": 0, "extra": true}), "not `extra`"),
    ] {
        let r = s.execute("command.batch", &json!({"commands": [frame, {"command": "node.set", "params": {"ids": [bad], "props": {"name": "x"}}}]}));
        match r {
            Err(EngineError::BadParams { msg, .. }) => assert!(msg.contains(needle), "{msg} should mention {needle}"),
            other => panic!("expected BadParams for {bad}, got {other:?}"),
        }
        assert_eq!(s.doc().unwrap().node(s.page().unwrap()).unwrap().children.len(), before, "rolled back");
    }
}

#[test]
fn deep_params_are_refused_not_overflowed() {
    let mut s = Session::new();
    let mut deep = json!(1);
    for _ in 0..200 {
        deep = json!([deep]);
    }
    let r = s.execute("command.batch", &json!({"commands": [{"command": "select.none", "params": {"x": deep}}]}));
    assert!(matches!(r, Err(EngineError::BadParams { .. })));
}

#[test]
fn text_that_looks_like_a_reference_is_left_alone() {
    let mut s = Session::new();
    let results = s
        .execute("command.batch", &json!({"commands": [{"command": "node.create", "params": {"type": "text", "text": "$step 0 {\"$step\": 0}"}}]}))
        .unwrap();
    let n = s.doc().unwrap().node(id(&results[0])).unwrap().clone();
    assert!(matches!(n.kind, figcraft_doc::NodeKind::Text(t) if t.characters == "$step 0 {\"$step\": 0}"));
}

#[test]
fn a_nested_batch_refers_to_its_own_steps() {
    let mut s = Session::new();
    let frame = |name: &str| json!({"command": "node.create", "params": {"type": "frame", "name": name, "width": 100, "height": 100}});
    let results = s
        .execute(
            "command.batch",
            &json!({"commands": [
                frame("Outer"),
                {"command": "command.batch", "params": {"commands": [
                    frame("Inner"),
                    {"command": "node.create", "params": {"type": "rectangle", "parent": {"$step": 0}, "width": 10, "height": 10}}
                ]}}
            ]}),
        )
        .unwrap();
    let inner = &results[1];
    let rect = s.doc().unwrap().node(id(&inner[1])).unwrap().clone();
    assert_eq!(rect.parent, Some(id(&inner[0])), "in the nested batch's own first frame, not the outer one");
}
