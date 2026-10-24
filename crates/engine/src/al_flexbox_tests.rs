//! The flexbox rules Penpot files use and Figma has no name for: a reversed flow, a stretched
//! cross axis and a percentage grid track. Each test drives them through commands (the props, the
//! undo, the CSS and inspect output, the REST and `.fig` mappings) and checks the places the
//! layout gives the children.

use serde_json::{Value, json};

use crate::params::id_of;
use crate::{EngineError, Session};
use figcraft_doc::{CrossAlign, NodeId};

fn id(v: Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

/// A 100 × 100 frame of two 40 × 20 rectangles.
fn row(s: &mut Session, props: Value) -> (NodeId, Vec<NodeId>) {
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100, "props": props})).unwrap());
    let kids = (0..2).map(|_| id(s.execute("shape.rectangle", &json!({"parent": f, "x": 0, "y": 0, "width": 40, "height": 20})).unwrap())).collect();
    (f, kids)
}

/// Each child's `(x, y, w, h)` in the frame.
fn rects(s: &Session, kids: &[NodeId]) -> Vec<(f64, f64, f64, f64)> {
    kids.iter()
        .map(|k| {
            let n = s.doc().unwrap().node(*k).unwrap();
            let t = n.transform.as_coeffs();
            (t[4], t[5], n.width, n.height)
        })
        .collect()
}

#[test]
fn a_reversed_flow_lays_the_children_out_in_the_opposite_order() {
    let mut s = Session::new();
    let (f, kids) = row(&mut s, json!({"layout": "horizontal"}));
    assert_eq!(rects(&s, &kids), vec![(0.0, 0.0, 40.0, 20.0), (40.0, 0.0, 40.0, 20.0)]);
    s.execute("node.set", &json!({"id": f, "props": {"reverse": true}})).unwrap();
    assert_eq!(rects(&s, &kids), vec![(40.0, 0.0, 40.0, 20.0), (0.0, 0.0, 40.0, 20.0)]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(rects(&s, &kids), vec![(0.0, 0.0, 40.0, 20.0), (40.0, 0.0, 40.0, 20.0)]);
}

#[test]
fn the_reverse_prop_has_its_alias_and_a_junk_value_is_refused() {
    let mut s = Session::new();
    let (f, _) = row(&mut s, json!({"layout": "horizontal"}));
    s.execute("node.set", &json!({"id": f, "props": {"reverseDirection": true}})).unwrap();
    assert!(s.doc().unwrap().node(f).unwrap().layout.as_ref().unwrap().reverse);
    // A value that is not a boolean is refused, and the setting stays as it was.
    let r = s.execute("node.set", &json!({"id": f, "props": {"reverse": "sideways"}}));
    assert!(matches!(r, Err(EngineError::BadParams { .. })), "{r:?}");
    assert!(s.doc().unwrap().node(f).unwrap().layout.as_ref().unwrap().reverse);
}

#[test]
fn a_stretched_cross_axis_gives_the_children_the_line_size() {
    let mut s = Session::new();
    let (f, kids) = row(&mut s, json!({"layout": "horizontal"}));
    s.execute("node.set", &json!({"id": f, "props": {"crossAlign": "stretch"}})).unwrap();
    assert_eq!(s.doc().unwrap().node(f).unwrap().layout.as_ref().unwrap().cross_align, CrossAlign::Stretch);
    // The frame is 100 tall: the children take its whole height.
    assert_eq!(rects(&s, &kids), vec![(0.0, 0.0, 40.0, 100.0), (40.0, 0.0, 40.0, 100.0)]);
}

#[test]
fn css_and_inspect_name_the_new_settings() {
    let mut s = Session::new();
    let (f, _) = row(&mut s, json!({"layout": "horizontal"}));
    let css = |s: &mut Session| s.execute("inspect.code", &json!({"id": f})).unwrap().to_string();
    s.execute("node.set", &json!({"id": f, "props": {"reverse": true}})).unwrap();
    assert!(css(&mut s).contains("flex-direction: row-reverse;"), "{}", css(&mut s));
    s.execute("node.set", &json!({"id": f, "props": {"crossAlign": "stretch"}})).unwrap();
    assert!(css(&mut s).contains("align-items: stretch;"), "{}", css(&mut s));
    let rows = s.execute("inspect.properties", &json!({"id": f})).unwrap().to_string();
    assert!(rows.contains("stretch"), "{rows}");
}

#[test]
fn rest_and_fig_carry_the_reverse_setting() {
    let frame = |extra: Value| {
        let mut f = json!({"id": "1:2", "name": "Row", "type": "FRAME", "relativeTransform": [[1, 0, 0], [0, 1, 0]], "size": {"x": 100, "y": 100},
            "layoutMode": "HORIZONTAL", "children": []});
        f.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        json!({"name": "File", "document": {"id": "0:0", "type": "DOCUMENT", "children": [{"id": "0:1", "name": "Page", "type": "CANVAS", "children": [f]}]}})
    };
    let read = |file: &Value| figcraft_format::rest::import(file, &figcraft_format::rest::no_images).unwrap().0;
    let of = |doc: &figcraft_doc::Document| doc.nodes().find(|n| n.name == "Row").unwrap().layout.as_ref().unwrap().reverse;
    assert!(!of(&read(&frame(json!({})))));
    assert!(of(&read(&frame(json!({"layoutReverse": true})))));
    // Written to `.fig` and read back, the setting survives.
    let doc = read(&frame(json!({"layoutReverse": true})));
    let (bytes, _) = figcraft_format::fig::export(&doc, None).unwrap();
    let (back, _) = figcraft_format::fig::import(&bytes, "x").unwrap();
    assert!(of(&back));
}
