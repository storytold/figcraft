//! Dev Mode annotations (notes with pinned properties on layers) and the ready-for-dev list.

use crate::components::tests::{ex, idv, node};
use crate::{EngineError, Session};
use figcraft_doc::*;
use serde_json::json;

fn rect(s: &mut Session) -> NodeId {
    idv(&ex(s, "shape.rectangle", json!({"x": 10, "y": 20, "width": 120, "height": 40, "props": {"name": "Button", "fill": "#3366FF"}})))
}

#[test]
fn annotations_are_added_edited_and_removed_with_undo() {
    let mut s = Session::new();
    let r = rect(&mut s);
    let out = ex(&mut s, "annotation.add", json!({"id": r, "label": "Use the brand button", "properties": ["width", "Fill", "fill"]}));
    assert_eq!(out, json!({"id": r.0, "index": 0}));
    let a = &node(&s, r).annotations;
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].label, "Use the brand button");
    assert_eq!(a[0].properties, vec!["width".to_string(), "fill".to_string()], "canonical ids, each once");
    // The selection is the default layer; a second annotation with only a pinned property.
    ex(&mut s, "select.set", json!({"ids": [r]}));
    assert_eq!(ex(&mut s, "annotation.add", json!({"properties": ["height"]}))["index"], 1);
    ex(&mut s, "annotation.edit", json!({"id": r, "index": 1, "label": "Tap target"}));
    assert_eq!(node(&s, r).annotations[1], Annotation { label: "Tap target".into(), properties: vec!["height".into()], category: None });
    ex(&mut s, "annotation.edit", json!({"id": r, "index": 1, "properties": []}));
    assert!(node(&s, r).annotations[1].properties.is_empty());
    // Each change is one undo step.
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(node(&s, r).annotations[1].properties, vec!["height".to_string()]);
    ex(&mut s, "annotation.remove", json!({"id": r, "index": 0}));
    assert_eq!(node(&s, r).annotations.len(), 1);
    assert_eq!(node(&s, r).annotations[0].label, "Tap target");
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(node(&s, r).annotations.len(), 2);
    ex(&mut s, "annotation.remove", json!({"id": r}));
    assert!(node(&s, r).annotations.is_empty(), "no index: every annotation");
    // Refused: empty, unknown properties, bad indexes, pages; the document is unchanged.
    let before = s.doc().unwrap().clone();
    for (cmd, p) in [
        ("annotation.add", json!({"id": r})),
        ("annotation.add", json!({"id": r, "label": "   "})),
        ("annotation.add", json!({"id": r, "label": "x", "properties": ["colour"]})),
        ("annotation.add", json!({"id": r, "label": "x", "properties": "width"})),
        ("annotation.add", json!({"id": r, "label": 5})),
        ("annotation.add", json!({"id": s.page().unwrap(), "label": "page"})),
        ("annotation.edit", json!({"id": r, "index": 0, "label": "x"})),
        ("annotation.remove", json!({"id": r})),
        ("annotation.remove", json!({"id": 999_999})),
    ] {
        assert!(s.execute(cmd, &p).is_err(), "{cmd} {p} refused");
    }
    assert_eq!(*s.doc().unwrap(), before);
    ex(&mut s, "annotation.add", json!({"id": r, "label": "x"}));
    assert!(s.execute("annotation.edit", &json!({"id": r, "index": 0, "label": ""})).is_err(), "a note or a property stays");
    assert!(s.execute("annotation.edit", &json!({"id": r, "index": 0})).is_err(), "nothing to change");
    assert!(s.execute("annotation.edit", &json!({"id": r, "index": -1, "label": "y"})).is_err());
    assert!(s.execute("annotation.edit", &json!({"id": r, "index": 1, "label": "y"})).is_err());
    // Capped per layer.
    for k in 1..crate::annotations::MAX_PER_LAYER {
        ex(&mut s, "annotation.add", json!({"id": r, "label": format!("n{k}")}));
    }
    assert!(s.execute("annotation.add", &json!({"id": r, "label": "one too many"})).is_err());
}

#[test]
fn pinned_properties_show_their_current_values() {
    let mut s = Session::new();
    let r = rect(&mut s);
    ex(&mut s, "variable.createCollection", json!({"name": "Tokens"}));
    ex(&mut s, "variable.create", json!({"collection": "Tokens", "name": "Brand/Primary", "type": "color", "value": "#3366FF"}));
    ex(&mut s, "variable.bind", json!({"ids": [r], "field": "fill", "variable": "Brand/Primary"}));
    ex(&mut s, "annotation.add", json!({"id": r, "label": "Primary action", "properties": ["width", "fill", "gap", "fontSize"]}));
    let list = ex(&mut s, "annotation.list", json!({}))["annotations"].clone();
    assert_eq!(list.as_array().map(Vec::len), Some(1));
    let a = &list[0];
    assert_eq!(a["id"], r.0);
    assert_eq!(a["name"], "Button");
    assert_eq!(a["label"], "Primary action");
    let props = a["properties"].as_array().cloned().unwrap_or_default();
    assert_eq!(props[0], json!({"property": "width", "label": "Width", "value": "120"}));
    assert_eq!(props[1]["value"], "#3366FF");
    assert_eq!(props[1]["token"], "Brand/Primary", "the variable behind the fill");
    assert_eq!(props[1]["color"], "#3366FF");
    assert_eq!(props[2]["value"], json!(null), "no auto layout: no gap");
    assert_eq!(props[3]["value"], json!(null));
    // Values follow edits.
    ex(&mut s, "node.set", json!({"id": r, "props": {"width": 200}}));
    let list = ex(&mut s, "annotation.list", json!({"id": r}))["annotations"].clone();
    assert_eq!(list[0]["properties"][0]["value"], "200");
    // Text layers: typography.
    let t = idv(&ex(&mut s, "text.create", json!({"text": "Hi", "x": 0, "y": 100})));
    ex(&mut s, "node.set", json!({"id": t, "props": {"fontSize": 24}}));
    ex(&mut s, "annotation.add", json!({"id": t, "properties": ["fontSize", "textAlign"]}));
    let list = ex(&mut s, "annotation.list", json!({"id": t}))["annotations"].clone();
    assert_eq!(list[0]["properties"][0]["value"], "24");
    assert_eq!(list[0]["properties"][1]["value"], "left");
    // Every annotation on the page, in layer order; other pages, "all".
    assert_eq!(ex(&mut s, "annotation.list", json!({}))["annotations"].as_array().map(Vec::len), Some(2));
    ex(&mut s, "page.add", json!({"name": "Other"}));
    assert_eq!(ex(&mut s, "annotation.list", json!({}))["annotations"].as_array().map(Vec::len), Some(0));
    assert_eq!(ex(&mut s, "annotation.list", json!({"page": "all"}))["annotations"].as_array().map(Vec::len), Some(2));
    assert!(s.execute("annotation.list", &json!({"page": r})).is_err(), "not a page");
}

#[test]
fn annotations_are_saved_and_old_files_open() {
    let mut s = Session::new();
    let r = rect(&mut s);
    ex(&mut s, "annotation.add", json!({"id": r, "label": "Note", "properties": ["height"]}));
    let doc = s.doc().unwrap().clone();
    let json = serde_json::to_string(&doc).unwrap();
    assert!(json.contains("\"annotations\":[{\"label\":\"Note\",\"properties\":[\"height\"]}]"), "{json}");
    let back: Document = serde_json::from_str(&json).unwrap();
    assert_eq!(back.node(r).unwrap().annotations, doc.node(r).unwrap().annotations);
    // Layers without annotations leave the field out; files without it read as none.
    let plain = serde_json::to_value(Node::new(NodeId(7), NodeKind::Rectangle, "R", 1.0, 1.0)).unwrap();
    assert!(plain.get("annotations").is_none());
    let n: Node = serde_json::from_value(plain).unwrap();
    assert!(n.annotations.is_empty());
    // Unknown pinned ids from newer files are kept and skipped.
    let mut n2 = n.clone();
    n2.annotations.push(Annotation { label: String::new(), properties: vec!["future".into(), "width".into()], category: None });
    let a = n2.annotations[0].clone();
    let pinned = crate::annotations::pinned(&doc, r, &a);
    assert_eq!(pinned.len(), 1);
    assert_eq!(pinned[0].0.id, "width");
    // node.get shows them.
    assert_eq!(ex(&mut s, "node.get", json!({"id": r}))["annotations"], json!([{"label": "Note", "properties": ["height"]}]));
}

#[test]
fn instances_keep_their_own_annotations() {
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100, "name": "Card"})));
    let inner = idv(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50, "parent": f})));
    let c = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
    ex(&mut s, "annotation.add", json!({"id": c, "label": "On the main"}));
    ex(&mut s, "annotation.add", json!({"id": inner, "label": "Inner on the main"}));
    let i = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 200, "y": 0})));
    assert!(node(&s, i).annotations.is_empty(), "the main's annotations stay on the main");
    let copy = crate::components::tests::copy_of(&s, i, inner);
    assert!(node(&s, copy).annotations.is_empty());
    ex(&mut s, "annotation.add", json!({"id": i, "label": "On the instance"}));
    ex(&mut s, "annotation.add", json!({"id": copy, "label": "Inner on the instance"}));
    // A main edit rebuilds the instance: its own annotations stay.
    ex(&mut s, "node.set", json!({"id": inner, "props": {"fill": "#00ff00"}}));
    assert_eq!(node(&s, i).annotations[0].label, "On the instance");
    let copy = crate::components::tests::copy_of(&s, i, inner);
    assert_eq!(node(&s, copy).annotations[0].label, "Inner on the instance");
    assert_eq!(node(&s, c).annotations[0].label, "On the main");
}

#[test]
fn ready_for_dev_lists_frames_by_status() {
    let mut s = Session::new();
    let a = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100, "name": "Login"})));
    let b = idv(&ex(&mut s, "frame.create", json!({"x": 200, "y": 0, "width": 100, "height": 100, "name": "Signup"})));
    let sec = idv(&ex(&mut s, "node.create", json!({"type": "section", "x": 1000, "y": 0, "width": 400, "height": 300})));
    ex(&mut s, "node.set", json!({"id": sec, "props": {"name": "Onboarding"}}));
    ex(&mut s, "node.set", json!({"ids": [a, sec], "props": {"devStatus": "readyForDev"}}));
    ex(&mut s, "node.set", json!({"id": b, "props": {"devStatus": "completed"}}));
    let page = s.page().unwrap();
    let pg2 = idv(&ex(&mut s, "page.add", json!({"name": "Settings"})));
    ex(&mut s, "page.select", json!({"id": pg2}));
    let c = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100, "name": "Profile"})));
    ex(&mut s, "node.set", json!({"id": c, "props": {"devStatus": "readyForDev"}}));
    let names = |v: &serde_json::Value| -> Vec<String> {
        v["items"].as_array().map(|a| a.iter().filter_map(|i| i["name"].as_str().map(String::from)).collect()).unwrap_or_default()
    };
    let ready = ex(&mut s, "inspect.readyForDev", json!({}));
    assert_eq!(names(&ready), vec!["Login", "Onboarding", "Profile"], "every page, page and layer order");
    assert_eq!(ready["items"][0], json!({"id": a.0, "name": "Login", "page": page.0, "status": "readyForDev", "changed": false}));
    assert_eq!(names(&ex(&mut s, "inspect.readyForDev", json!({"status": "completed"}))), vec!["Signup"]);
    assert_eq!(names(&ex(&mut s, "inspect.readyForDev", json!({"status": "any", "page": page}))), vec!["Login", "Signup", "Onboarding"]);
    assert!(s.execute("inspect.readyForDev", &json!({"status": "none"})).is_err());
    assert!(s.execute("inspect.readyForDev", &json!({"page": a})).is_err(), "not a page");
    let doc = s.doc().unwrap();
    assert_eq!(crate::devinspect::dev_items(doc, Some(DevStatus::ReadyForDev), Some(pg2)).len(), 1);
}

#[test]
fn annotation_commands_survive_junk() {
    let junk = [
        json!({}),
        json!(null),
        json!({"id": "x", "index": "NaN", "label": [1], "properties": {"a": 1}}),
        json!({"index": 1e300, "properties": [null, 5, "width", {"x": 1}]}),
        json!({"ids": [-1, "0:0"], "page": {"x": 1}, "status": 7}),
        json!({"index": -0.5, "label": "\u{0}\u{2028}".repeat(4000)}),
    ];
    for cmd in ["annotation.add", "annotation.edit", "annotation.remove", "annotation.list", "inspect.readyForDev", "inspect.componentCode"] {
        for j in &junk {
            let mut s = Session::new();
            let r = rect(&mut s);
            ex(&mut s, "annotation.add", json!({"id": r, "label": "n", "properties": ["width"]}));
            ex(&mut s, "select.set", json!({"ids": [r]}));
            if let Err(EngineError::Internal { cmd, msg }) = s.execute(cmd, j) {
                panic!("{cmd} panicked on {j}: {msg}");
            }
        }
    }
}
