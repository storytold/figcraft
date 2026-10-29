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

#[test]
fn a_failing_command_names_its_step_and_rolls_back() {
    let mut s = Session::new();
    let frame = id(&s.execute("node.create", &json!({"type": "frame", "width": 50, "height": 50})).unwrap());
    let text = id(&s.execute("node.create", &json!({"type": "text", "text": "orig"})).unwrap());
    let undo = s.ds().unwrap().history.undo.len();
    let name = s.doc().unwrap().node(s.page().unwrap()).unwrap().name.clone();
    let msg = batch_error(
        &mut s,
        json!([
            {"command": "text.set", "params": {"ids": [text.0], "text": "novo"}},
            {"command": "text.set", "params": {"ids": [frame.0], "text": "erro"}},
            {"command": "page.rename", "params": {"name": "Depois do erro"}}
        ]),
    );
    // Still the command's own kind of error, its message saying which step.
    assert!(msg.starts_with("step 1: ") && msg.contains("is not text"), "{msg}");
    assert!(matches!(&s.doc().unwrap().node(text).unwrap().kind, figcraft_doc::NodeKind::Text(t) if t.characters == "orig"));
    assert_eq!(s.doc().unwrap().node(s.page().unwrap()).unwrap().name, name);
    assert_eq!(s.ds().unwrap().history.undo.len(), undo);
}

#[test]
fn a_continue_batch_can_undo_then_redo_between_steps() {
    for mode in ["abort", "continue"] {
        let mut s = Session::new();
        let rect = id(&s.execute("node.create", &json!({"type": "rectangle"})).unwrap());
        let before = s.doc().unwrap().clone();
        let results =
            s.execute("command.batch", &json!({"onError": mode, "commands": [{"command": "edit.undo"}, {"command": "edit.redo"}]})).unwrap();
        let undo = if mode == "continue" { &results[0]["result"] } else { &results[0] };
        let redo = if mode == "continue" { &results[1]["result"] } else { &results[1] };
        assert_eq!(undo["undone"], true, "{mode}: {results}");
        assert_eq!(redo["redone"], true, "{mode}: {results}");
        assert!(s.doc().unwrap().contains(rect));
        assert_eq!(s.doc().unwrap(), &before);
    }
}

#[test]
fn a_continue_batch_can_open_a_document_after_closing_the_last_tab() {
    let mut s = Session::new();
    let uid = s.ds().unwrap().uid;
    let results = s
        .execute(
            "command.batch",
            &json!({"onError": "continue", "commands": [
                {"command": "file.close", "params": {"force": true}},
                {"command": "file.new"},
                {"command": "page.rename", "params": {"name": "New page"}}
            ]}),
        )
        .unwrap();
    assert!(results.as_array().unwrap().iter().all(|r| r["ok"] == true), "{results}");
    assert_eq!(s.docs.len(), 1);
    assert_ne!(s.ds().unwrap().uid, uid);
    assert_eq!(s.doc().unwrap().node(s.page().unwrap()).unwrap().name, "New page");
    assert!(s.closed_in_group.is_empty());
}

#[test]
fn a_continue_batch_repeats_the_last_successful_command_after_a_failed_step() {
    let mut s = Session::new();
    s.execute("node.create", &json!({"type": "rectangle"})).unwrap();
    let last = s.last_command.clone();
    let results = s
        .execute(
            "command.batch",
            &json!({"onError": "continue", "commands": [
                {"command": "command.batch", "params": {"commands": [
                    {"command": "node.create", "params": {"type": "text", "text": "Rolled back"}},
                    {"command": "no.such.command"}
                ]}},
                {"command": "edit.repeat"}
            ]}),
        )
        .unwrap();
    assert_eq!(results[0]["ok"], false);
    assert_eq!(results[1]["ok"], true);
    let repeated = s.doc().unwrap().node(id(&results[1]["result"])).unwrap();
    assert!(matches!(repeated.kind, figcraft_doc::NodeKind::Rectangle), "{repeated:?}");
    assert_eq!(s.last_command, last);
    assert_eq!(s.doc().unwrap().node(s.page().unwrap()).unwrap().children.len(), 2);
}

#[test]
fn a_continue_batch_keeps_successful_steps_as_one_undo_entry() {
    let mut s = Session::new();
    let frame = id(&s.execute("node.create", &json!({"type": "frame", "width": 50, "height": 50})).unwrap());
    let text = id(&s.execute("node.create", &json!({"type": "text", "text": "orig"})).unwrap());
    assert_eq!((frame.0, text.0), (2, 3));
    let undo = s.undo_depth().0;
    let name = s.doc().unwrap().node(s.page().unwrap()).unwrap().name.clone();
    let results = s
        .execute(
            "command.batch",
            &json!({"label": "Continue", "onError": "continue", "commands": [
                {"command": "text.set", "params": {"ids": [3], "text": "novo"}},
                {"command": "text.set", "params": {"ids": [2], "text": "erro"}},
                {"command": "page.rename", "params": {"name": "Depois do erro"}}
            ]}),
        )
        .unwrap();
    assert_eq!(results.as_array().unwrap().len(), 3);
    assert_eq!(results[0], json!({"step": 0, "command": "text.set", "ok": true, "result": {"ok": true}}));
    assert_eq!(results[2], json!({"step": 2, "command": "page.rename", "ok": true, "result": {"ok": true}}));
    assert_eq!(results[1]["step"], 1);
    assert_eq!(results[1]["command"], "text.set");
    assert_eq!(results[1]["ok"], false);
    let error = results[1]["error"].as_str().unwrap();
    assert!(error.contains("step 1:") && error.contains("is not text"), "{error}");
    assert!(results[1].get("result").is_none());
    undo_and_redo_the_continued_batch(&mut s, frame, text, &name, undo);
}

/// The successful steps of the batch above undo and redo as one step.
fn undo_and_redo_the_continued_batch(s: &mut Session, frame: figcraft_doc::NodeId, text: figcraft_doc::NodeId, name: &str, undo: usize) {
    assert!(matches!(&s.doc().unwrap().node(text).unwrap().kind, figcraft_doc::NodeKind::Text(t) if t.characters == "novo"));
    assert_eq!(s.doc().unwrap().node(s.page().unwrap()).unwrap().name, "Depois do erro");
    assert_eq!(s.undo_depth(), (undo + 1, 0));
    assert_eq!(s.undo_labels(1), ["Continue"]);
    assert!(s.undo().unwrap());
    assert!(matches!(&s.doc().unwrap().node(text).unwrap().kind, figcraft_doc::NodeKind::Text(t) if t.characters == "orig"));
    assert_eq!(s.doc().unwrap().node(s.page().unwrap()).unwrap().name, name);
    assert!(s.doc().unwrap().contains(frame), "undo leaves earlier edits alone");
    assert_eq!(s.undo_depth(), (undo, 1));
    assert!(s.redo().unwrap());
    assert!(matches!(&s.doc().unwrap().node(text).unwrap().kind, figcraft_doc::NodeKind::Text(t) if t.characters == "novo"));
    assert_eq!(s.doc().unwrap().node(s.page().unwrap()).unwrap().name, "Depois do erro");
    assert_eq!(s.undo_depth(), (undo + 1, 0));
}

#[test]
fn a_continue_batch_rolls_back_partial_steps_and_keeps_reference_positions() {
    let mut s = Session::new();
    let text = id(&s.execute("node.create", &json!({"type": "text", "text": "orig"})).unwrap());
    let frame = id(&s.execute("node.create", &json!({"type": "frame"})).unwrap());
    let results = s
        .execute(
            "command.batch",
            &json!({"onError": "continue", "commands": [
                {"command": "text.set", "as": "bad", "params": {"ids": [text.0, frame.0], "text": "partial"}},
                {"command": "node.create", "as": "card", "params": {"type": "frame"}},
                {"command": "node.create", "params": {"type": "text", "parent": {"$step": 0}}},
                {"command": "node.create", "params": {"type": "text", "parent": {"$ref": "bad"}}},
                {"command": "node.create", "params": {"type": "text", "parent": {"$step": 1}}},
                {"command": "node.create", "params": {"type": "text", "parent": {"$ref": "card"}}},
                {"command": "node.set", "params": {"ids": [{"$step": 4}], "props": {"name": "Child"}}}
            ]}),
        )
        .unwrap();
    assert_eq!(results.as_array().unwrap().len(), 7);
    for (i, result) in results.as_array().unwrap().iter().enumerate() {
        assert_eq!(result["step"], i);
        assert_eq!(result["ok"], ![0, 2, 3].contains(&i));
    }
    assert!(results[2]["error"].as_str().unwrap().contains("step 2: references step 0, which failed"));
    assert!(results[3]["error"].as_str().unwrap().contains("step 3: references step 0 (\"bad\"), which failed"));
    assert!(
        matches!(&s.doc().unwrap().node(text).unwrap().kind, figcraft_doc::NodeKind::Text(t) if t.characters == "orig"),
        "the failed step's partial edit was rolled back"
    );
    let card = id(&results[1]["result"]);
    for i in [4, 5] {
        assert_eq!(s.doc().unwrap().node(id(&results[i]["result"])).unwrap().parent, Some(card), "references use raw results, not outcomes");
    }
    assert_eq!(s.doc().unwrap().node(id(&results[4]["result"])).unwrap().name, "Child");
}

#[test]
fn invalid_batch_policies_run_nothing() {
    for mode in [json!("skip"), json!(""), json!(true), json!(3), json!(["continue"]), json!({})] {
        let mut s = Session::new();
        let before = s.doc().unwrap().clone();
        let journal = s.journal.clone();
        let error = s
            .execute("command.batch", &json!({"onError": mode, "commands": [{"command": "page.rename", "params": {"name": "Changed"}}]}))
            .unwrap_err();
        assert!(matches!(error, EngineError::BadParams { .. }));
        assert!(error.to_string().contains("`onError` must be \"abort\" or \"continue\""), "{error}");
        assert_eq!(s.doc().unwrap(), &before);
        assert_eq!(s.journal, journal);
        assert_eq!(s.undo_depth(), (0, 0));
    }
}

#[test]
fn invalid_batch_structure_runs_nothing_in_either_mode() {
    let rename = json!({"command": "page.rename", "params": {"name": "Changed"}});
    for mode in ["abort", "continue"] {
        for commands in [
            json!(null),
            json!({}),
            json!([rename, null]),
            json!([rename, {}]),
            json!([rename, {"command": 3}]),
            json!([rename, {"command": "page.rename", "as": ""}]),
            json!([rename, {"command": "page.rename", "as": 3}]),
            json!([{"command": "page.rename", "as": "same", "params": {"name": "Changed"}}, {"command": "select.none", "as": "same"}]),
        ] {
            let mut s = Session::new();
            let before = s.doc().unwrap().clone();
            let journal = s.journal.clone();
            let result = s.execute("command.batch", &json!({"onError": mode, "commands": commands}));
            assert!(matches!(result, Err(EngineError::BadParams { .. })), "{mode}: {result:?}");
            assert_eq!(s.doc().unwrap(), &before);
            assert_eq!(s.journal, journal);
            assert_eq!(s.undo_depth(), (0, 0));
        }
    }
}

#[test]
fn an_explicit_abort_batch_keeps_the_default_results_and_errors() {
    let commands = json!([
        {"command": "node.create", "params": {"type": "frame"}},
        {"command": "node.create", "params": {"type": "text", "text": "orig"}}
    ]);
    let mut default = Session::new();
    let mut abort = Session::new();
    let raw = default.execute("command.batch", &json!({"commands": commands})).unwrap();
    assert_eq!(abort.execute("command.batch", &json!({"onError": "abort", "commands": commands})).unwrap(), raw);
    assert!(raw[0].get("id").is_some() && raw[0].get("ok").is_none(), "still raw results");
    let commands = json!([
        {"command": "text.set", "params": {"ids": [3], "text": "novo"}},
        {"command": "text.set", "params": {"ids": [2], "text": "erro"}},
        {"command": "page.rename", "params": {"name": "Depois do erro"}}
    ]);
    let before = abort.doc().unwrap().clone();
    let undo = abort.undo_depth();
    let error = default.execute("command.batch", &json!({"commands": commands})).unwrap_err();
    let explicit = abort.execute("command.batch", &json!({"onError": "abort", "commands": commands})).unwrap_err();
    assert_eq!(explicit.to_string(), error.to_string());
    assert!(matches!(explicit, EngineError::BadParams { .. }));
    assert_eq!(abort.doc().unwrap(), &before);
    assert_eq!(abort.undo_depth(), undo);
}

#[test]
fn nested_batch_error_policies_are_local() {
    for outer in ["abort", "continue"] {
        let mut s = Session::new();
        let results = s
            .execute(
                "command.batch",
                &json!({"onError": outer, "commands": [
                    {"command": "command.batch", "as": "inner", "params": {"onError": "continue", "commands": [
                        {"command": "node.create", "as": "card", "params": {"type": "frame"}},
                        {"command": "no.such.command"},
                        {"command": "node.create", "params": {"type": "text", "parent": {"$ref": "card"}}}
                    ]}},
                    {"command": "node.set", "params": {"ids": [{"$ref": "inner", "field": "2/result/id"}], "props": {"name": "Inner child"}}}
                ]}),
            )
            .unwrap();
        let inner = if outer == "continue" { &results[0]["result"] } else { &results[0] };
        assert_eq!(inner[1]["ok"], false);
        let child = s.doc().unwrap().node(id(&inner[2]["result"])).unwrap();
        assert_eq!(child.name, "Inner child");
        assert_eq!(child.parent, Some(id(&inner[0]["result"])));
        assert_eq!(s.undo_depth(), (1, 0));
        assert!(s.undo().unwrap());
        assert_eq!(s.doc().unwrap().node(s.page().unwrap()).unwrap().children.len(), 0);
        assert!(s.redo().unwrap());
        assert!(s.doc().unwrap().contains(id(&inner[2]["result"])));
    }
}

#[test]
fn an_outer_continue_batch_rolls_back_a_failed_inner_abort_batch() {
    for explicit in [false, true] {
        let mut s = Session::new();
        let mut inner = json!({"commands": [
            {"command": "file.close", "params": {"force": true}},
            {"command": "file.new", "params": {"name": "Temporary"}},
            {"command": "node.create", "params": {"type": "frame"}},
            {"command": "no.such.command"}
        ]});
        if explicit {
            inner["onError"] = json!("abort");
        }
        let uid = s.ds().unwrap().uid;
        let results = s
            .execute(
                "command.batch",
                &json!({"onError": "continue", "commands": [
                    {"command": "page.rename", "params": {"name": "Before"}},
                    {"command": "command.batch", "params": inner},
                    {"command": "page.rename", "params": {"name": "After"}}
                ]}),
            )
            .unwrap();
        assert_eq!(results[1]["ok"], false);
        assert!(results[1]["error"].as_str().unwrap().contains("unknown command"));
        assert_eq!(results[2]["ok"], true);
        assert_eq!(s.docs.len(), 1);
        assert_eq!(s.ds().unwrap().uid, uid);
        assert_eq!(s.doc().unwrap().node(s.page().unwrap()).unwrap().name, "After");
        assert!(s.doc().unwrap().node(s.page().unwrap()).unwrap().children.is_empty());
        assert!(s.closed_in_group.is_empty());
        assert_eq!(s.undo_depth(), (1, 0));
    }
}

#[test]
fn an_outer_abort_batch_rolls_back_successes_of_an_inner_continue_batch() {
    let mut s = Session::new();
    let before = s.doc().unwrap().clone();
    let result = s.execute(
        "command.batch",
        &json!({"commands": [
            {"command": "command.batch", "params": {"onError": "continue", "commands": [
                {"command": "node.create", "params": {"type": "frame"}},
                {"command": "no.such.command"}
            ]}},
            {"command": "no.such.command"}
        ]}),
    );
    assert!(matches!(result, Err(EngineError::UnknownCommand(_))));
    assert_eq!(s.doc().unwrap(), &before);
    assert_eq!(s.undo_depth(), (0, 0));
}

#[test]
fn an_all_failed_continue_batch_adds_no_undo_and_keeps_redo() {
    let mut s = Session::new();
    let frame = id(&s.execute("node.create", &json!({"type": "frame"})).unwrap());
    s.execute("page.rename", &json!({"name": "Redo name"})).unwrap();
    assert!(s.undo().unwrap());
    let depth = s.undo_depth();
    let before = s.doc().unwrap().clone();
    let results = s
        .execute(
            "command.batch",
            &json!({"onError": "continue", "commands": [
                {"command": "node.set", "params": {"ids": [frame.0, 999999], "props": {"name": "Partial"}}},
                {"command": "no.such.command"},
                {"command": "node.create", "params": {"type": "text", "parent": {"$step": 0}}}
            ]}),
        )
        .unwrap();
    assert!(results.as_array().unwrap().iter().all(|r| r["ok"] == false));
    assert_eq!(s.doc().unwrap(), &before);
    assert_eq!(s.undo_depth(), depth);
    assert!(s.redo().unwrap());
    assert_eq!(s.doc().unwrap().node(s.page().unwrap()).unwrap().name, "Redo name");
}

#[test]
fn an_empty_continue_batch_returns_an_empty_array_without_undo() {
    let mut s = Session::new();
    assert_eq!(s.execute("command.batch", &json!({"onError": "continue", "commands": []})).unwrap(), json!([]));
    assert_eq!(s.undo_depth(), (0, 0));
}

#[test]
fn a_null_batch_result_is_not_a_failed_slot() {
    let mut scope = crate::BatchScope::default();
    scope.push(serde_json::Value::Null);
    scope.push_failed();
    let null = scope.resolve(&json!({"$step": 0})).unwrap_err().to_string();
    assert!(null.contains("step 0's result has no `id`") && !null.contains("which failed"), "{null}");
    let failed = scope.resolve(&json!({"$step": 1})).unwrap_err().to_string();
    assert!(failed.contains("step 2: references step 1, which failed"), "{failed}");
    assert_eq!(scope.next_step(), 2);
}

#[test]
fn a_continue_batch_reports_runtime_errors_and_runs_later_steps() {
    let mut s = Session::new();
    let results = s
        .execute(
            "command.batch",
            &json!({"onError": "continue", "commands": [
                {"command": "no.such.command"},
                {"command": "node.create", "params": {"type": "no-such-type"}},
                {"command": "node.create", "params": {"type": "text", "parent": {"$ref": "missing"}}},
                {"command": "node.create", "params": {"type": "text", "parent": {"$step": 99}}},
                {"command": "node.create", "params": {"type": "frame"}},
                {"command": "node.set", "params": {"ids": [{"$step": 4, "field": "missing"}], "props": {"name": "No"}}},
                {"command": "page.rename", "params": {"name": "Done"}}
            ]}),
        )
        .unwrap();
    assert_eq!(results.as_array().unwrap().len(), 7);
    for (i, needle) in [(0, "unknown command"), (1, "unknown type"), (2, "names no step"), (3, "earlier step"), (5, "has no `missing`")] {
        assert_eq!(results[i]["ok"], false);
        assert!(results[i]["error"].as_str().unwrap().contains(needle), "{}", results[i]);
    }
    assert_eq!(results[4]["ok"], true);
    assert_eq!(results[6]["ok"], true);
    assert_eq!(s.doc().unwrap().node(s.page().unwrap()).unwrap().name, "Done");
}
