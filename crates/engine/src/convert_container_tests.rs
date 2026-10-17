//! Frames, groups and sections converted into each other in place: the layers stay where they are on
//! the page, what only a frame has goes with it, what can't convert is refused, one undo step.

use serde_json::{Value, json};

use crate::params::id_of;
use crate::{EngineError, Session};
use figcraft_doc::{NodeId, NodeKind, Sizing};

fn id(v: Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn rect(s: &mut Session, parent: NodeId, x: f64, y: f64) -> NodeId {
    id(ex(s, "shape.rectangle", json!({"parent": parent, "x": x, "y": y, "width": 40, "height": 20})))
}

/// A layer's box on the page: `[x0, y0, x1, y1]`, rounded.
fn page_box(s: &Session, n: NodeId) -> [f64; 4] {
    let b = s.doc().unwrap().world_bounds(n).unwrap();
    [b.x0, b.y0, b.x1, b.y1].map(|v| (v * 1000.0).round() / 1000.0)
}

fn kind(s: &Session, n: NodeId) -> NodeKind {
    s.doc().unwrap().node(n).unwrap().kind.clone()
}

#[test]
fn a_frame_becomes_a_group_around_its_layers() {
    let mut s = Session::new();
    let f = id(ex(
        &mut s,
        "frame.create",
        json!({"x": 100, "y": 50, "width": 300, "height": 200, "name": "Card", "props": {"fill": "#FF0000", "cornerRadius": 8}}),
    ));
    let (a, b) = (rect(&mut s, f, 20.0, 30.0), rect(&mut s, f, 120.0, 90.0));
    ex(&mut s, "node.set", json!({"id": f, "props": {"opacity": 0.5}}));
    let before = [page_box(&s, a), page_box(&s, b)];
    let r = ex(&mut s, "object.convertToGroup", json!({"ids": [f]}));
    assert_eq!(r["ids"], json!([f]));
    let n = s.doc().unwrap().node(f).unwrap().clone();
    assert!(matches!(n.kind, NodeKind::Group));
    assert_eq!((n.name.as_str(), n.opacity, n.children.clone()), ("Card", 0.5, vec![a, b]), "the same layer: name, opacity, layers");
    assert!(n.fills.is_empty() && n.strokes.is_empty() && !n.clip && n.corners.uniform() == Some(0.0), "what only a frame has is gone");
    assert_eq!([page_box(&s, a), page_box(&s, b)], before, "the layers stay where they are");
    // A group's box is its layers': (120, 80) to (260, 160).
    assert_eq!(page_box(&s, f), [120.0, 80.0, 260.0, 160.0]);
    ex(&mut s, "edit.undo", json!({}));
    let n = s.doc().unwrap().node(f).unwrap();
    assert!(matches!(n.kind, NodeKind::Frame) && n.fills.len() == 1 && n.clip, "one undo step brings the frame back");
    assert_eq!(page_box(&s, f), [100.0, 50.0, 400.0, 250.0]);
}

#[test]
fn auto_layout_is_dropped_and_positions_kept() {
    let mut s = Session::new();
    let props = json!({"layout": "horizontal", "padding": 10, "gap": 10});
    let f = id(ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 100, "props": props})));
    let (a, b) = (rect(&mut s, f, 0.0, 0.0), rect(&mut s, f, 0.0, 0.0));
    ex(&mut s, "node.set", json!({"id": b, "props": {"sizingH": "fill", "sizingV": "fill"}}));
    // Laid out: a at (10, 10) 40 × 20, b fills (60, 10) to (190, 90).
    let before = [page_box(&s, a), page_box(&s, b)];
    assert_eq!(before, [[10.0, 10.0, 50.0, 30.0], [60.0, 10.0, 190.0, 90.0]]);
    ex(&mut s, "object.convertToGroup", json!({"ids": [f]}));
    let doc = s.doc().unwrap();
    assert!(doc.node(f).unwrap().layout.is_none());
    assert_eq!([page_box(&s, a), page_box(&s, b)], before, "the layout's positions and sizes are kept");
    let cl = &doc.node(b).unwrap().child_layout;
    assert_eq!((cl.sizing_h, cl.sizing_v), (Sizing::Fixed, Sizing::Fixed), "nothing to fill in a group");
    // And back: a plain frame of the group's box, the layers still in place.
    ex(&mut s, "object.convertToFrame", json!({"ids": [f]}));
    let n = s.doc().unwrap().node(f).unwrap();
    assert!(matches!(n.kind, NodeKind::Frame) && n.layout.is_none() && !n.clip && n.fills.is_empty());
    assert_eq!((page_box(&s, f), [page_box(&s, a), page_box(&s, b)]), ([10.0, 10.0, 190.0, 90.0], before));
}

#[test]
fn a_rotated_group_becomes_a_frame_in_place() {
    let mut s = Session::new();
    let page = s.page().unwrap();
    let (a, b) = (rect(&mut s, page, 10.0, 10.0), rect(&mut s, page, 80.0, 60.0));
    let g = id(ex(&mut s, "object.group", json!({"ids": [a, b]})));
    ex(&mut s, "node.set", json!({"id": g, "props": {"rotation": 30}}));
    let before = [page_box(&s, a), page_box(&s, b), page_box(&s, g)];
    ex(&mut s, "object.convertToFrame", json!({"ids": [g]}));
    assert!(matches!(kind(&s, g), NodeKind::Frame));
    assert_eq!([page_box(&s, a), page_box(&s, b), page_box(&s, g)], before);
    // The layers now sit in the frame's space: a at its origin.
    let c = s.doc().unwrap().node(a).unwrap().transform.as_coeffs();
    assert!(c[4].abs() < 1e-6 && c[5].abs() < 1e-6, "{c:?}");
}

#[test]
fn what_cannot_convert_is_refused_and_nothing_changes() {
    let mut s = Session::new();
    let page = s.page().unwrap();
    let f = id(ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100})));
    let inner = id(ex(&mut s, "frame.create", json!({"x": 10, "y": 10, "width": 50, "height": 50, "parent": f})));
    rect(&mut s, inner, 0.0, 0.0);
    let comp = id(ex(&mut s, "component.create", json!({"ids": [f]})));
    let inst = id(ex(&mut s, "instance.create", json!({"component": comp, "x": 300, "y": 0})));
    let inst_inner = s.doc().unwrap().node(inst).unwrap().children[0];
    let empty = id(ex(&mut s, "frame.create", json!({"x": 0, "y": 300, "width": 50, "height": 50})));
    let loose = rect(&mut s, page, 500.0, 0.0);
    let ok_frame = id(ex(&mut s, "frame.create", json!({"x": 0, "y": 500, "width": 50, "height": 50})));
    rect(&mut s, ok_frame, 5.0, 5.0);
    // (layer, what the message says)
    for (layer, says) in [
        (comp, "main component"),
        (inst, "is an instance"),
        (inst_inner, "inside an instance"),
        (empty, "is empty"),
        (loose, "neither a frame nor a group"),
    ] {
        for cmd in ["object.convertToGroup", "object.convertToFrame"] {
            if layer == empty && cmd == "object.convertToFrame" {
                continue;
            }
            // Together with a frame that could convert: all or nothing.
            let e = s.execute(cmd, &json!({"ids": [ok_frame, layer]})).unwrap_err().to_string();
            assert!(e.contains(says), "{cmd} on {layer}: {e}");
            assert!(matches!(kind(&s, ok_frame), NodeKind::Frame), "{cmd}: nothing converts when one can't");
        }
    }
    // Already what was asked for: left alone, not an error.
    assert_eq!(ex(&mut s, "object.convertToFrame", json!({"ids": [ok_frame]}))["ids"], json!([]));
    // A frame inside a main component converts (its instances follow their main).
    ex(&mut s, "object.convertToGroup", json!({"ids": [inner]}));
    assert!(matches!(kind(&s, inner), NodeKind::Group));
}

#[test]
fn junk_parameters_are_refused_not_fatal() {
    let junk = [json!(null), json!(true), json!(-1), json!(1e308), json!("x"), json!([[1]]), json!({"a": 1}), json!([0, 999_999, "7"])];
    for cmd in ["object.convertToGroup", "object.convertToFrame", "object.convertToSection"] {
        for j in &junk {
            let mut s = Session::new();
            let f = id(ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 50, "height": 50})));
            rect(&mut s, f, 0.0, 0.0);
            for p in [json!({"ids": j}), json!({"id": j}), j.clone()] {
                let r = s.execute(cmd, &p);
                assert!(!matches!(r, Err(EngineError::Internal { .. })), "{cmd} {p}: {r:?}");
            }
        }
    }
}

#[test]
fn a_frame_on_the_page_becomes_a_section_and_back() {
    let mut s = Session::new();
    let f = id(ex(&mut s, "frame.create", json!({"x": 100, "y": 50, "width": 300, "height": 200, "name": "Flow", "props": {"fill": "#FFEEDD"}})));
    let (a, b) = (rect(&mut s, f, 20.0, 30.0), rect(&mut s, f, 400.0, 90.0));
    ex(&mut s, "layout.add", json!({"ids": [f], "direction": "horizontal"}));
    ex(&mut s, "node.set", json!({"id": f, "props": {"width": 300, "height": 200}}));
    let before = [page_box(&s, a), page_box(&s, b), page_box(&s, f)];
    assert_eq!(ex(&mut s, "object.convertToSection", json!({"ids": [f]}))["ids"], json!([f]));
    let n = s.doc().unwrap().node(f).unwrap().clone();
    assert!(matches!(n.kind, NodeKind::Section { contents_hidden: false }));
    assert_eq!((n.name.as_str(), n.children.clone(), n.fills.len()), ("Flow", vec![a, b], 1), "same name, layers and fill");
    assert!(n.layout.is_none() && !n.clip, "no auto layout or clipping on a section");
    assert_eq!([page_box(&s, a), page_box(&s, b), page_box(&s, f)], before, "nothing moves");
    // And back: a frame that keeps the fill and doesn't clip, so the look stays.
    assert_eq!(ex(&mut s, "object.convertToFrame", json!({"ids": [f]}))["ids"], json!([f]));
    let n = s.doc().unwrap().node(f).unwrap().clone();
    assert!(matches!(n.kind, NodeKind::Frame) && n.fills.len() == 1 && !n.clip);
    assert_eq!([page_box(&s, a), page_box(&s, b), page_box(&s, f)], before);
    // Each conversion is one undo step.
    ex(&mut s, "edit.undo", json!({}));
    assert!(matches!(kind(&s, f), NodeKind::Section { .. }));
    ex(&mut s, "edit.undo", json!({}));
    assert!(matches!(kind(&s, f), NodeKind::Frame));
    assert!(s.doc().unwrap().node(f).unwrap().layout.is_some(), "the frame's auto layout is back");
}

#[test]
fn only_frames_on_the_page_or_in_a_section_become_sections() {
    let mut s = Session::new();
    let outer = id(ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 300})));
    let inner = id(ex(&mut s, "frame.create", json!({"parent": outer, "x": 10, "y": 10, "width": 100, "height": 100})));
    let err = s.execute("object.convertToSection", &json!({"ids": [inner]})).unwrap_err().to_string();
    assert!(err.contains("inside a frame"), "{err}");
    let r = rect(&mut s, outer, 150.0, 150.0);
    assert!(s.execute("object.convertToSection", &json!({"ids": [r]})).is_err(), "a rectangle isn't a frame");
    // A frame inside a section converts; a section holding a section can't become a frame.
    let sec = id(ex(&mut s, "node.create", json!({"type": "section", "x": 500, "y": 0, "width": 400, "height": 400})));
    let in_sec = id(ex(&mut s, "frame.create", json!({"parent": sec, "x": 20, "y": 20, "width": 100, "height": 100})));
    ex(&mut s, "object.convertToSection", json!({"ids": [in_sec]}));
    assert!(matches!(kind(&s, in_sec), NodeKind::Section { .. }));
    let err = s.execute("object.convertToFrame", &json!({"ids": [sec]})).unwrap_err().to_string();
    assert!(err.contains("holds sections"), "{err}");
    assert!(matches!(kind(&s, sec), NodeKind::Section { .. }), "nothing changed");
}
