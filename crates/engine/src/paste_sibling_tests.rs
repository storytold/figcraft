//! Cmd/Ctrl+V with one layer inside a frame selected pastes next to it, in its parent (#2355).

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use figcraft_doc::NodeId;

fn id(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn sess() -> Session {
    Session::new()
}

fn rect(s: &mut Session, parent: Option<NodeId>, x: f64, y: f64) -> NodeId {
    let mut p = json!({"x": x, "y": y, "width": 40, "height": 20});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id(&s.execute("shape.rectangle", &p).unwrap())
}

fn kids(s: &Session, parent: NodeId) -> Vec<NodeId> {
    s.doc().unwrap().node(parent).unwrap().children.clone()
}

fn parent_of(s: &Session, n: NodeId) -> Option<NodeId> {
    s.doc().unwrap().node(n).unwrap().parent
}

fn paste(s: &mut Session) -> Vec<NodeId> {
    s.execute("edit.paste", &json!({})).unwrap()["ids"].as_array().unwrap().iter().filter_map(id_of).collect()
}

#[test]
fn paste_with_a_layer_in_an_auto_layout_frame_selected_goes_right_after_it() {
    let mut s = sess();
    let a = rect(&mut s, None, 0.0, 0.0);
    let b = rect(&mut s, None, 0.0, 30.0);
    let r = s.execute("layout.add", &json!({"ids": [a, b], "direction": "vertical"})).unwrap();
    let frame = id_of(&r["ids"][0]).unwrap();
    s.execute("node.set", &json!({"id": frame, "props": {"gap": 10}})).unwrap();
    let h0 = s.doc().unwrap().node(frame).unwrap().height;
    assert_eq!(h0, 50.0);
    assert_eq!(kids(&s, frame), vec![a, b]);
    s.set_selection(vec![b]);
    s.execute("edit.copy", &json!({})).unwrap();
    let pasted = paste(&mut s);
    assert_eq!(pasted.len(), 1);
    assert_eq!(parent_of(&s, pasted[0]), Some(frame), "the copy joins the selected layer's frame");
    assert_eq!(kids(&s, frame), vec![a, b, pasted[0]], "right after the selected layer in the flow");
    assert_eq!(s.doc().unwrap().node(frame).unwrap().height, h0 + 20.0 + 10.0, "the frame grew by the copy's height plus the gap");
    assert_eq!(s.selection(), pasted);
}

#[test]
fn paste_with_a_layer_in_a_plain_frame_selected_stays_in_the_frame_in_place() {
    let mut s = sess();
    let frame = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 200})).unwrap());
    let a = rect(&mut s, Some(frame), 10.0, 10.0);
    let b = rect(&mut s, Some(frame), 100.0, 100.0);
    s.set_selection(vec![a]);
    s.execute("edit.copy", &json!({})).unwrap();
    let pasted = paste(&mut s);
    assert_eq!(pasted.len(), 1);
    assert_eq!(parent_of(&s, pasted[0]), Some(frame));
    assert_eq!(kids(&s, frame), vec![a, pasted[0], b], "just above the selected layer");
    let wb = s.doc().unwrap().world_bounds(pasted[0]).unwrap();
    assert_eq!((wb.x0, wb.y0), (10.0, 10.0), "at the selected layer's position");
    assert_eq!(s.selection(), pasted);
}

#[test]
fn paste_with_a_top_level_layer_selected_is_unchanged() {
    let mut s = sess();
    let a = rect(&mut s, None, 10.0, 10.0);
    let page = parent_of(&s, a).unwrap();
    let before = kids(&s, page);
    s.set_selection(vec![a]);
    s.execute("edit.copy", &json!({})).unwrap();
    let pasted = paste(&mut s);
    assert_eq!(parent_of(&s, pasted[0]), Some(page));
    let mut want = before;
    want.push(pasted[0]);
    assert_eq!(kids(&s, page), want, "on the page, on top, as before");
}

#[test]
fn paste_with_two_layers_selected_is_unchanged() {
    let mut s = sess();
    let frame = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 200})).unwrap());
    let a = rect(&mut s, Some(frame), 10.0, 10.0);
    let b = rect(&mut s, Some(frame), 100.0, 100.0);
    let page = parent_of(&s, frame).unwrap();
    s.set_selection(vec![a, b]);
    s.execute("edit.copy", &json!({})).unwrap();
    let pasted = paste(&mut s);
    assert_eq!(pasted.len(), 2);
    assert!(pasted.iter().all(|p| parent_of(&s, *p) == Some(page)), "on the page, as before");
    assert_eq!(kids(&s, frame), vec![a, b]);
}

#[test]
fn paste_with_a_layer_inside_an_instance_selected_is_unchanged() {
    let mut s = sess();
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 50, "name": "Button"})).unwrap());
    s.execute("text.create", &json!({"text": "OK", "x": 10, "y": 10, "parent": f})).unwrap();
    let c = id(&s.execute("component.create", &json!({"ids": [f]})).unwrap());
    let inst = id(&s.execute("instance.create", &json!({"component": c, "x": 200, "y": 0})).unwrap());
    let page = parent_of(&s, inst).unwrap();
    let inner = kids(&s, inst)[0];
    let inner_kids = kids(&s, inst);
    let other = rect(&mut s, None, 500.0, 500.0);
    s.set_selection(vec![other]);
    s.execute("edit.copy", &json!({})).unwrap();
    s.set_selection(vec![inner]);
    let pasted = paste(&mut s);
    assert_eq!(parent_of(&s, pasted[0]), Some(page), "an instance's children are fixed: the copy goes to the page");
    assert_eq!(kids(&s, inst), inner_kids);
}
