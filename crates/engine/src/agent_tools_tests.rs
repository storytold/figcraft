//! Commands for agents: node.createTree, inspect.outline, inspect.lint, text.measure and the copy
//! map of edit.duplicate.

use serde_json::{Value, json};

use crate::{EngineError, Session};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn id(v: &Value) -> crate::NodeId {
    crate::params::id_of(v).unwrap()
}

fn bad_msg(r: crate::Result<Value>) -> String {
    match r {
        Err(EngineError::BadParams { msg, .. }) => msg,
        other => panic!("expected BadParams, got {other:?}"),
    }
}

fn card_tree() -> Value {
    json!({"type": "frame", "name": "Card", "width": 320, "height": 40, "props": {"layout": "vertical", "gap": 12, "padding": 16, "sizingV": "hug"},
    "children": [
        {"type": "text", "name": "Title", "text": "Ada Lovelace"},
        {"type": "frame", "name": "Button", "height": 40, "props": {"layout": "horizontal", "sizingH": "fill"},
            "children": [{"type": "text", "name": "Label", "text": "Follow"}]},
        {"type": "text", "name": "Title", "text": "Second title"}
    ]})
}

#[test]
fn create_tree_builds_nested_layers_in_one_undo_step() {
    let mut s = Session::new();
    let undo_before = s.ds().unwrap().history.undo.len();
    let r = ex(&mut s, "node.createTree", json!({"tree": card_tree(), "x": 40, "y": 60}));
    let ids = r["ids"].as_object().unwrap();
    assert_eq!(ids.len(), 5, "{ids:?}");
    let card = id(&r["id"]);
    assert_eq!(id(&ids["Card"]), card);
    let doc = s.doc().unwrap();
    let button = id(&ids["Card/Button"]);
    assert_eq!(doc.node(id(&ids["Card/Button/Label"])).unwrap().parent, Some(button));
    assert_eq!(doc.node(button).unwrap().parent, Some(card));
    // A repeated name gets its own key.
    assert_eq!(doc.node(id(&ids["Card/Title #2"])).unwrap().parent, Some(card));
    assert_eq!(doc.node(card).unwrap().children.len(), 3);
    let c = doc.frame_relative(card).as_coeffs();
    assert_eq!((c[4], c[5]), (40.0, 60.0), "x/y apply to the root");
    assert_eq!(s.ds().unwrap().history.undo.len(), undo_before + 1, "one undo step");
    assert_eq!(s.selection(), vec![card]);
}

#[test]
fn create_tree_errors_name_the_layer_and_roll_back() {
    let mut s = Session::new();
    let page = s.page().unwrap();
    let before = s.doc().unwrap().node(page).unwrap().children.len();
    let bad_kid = json!({"type": "frame", "children": [{"type": "text", "text": "ok"}, {"type": "blob"}]});
    let msg = bad_msg(s.execute("node.createTree", &json!({"tree": bad_kid})));
    assert!(msg.contains("tree.children[1]") && msg.contains("blob"), "{msg}");
    assert_eq!(s.doc().unwrap().node(page).unwrap().children.len(), before, "nothing is left behind");
    assert!(bad_msg(s.execute("node.createTree", &json!({}))).contains("`tree`"));
    assert!(bad_msg(s.execute("node.createTree", &json!({"tree": {"type": "frame", "children": 3}}))).contains("must be an array"));
    let mut deep = json!({"type": "frame"});
    for _ in 0..40 {
        deep = json!({"type": "frame", "children": [deep]});
    }
    assert!(bad_msg(s.execute("node.createTree", &json!({"tree": deep}))).contains("deeper than"));
    assert_eq!(s.doc().unwrap().node(page).unwrap().children.len(), before);
}

#[test]
fn duplicate_returns_which_copy_each_layer_became() {
    let mut s = Session::new();
    let r = ex(&mut s, "node.createTree", json!({"tree": card_tree()}));
    let card = id(&r["id"]);
    let label = id(&r["ids"]["Card/Button/Label"]);
    let d = ex(&mut s, "edit.duplicate", json!({"ids": [card]}));
    let map = d["map"].as_object().unwrap();
    assert_eq!(map.len(), 5, "every layer of the subtree: {map:?}");
    let copy = id(&map[&card.to_string()]);
    assert_eq!(d["ids"], json!([copy]));
    let label_copy = id(&map[&label.to_string()]);
    let doc = s.doc().unwrap();
    assert_ne!(label_copy, label);
    assert_eq!(doc.node(label_copy).unwrap().name, "Label");
    assert!(doc.ancestors(label_copy).contains(&copy), "the copy of the label sits in the copy of the card");
}

#[test]
fn outline_lists_one_line_per_layer() {
    let mut s = Session::new();
    let r = ex(&mut s, "node.createTree", json!({"tree": card_tree()}));
    let o = ex(&mut s, "inspect.outline", json!({"id": r["id"]}));
    let text = o["outline"].as_str().unwrap();
    assert_eq!(o["count"], 5);
    assert_eq!(text.lines().count(), 5, "{text}");
    let first = text.lines().next().unwrap();
    assert!(first.contains("FRAME") && first.contains("“Card”") && first.contains("[vertical gap 12]"), "{first}");
    assert!(text.contains("    ") && text.contains("text “Follow”"), "children are indented under their parents:\n{text}");
    // Depth 0 says how many children it left out.
    let shallow = ex(&mut s, "inspect.outline", json!({"id": r["id"], "depth": 0}));
    assert!(shallow["outline"].as_str().unwrap().contains("… 3 children"), "{shallow}");
    assert!(s.execute("inspect.outline", &json!({"id": 999_999})).is_err());
}

#[test]
fn lint_finds_overflow_clipping_and_overlapping_text() {
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let frame = id(&ex(&mut s, "node.create", json!({"type": "frame", "name": "Box", "width": 200, "height": 100, "props": {"clip": true}}))["id"]);
    // A fixed-size text far too small for its words.
    let cramped = id(&ex(
        &mut s,
        "node.create",
        json!({"type": "text", "parent": frame, "x": 10, "y": 10, "width": 30, "height": 10, "text": "Zero downtime, even at four times the load",
            "props": {"textAutoResize": "none", "fontSize": 14}}),
    )["id"]);
    // A rectangle past the frame's right edge.
    let wide = id(&ex(&mut s, "node.create", json!({"type": "rectangle", "parent": frame, "x": 150, "y": 50, "width": 100, "height": 20}))["id"]);
    // Two texts on top of each other.
    let a = id(&ex(&mut s, "node.create", json!({"type": "text", "parent": frame, "x": 10, "y": 60, "text": "Overlapping"}))["id"]);
    let b = id(&ex(&mut s, "node.create", json!({"type": "text", "parent": frame, "x": 14, "y": 62, "text": "Overlapping"}))["id"]);
    let issues = ex(&mut s, "inspect.lint", json!({}))["issues"].as_array().unwrap().clone();
    let has = |rule: &str, who: crate::NodeId| issues.iter().any(|i| i["rule"] == rule && id(&i["id"]) == who);
    assert!(has("textOverflow", cramped), "{issues:#?}");
    assert!(has("clipped", wide), "{issues:#?}");
    assert!(issues.iter().any(|i| i["rule"] == "textOverlap" && [id(&i["id"]), id(&i["other"])] == [a, b]), "{issues:#?}");
    // Without clipping, the rectangle is drawn outside the frame instead.
    ex(&mut s, "node.set", json!({"ids": [frame], "props": {"clip": false}}));
    let issues = ex(&mut s, "inspect.lint", json!({"id": frame}))["issues"].as_array().unwrap().clone();
    assert!(issues.iter().any(|i| i["rule"] == "outsideParent" && id(&i["id"]) == wide), "{issues:#?}");
    // A tidy layout has nothing to report; hidden layers are not checked.
    let mut tidy = Session::new();
    let r = ex(&mut tidy, "node.createTree", json!({"tree": card_tree()}));
    assert_eq!(ex(&mut tidy, "inspect.lint", json!({"id": r["id"]}))["issues"], json!([]));
    ex(&mut s, "node.set", json!({"ids": [frame], "props": {"visible": false}}));
    assert_eq!(ex(&mut s, "inspect.lint", json!({}))["issues"], json!([]));
}

#[test]
fn measure_text_matches_the_box_a_text_layer_gets() {
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let words = "Engineering for what can't stop";
    let m = ex(&mut s, "text.measure", json!({"text": words, "fontSize": 24}));
    let t = id(&ex(&mut s, "node.create", json!({"type": "text", "text": words, "props": {"fontSize": 24}}))["id"]);
    let n = s.doc().unwrap().node(t).unwrap().clone();
    assert_eq!(m["lines"], 1);
    assert!((m["height"].as_f64().unwrap() - n.height).abs() < 0.01, "{m} vs {}", n.height);
    assert!((m["width"].as_f64().unwrap().ceil() - n.width).abs() <= 1.0, "{m} vs {}", n.width);
    // Wrapping at a width adds lines and height.
    let wrapped = ex(&mut s, "text.measure", json!({"text": words, "fontSize": 24, "width": 120}));
    assert!(wrapped["lines"].as_u64().unwrap() > 1 && wrapped["height"].as_f64().unwrap() > m["height"].as_f64().unwrap(), "{wrapped}");
    assert!(s.execute("text.measure", &json!({})).is_err());
    assert!(s.execute("text.measure", &json!({"text": "x", "width": -5})).is_err());
    assert!(s.execute("text.measure", &json!({"text": "x", "fontSize": "big"})).is_err());
}
