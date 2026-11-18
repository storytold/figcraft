//! Instances scaled with the Scale tool keep their scaled layers when a main around them or their
//! own main changes (#922): a 24 px icon with a 1.5 px stroke shown at 14 px has a 0.875 px stroke,
//! and its points and size are 14/24 of its main's.

use serde_json::{Value, json};

use crate::Session;
use crate::components::tests::{copy_of, ex, healthy, idv, node};
use figcraft_doc::{NodeId, NodeKind};

/// The scale the icon is shown at: 24 px to 14 px.
const K: f64 = 14.0 / 24.0;

/// Whether two lengths are equal up to rounding.
fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// The icon: a 24 px component holding a flattened triangle with a 1.5 px stroke; returns the
/// component and the triangle.
fn icon(s: &mut Session) -> (NodeId, NodeId) {
    let f = idv(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 24, "height": 24})));
    let p = idv(&ex(s, "shape.polygon", json!({"x": 4, "y": 4, "width": 16, "height": 16, "parent": f})));
    ex(s, "vector.flatten", json!({"ids": [p]}));
    let v = node(s, f).children[0];
    assert!(matches!(node(s, v).kind, NodeKind::Vector { .. }));
    ex(s, "node.set", json!({"id": v, "props": {"strokes": ["#000000"], "strokeWeight": 1.5}}));
    let c = idv(&ex(s, "component.create", json!({"ids": [f]})));
    (c, v)
}

/// `copy` shows main layer `main` at 14/24: stroke, size and points.
fn assert_scaled(s: &Session, copy: NodeId, main: NodeId, why: &str) {
    let (c, m) = (node(s, copy), node(s, main));
    assert!(near(c.stroke.weight, m.stroke.weight * K), "{why}: stroke {} (main {})", c.stroke.weight, m.stroke.weight);
    assert!(near(c.width, m.width * K) && near(c.height, m.height * K), "{why}: size {}×{} (main {}×{})", c.width, c.height, m.width, m.height);
    let (NodeKind::Vector { network: a }, NodeKind::Vector { network: b }) = (&c.kind, &m.kind) else { panic!("{why}: not vectors") };
    assert_eq!(a.vertices.len(), b.vertices.len(), "{why}");
    for (va, vb) in a.vertices.iter().zip(&b.vertices) {
        assert!(near(va.x, vb.x * K) && near(va.y, vb.y * K), "{why}: point ({}, {}) (main ({}, {}))", va.x, va.y, vb.x, vb.y);
    }
}

/// Edits of a main: a value on its root, a value on a layer, a layer added (a rebuild).
fn main_edits(main: NodeId, layer: NodeId) -> Vec<(&'static str, Value)> {
    vec![
        ("effects on the main", json!({"id": main, "props": {"effects": [{"type": "dropShadow"}]}})),
        ("effects cleared on the main", json!({"id": main, "props": {"effects": []}})),
        ("fill on the main", json!({"id": main, "props": {"fills": ["#ff0000"]}})),
        ("fill on a main layer", json!({"id": layer, "props": {"fills": ["#00ff00"]}})),
    ]
}

/// An instance scaled on the page keeps its scaled layers through value edits of its main and
/// a rebuild (a layer added to the main).
#[test]
fn a_scaled_instance_keeps_its_scale_when_its_main_changes() {
    let mut s = Session::new();
    let (c, v) = icon(&mut s);
    let i = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 100, "y": 0})));
    ex(&mut s, "transform.scale", json!({"ids": [i], "factor": K}));
    let iv = copy_of(&s, i, v);
    assert_scaled(&s, iv, v, "scaled");
    for (why, p) in main_edits(c, v) {
        ex(&mut s, "node.set", p);
        assert_scaled(&s, copy_of(&s, i, v), v, why);
    }
    ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 2, "height": 2, "parent": c}));
    assert_scaled(&s, copy_of(&s, i, v), v, "a layer added to the main");
    healthy(&s);
}

/// Dragging a Scale tool handle records the same overrides, and undoing the drag takes them back.
#[test]
fn an_instance_scaled_with_the_tool_keeps_its_scale_and_undo_takes_it_back() {
    use crate::tools::{PointerEvent, PointerKind};
    let mut s = Session::new();
    let (c, v) = icon(&mut s);
    let i = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 100, "y": 0})));
    ex(&mut s, "select.set", json!({"ids": [i]}));
    ex(&mut s, "tool.scale", json!({}));
    // The bottom right handle, from 24 px to 14 px about the top left corner.
    for (kind, (x, y)) in
        [(PointerKind::Down, (124.0, 24.0)), (PointerKind::Move, (119.0, 19.0)), (PointerKind::Move, (114.0, 14.0)), (PointerKind::Up, (114.0, 14.0))]
    {
        s.pointer(&PointerEvent::at(kind, x, y, 1.0)).unwrap();
    }
    assert!(near(node(&s, i).width, 14.0), "{}", node(&s, i).width);
    assert_scaled(&s, copy_of(&s, i, v), v, "scaled with the tool");
    ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 2, "height": 2, "parent": c}));
    assert_scaled(&s, copy_of(&s, i, v), v, "a layer added to the main");
    s.undo().unwrap();
    s.undo().unwrap();
    let back = node(&s, copy_of(&s, i, v));
    assert!(near(back.stroke.weight, 1.5) && back.overrides.is_empty(), "undo: {} {:?}", back.stroke.weight, back.overrides);
    healthy(&s);
}

/// The report's case: an icon scaled inside a header that then becomes a component keeps its scale
/// in the header and in the header's instances, through edits of both mains.
#[test]
fn a_scaled_nested_icon_keeps_its_scale_when_the_component_holding_it_changes() {
    let mut s = Session::new();
    let (c, v) = icon(&mut s);
    // The header holds the icon at 14 px, scaled with the Scale tool, then becomes a component.
    let h = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 100, "width": 200, "height": 48})));
    let hi = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 10, "y": 117, "parent": h})));
    ex(&mut s, "transform.scale", json!({"ids": [hi], "factor": K}));
    let header = idv(&ex(&mut s, "component.create", json!({"ids": [h]})));
    let screen = idv(&ex(&mut s, "instance.create", json!({"component": header, "x": 0, "y": 300})));
    // The icon's triangle as the header and as the instance of the header show it.
    let both = |s: &Session| {
        let in_header = copy_of(s, hi, v);
        [in_header, copy_of(s, copy_of(s, screen, hi), in_header)]
    };
    let check = |s: &Session, why: &str| {
        for (copy, where_) in both(s).into_iter().zip(["the header's icon", "the icon in an instance of the header"]) {
            assert_scaled(s, copy, v, &format!("{why}: {where_}"));
        }
    };
    check(&s, "made a component");
    for (why, p) in main_edits(header, hi).into_iter().chain(main_edits(c, v)) {
        ex(&mut s, "node.set", p);
        check(&s, why);
    }
    ex(&mut s, "shape.rectangle", json!({"x": 150, "y": 110, "width": 20, "height": 20, "parent": header}));
    check(&s, "a layer added to the header");
    ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 2, "height": 2, "parent": c}));
    check(&s, "a layer added to the icon");
    healthy(&s);
}
