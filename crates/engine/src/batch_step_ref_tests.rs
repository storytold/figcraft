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

/// Runs `commands` as a batch and returns the error message, which must be `BadParams`.
fn batch_error(s: &mut Session, commands: serde_json::Value) -> String {
    match s.execute("command.batch", &json!({"commands": commands})) {
        Err(EngineError::BadParams { msg, .. }) => msg,
        other => panic!("expected BadParams, got {other:?}"),
    }
}

#[test]
fn a_named_step_is_referred_to_by_its_name() {
    let mut s = Session::new();
    let results = s
        .execute(
            "command.batch",
            &json!({"commands": [
                {"command": "node.create", "as": "card", "params": {"type": "frame", "width": 200, "height": 100}},
                {"command": "node.create", "as": "swatch", "params": {"type": "rectangle", "parent": {"$ref": "card"}, "width": 20, "height": 20}},
                {"command": "node.set", "params": {"ids": [{"$ref": "swatch", "field": "id"}], "props": {"name": "Swatch"}}},
                {"command": "node.create", "params": {"type": "text", "text": "hi", "parent": {"$step": 0}}}
            ]}),
        )
        .unwrap();
    let (frame, rect, text) = (id(&results[0]), id(&results[1]), id(&results[3]));
    let doc = s.doc().unwrap();
    assert_eq!(doc.node(rect).unwrap().parent, Some(frame));
    assert_eq!(doc.node(rect).unwrap().name, "Swatch");
    assert_eq!(doc.node(text).unwrap().parent, Some(frame), "`$step` and `$ref` mix in one batch");
    assert_eq!(s.ds().unwrap().history.undo.len(), 1, "still one undo step");
}

/// `edit.duplicate` answers `{"ids": [...]}`: a slash path in `field` takes one of them.
#[test]
fn a_field_path_indexes_into_a_list_in_the_result() {
    let mut s = Session::new();
    let results = s
        .execute(
            "command.batch",
            &json!({"commands": [
                {"command": "node.create", "as": "a", "params": {"type": "frame", "name": "A", "width": 10, "height": 10}},
                {"command": "node.create", "as": "b", "params": {"type": "frame", "name": "B", "width": 10, "height": 10}},
                {"command": "edit.duplicate", "as": "copies", "params": {"ids": [{"$ref": "a"}, {"$ref": "b"}]}},
                {"command": "node.set", "params": {"ids": [{"$ref": "copies", "field": "ids/1"}], "props": {"name": "B copy"}}},
                {"command": "node.set", "params": {"ids": [{"$step": 2, "field": "/ids/0"}], "props": {"name": "A copy"}}}
            ]}),
        )
        .unwrap();
    let copies = results[2]["ids"].as_array().unwrap().clone();
    let doc = s.doc().unwrap();
    assert_eq!(doc.node(crate::params::id_of(&copies[0]).unwrap()).unwrap().name, "A copy", "a leading `/` works too");
    assert_eq!(doc.node(crate::params::id_of(&copies[1]).unwrap()).unwrap().name, "B copy");
}

#[test]
fn a_field_path_walks_nested_objects_and_takes_whole_values() {
    let mut scope = crate::BatchScope::default();
    scope.push(json!({"ids": [7, 8], "bounds": {"x": 1.5, "corners": [{"r": 4}]}, "a/b": "slash key"}));
    let resolved = scope
        .resolve(&json!({
            "x": {"$step": 0, "field": "bounds/x"},
            "r": {"$step": 0, "field": "bounds/corners/0/r"},
            "all": {"$step": 0, "field": "ids"},
            "key": {"$step": 0, "field": "a/b"},
            "pointer": {"$step": 0, "field": "/ids"}
        }))
        .unwrap();
    assert_eq!(resolved, json!({"x": 1.5, "r": 4, "all": [7, 8], "key": "slash key", "pointer": [7, 8]}), "a key holding `/` is still read as a key");
}

#[test]
fn bad_names_and_paths_fail_and_roll_back() {
    let mut s = Session::new();
    let before = s.doc().unwrap().node(s.page().unwrap()).unwrap().children.len();
    let frame = |name: &str| json!({"command": "node.create", "as": name, "params": {"type": "frame", "width": 10, "height": 10}});
    let dup = json!({"command": "edit.duplicate", "as": "copy", "params": {"ids": [{"$ref": "card"}]}});
    let set = |r: serde_json::Value| json!({"command": "node.set", "params": {"ids": [r], "props": {"name": "x"}}});
    let later = json!({"command": "node.create", "as": "later", "params": {"type": "frame", "width": 10, "height": 10}});
    for (commands, needle) in [
        (json!([frame("card"), frame("card")]), "step 1: `as` \"card\" is already step 0's name"),
        (json!([frame("card"), set(json!({"$ref": "nope"}))]), "step 1: `$ref` \"nope\" names no step (\"card\" is step 0)"),
        (json!([{"command": "node.create", "params": {"type": "frame"}}, set(json!({"$ref": "nope"}))]), "no step has an `as` name"),
        (json!([frame("card"), set(json!({"$ref": "later"})), later]), "step 1: `$ref` \"later\" is step 2, not an earlier step"),
        (
            json!([frame("card"), {"command": "node.set", "as": "self", "params": {"ids": [{"$ref": "self"}], "props": {}}}]),
            "is step 1, not an earlier step",
        ),
        (json!([frame("card"), set(json!({"$ref": 0}))]), "`$ref` must be a step name"),
        (json!([frame("card"), set(json!({"$ref": "card", "id": 3}))]), "a `$ref` reference takes only `field`, not `id`"),
        (json!([frame("card"), set(json!({"$ref": "card", "$step": 0}))]), "a `$step` reference takes only `field`, not `$ref`"),
        (json!([frame("card"), {"command": "node.create", "as": "", "params": {}}]), "step 1: `as` must be a name"),
        (json!([frame("card"), {"command": "node.create", "as": 3, "params": {}}]), "step 1: `as` must be a name"),
        (
            json!([frame("card"), dup.clone(), set(json!({"$ref": "copy", "field": "ids/1"}))]),
            "step 2: step 1 (\"copy\")'s result has no `ids/1`: `ids` has 1 item",
        ),
        (json!([frame("card"), dup.clone(), set(json!({"$ref": "copy", "field": "ids/first"}))]), "`ids` is a list, so `first` must be an index"),
        (json!([frame("card"), dup.clone(), set(json!({"$ref": "copy", "field": "ids/0/x"}))]), "has no `ids/0/x`: `ids/0` is a number"),
        (json!([frame("card"), dup.clone(), set(json!({"$ref": "copy", "field": "map/0"}))]), "step 1 (\"copy\")'s result has no `map/0`"),
        (json!([frame("card"), dup.clone(), set(json!({"$ref": "copy", "field": "ids.0"}))]), "has no `ids.0` (paths use `/`, as in `ids/0`)"),
        (json!([frame("card"), set(json!({"$step": 0, "field": "id/0"}))]), "has no `id/0`: `id` is a number"),
    ] {
        let msg = batch_error(&mut s, commands);
        assert!(msg.contains(needle), "{msg:?} should contain {needle:?}");
        assert_eq!(s.doc().unwrap().node(s.page().unwrap()).unwrap().children.len(), before, "rolled back");
    }
}

#[test]
fn text_that_looks_like_a_named_reference_is_left_alone() {
    let mut s = Session::new();
    let text = "{\"$ref\": \"card\"} $ref";
    let results = s.execute("command.batch", &json!({"commands": [{"command": "node.create", "params": {"type": "text", "text": text}}]})).unwrap();
    let n = s.doc().unwrap().node(id(&results[0])).unwrap().clone();
    assert!(matches!(n.kind, figcraft_doc::NodeKind::Text(t) if t.characters == text));
}

/// A nested batch has its own names: it can't see the outer batch's, may reuse them, and the
/// outer batch can't see its.
#[test]
fn a_nested_batch_has_its_own_names() {
    let mut s = Session::new();
    let frame = |name: &str| json!({"command": "node.create", "as": "frame", "params": {"type": "frame", "name": name, "width": 100, "height": 100}});
    let results = s
        .execute(
            "command.batch",
            &json!({"commands": [
                frame("Outer"),
                {"command": "command.batch", "as": "inner", "params": {"commands": [
                    frame("Inner"),
                    {"command": "node.create", "params": {"type": "rectangle", "parent": {"$ref": "frame"}, "width": 10, "height": 10}}
                ]}},
                {"command": "node.set", "params": {"ids": [{"$ref": "inner", "field": "1/id"}], "props": {"name": "In inner"}}}
            ]}),
        )
        .unwrap();
    let inner = &results[1];
    let rect = s.doc().unwrap().node(id(&inner[1])).unwrap().clone();
    assert_eq!(rect.parent, Some(id(&inner[0])), "in the nested batch's own frame, not the outer one");
    assert_eq!(rect.name, "In inner", "a path indexes into a nested batch's result list");

    let msg = batch_error(
        &mut s,
        json!([frame("Outer"), {"command": "command.batch", "params": {"commands": [
            {"command": "node.create", "params": {"type": "rectangle", "parent": {"$ref": "frame"}, "width": 10, "height": 10}}
        ]}}]),
    );
    assert!(msg.contains("`$ref` \"frame\" names no step"), "the outer names aren't visible inside: {msg}");
}
