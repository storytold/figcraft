//! Sections: hiding their contents, wrapping layers in a new one, "Changed" after ready for dev,
//! and frames in sections as prototype screens.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use figcraft_doc::NodeId;
use figcraft_geom::Point;

fn id(v: Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn section(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id(s.execute("node.create", &json!({"type": "section", "x": x, "y": y, "width": w, "height": h})).unwrap())
}

fn get(s: &mut Session, node: NodeId) -> Value {
    s.execute("node.get", &json!({"id": node})).unwrap()
}

/// The RGBA of `node` rendered at 1x, at (`x`, `y`) of its box.
fn pixel(s: &Session, node: NodeId, x: u32, y: u32) -> [u8; 4] {
    let pix = figcraft_render::render_node(s.doc().unwrap(), node, 1.0).unwrap();
    let i = ((y * pix.width() + x) * 4) as usize;
    let d = pix.data();
    [d[i], d[i + 1], d[i + 2], d[i + 3]]
}

#[test]
fn hidden_contents_skip_rendering_and_hit_testing() {
    let mut s = Session::new();
    let sec = section(&mut s, 0.0, 0.0, 200.0, 200.0);
    let r = id(s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 40, "height": 40, "parent": sec})).unwrap());
    s.execute("node.set", &json!({"id": r, "props": {"fills": ["#FF0000"]}})).unwrap();
    let page = s.page().unwrap();
    assert_eq!(s.doc().unwrap().hit_test(page, Point::new(60.0, 60.0), 0.0, None), Some(r));
    assert_eq!(pixel(&s, sec, 60, 60), [255, 0, 0, 255]);
    // Hiding: the selected child gives way to the section.
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    let v = s.execute("section.toggleContents", &json!({"ids": [sec]})).unwrap();
    assert_eq!(v["hidden"], json!(true));
    assert_eq!(s.selection(), vec![sec]);
    assert_eq!(get(&mut s, sec)["sectionContentsHidden"], json!(true));
    assert_eq!(s.doc().unwrap().hit_test(page, Point::new(60.0, 60.0), 0.0, None), Some(sec));
    assert_eq!(pixel(&s, sec, 60, 60), [255, 255, 255, 255], "the child isn't drawn");
    // Toggling again shows them; explicit `hidden` sets.
    s.execute("section.toggleContents", &json!({"ids": [sec]})).unwrap();
    assert_eq!(pixel(&s, sec, 60, 60), [255, 0, 0, 255]);
    s.execute("section.toggleContents", &json!({"ids": [sec], "hidden": true})).unwrap();
    s.execute("section.toggleContents", &json!({"ids": [sec], "hidden": true})).unwrap();
    assert!(s.doc().unwrap().node(sec).unwrap().hides_contents());
    s.undo().unwrap();
    s.undo().unwrap();
    assert!(!s.doc().unwrap().node(sec).unwrap().hides_contents());
    // Only sections.
    assert!(s.execute("section.toggleContents", &json!({"ids": [r]})).is_err());
}

#[test]
fn hiding_nested_sections_selects_the_outermost_target() {
    let mut s = Session::new();
    let outer = section(&mut s, 0.0, 0.0, 200.0, 200.0);
    let inner = section(&mut s, 0.0, 0.0, 100.0, 100.0);
    s.execute("node.reparent", &json!({"ids": [inner], "parent": outer})).unwrap();
    let child = id(s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 30, "height": 30, "parent": inner})).unwrap());

    s.execute("section.toggleContents", &json!({"ids": [outer], "hidden": true})).unwrap();
    s.execute("select.set", &json!({"ids": [child]})).unwrap();
    s.execute("section.toggleContents", &json!({"ids": [inner], "hidden": true})).unwrap();
    assert_eq!(s.selection(), vec![inner], "the outer section was outside this hide request");

    s.execute("section.toggleContents", &json!({"ids": [outer, inner], "hidden": false})).unwrap();
    s.execute("select.set", &json!({"ids": [child]})).unwrap();
    s.execute("section.toggleContents", &json!({"ids": [inner, outer], "hidden": true})).unwrap();
    assert_eq!(s.selection(), vec![outer]);
    assert!([inner, outer].iter().all(|id| s.doc().unwrap().node(*id).unwrap().hides_contents()));
}

#[test]
fn wrap_in_new_section() {
    let mut s = Session::new();
    let a = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    let b = id(s.execute("frame.create", &json!({"x": 200, "y": 50, "width": 100, "height": 100})).unwrap());
    let before = (s.doc().unwrap().world_bounds(a).unwrap(), s.doc().unwrap().world_bounds(b).unwrap());
    let sec = id(s.execute("object.wrapInSection", &json!({"ids": [a, b]})).unwrap());
    let doc = s.doc().unwrap();
    assert!(doc.node(sec).unwrap().kind == figcraft_doc::NodeKind::Section { contents_hidden: false });
    assert_eq!(doc.node(sec).unwrap().children, vec![a, b]);
    let sb = doc.world_bounds(sec).unwrap();
    assert_eq!((sb.x0, sb.y0, sb.x1, sb.y1), (-40.0, -40.0, 340.0, 190.0), "some room around");
    assert_eq!((doc.world_bounds(a).unwrap(), doc.world_bounds(b).unwrap()), before, "contents stay put");
    assert_eq!(s.selection(), vec![sec]);
    assert!(doc.node(sec).unwrap().strokes.len() == 1, "the section look");
    // Not from inside a frame.
    let r = id(s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 10, "height": 10, "parent": a})).unwrap());
    assert!(s.execute("object.wrapInSection", &json!({"ids": [r]})).is_err());
}

#[test]
fn ready_for_dev_turns_changed_until_resolved() {
    let mut s = Session::new();
    let sec = section(&mut s, 0.0, 0.0, 400.0, 400.0);
    let f = id(s.execute("frame.create", &json!({"x": 20, "y": 20, "width": 100, "height": 100, "parent": sec})).unwrap());
    let r = id(s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 10, "height": 10, "parent": f})).unwrap());
    let other = id(s.execute("frame.create", &json!({"x": 600, "y": 0, "width": 100, "height": 100})).unwrap());
    s.execute("node.set", &json!({"ids": [sec, f], "props": {"devStatus": "readyForDev"}})).unwrap();
    let changed = |s: &mut Session, n: NodeId| get(s, n).get("devChanged").is_some();
    assert!(!changed(&mut s, sec) && !changed(&mut s, f), "marking isn't a change");
    // Hiding contents and editing elsewhere aren't changes either.
    s.execute("section.toggleContents", &json!({"ids": [sec]})).unwrap();
    s.execute("section.toggleContents", &json!({"ids": [sec]})).unwrap();
    s.execute("node.set", &json!({"id": other, "props": {"opacity": 0.5}})).unwrap();
    assert!(!changed(&mut s, sec));
    // Editing a layer inside: both the frame and the section around it changed.
    s.execute("node.set", &json!({"id": r, "props": {"fills": ["#00FF00"]}})).unwrap();
    assert!(changed(&mut s, sec) && changed(&mut s, f));
    s.undo().unwrap();
    assert!(!changed(&mut s, sec), "undo takes the change back");
    s.redo().unwrap();
    // Resolving keeps the reason and returns to Ready for dev.
    s.execute("dev.resolveChange", &json!({"ids": [sec], "reason": "  New brand colour  "})).unwrap();
    let v = get(&mut s, sec);
    assert_eq!((v.get("devChanged"), v["devNote"].clone(), v["devStatus"].clone()), (None, json!("New brand colour"), json!("readyForDev")));
    assert!(s.execute("dev.resolveChange", &json!({"ids": [sec]})).is_err(), "nothing left to resolve");
    // Marking again also starts over; removing a layer is a change.
    s.execute("node.set", &json!({"id": f, "props": {"devStatus": "readyForDev"}})).unwrap();
    assert!(!changed(&mut s, f));
    s.execute("edit.delete", &json!({"ids": [r]})).unwrap();
    assert!(changed(&mut s, f) && changed(&mut s, sec));
    // Other statuses never show changes.
    s.execute("node.set", &json!({"id": f, "props": {"devStatus": "completed"}})).unwrap();
    assert!(!changed(&mut s, f));
}

#[test]
fn frames_in_sections_are_prototype_screens() {
    let mut s = Session::new();
    let page = s.page().unwrap();
    let sec = section(&mut s, 0.0, 0.0, 800.0, 400.0);
    let inner = section(&mut s, 0.0, 0.0, 300.0, 300.0);
    s.execute("node.reparent", &json!({"ids": [inner], "parent": sec})).unwrap();
    let a = id(s.execute("frame.create", &json!({"x": 400, "y": 20, "width": 100, "height": 100, "parent": sec})).unwrap());
    let b = id(s.execute("frame.create", &json!({"x": 20, "y": 20, "width": 100, "height": 100, "parent": inner})).unwrap());
    let child = id(s.execute("shape.rectangle", &json!({"x": 1, "y": 1, "width": 10, "height": 10, "parent": b})).unwrap());
    let loose = id(s.execute("frame.create", &json!({"x": 0, "y": 600, "width": 100, "height": 100})).unwrap());
    let doc = s.doc().unwrap();
    assert_eq!(doc.top_level_frame(child), Some(b));
    assert_eq!(doc.top_level_frame(sec), None);
    assert_eq!(doc.top_frames(page), vec![b, a, loose], "reading order, sections looked into");
    assert_eq!(doc.sections_in(page), vec![sec, inner]);
    // Flows can start there.
    let v = s.execute("proto.flow.add", &json!({"start": b})).unwrap();
    assert_eq!(v["start"], json!(b));
    // Navigating to a section shows its first frame, then the one shown last.
    let doc = s.doc().unwrap();
    let mut visited = HashMap::new();
    assert_eq!(crate::proto_run::navigate_target(doc, sec, &visited), Some(b));
    crate::proto_run::remember_sections(doc, a, &mut visited);
    assert_eq!(crate::proto_run::navigate_target(doc, sec, &visited), Some(a));
    assert_eq!(crate::proto_run::navigate_target(doc, inner, &visited), Some(b), "a was never shown in the inner section");
    assert_eq!(crate::proto_run::navigate_target(doc, child, &visited), Some(b), "a layer goes to its frame");
    let empty = {
        let e = section(&mut s, 0.0, 900.0, 50.0, 50.0);
        (e, s.doc().unwrap())
    };
    assert_eq!(crate::proto_run::navigate_target(empty.1, empty.0, &visited), None);
}
