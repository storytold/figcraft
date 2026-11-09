//! The Scale tool and variable-bound lengths (#2090): an instance keeps its bindings and draws them
//! at the variable's value times its scale factor; a plain frame bakes the scaled value and drops
//! the binding of each length it scaled (what the incumbent does, measured for the gap).

use serde_json::{Value, json};

use crate::Session;
use crate::components::tests::{copy_of, ex, healthy, idv, node};
use figcraft_doc::NodeId;

/// The scale factor an instance stores (1 when the file has none), read through the saved form.
fn factor(s: &Session, id: NodeId) -> f64 {
    serde_json::to_value(node(s, id)).ok().and_then(|v| v.get("scale_factor").and_then(Value::as_f64)).unwrap_or(1.0)
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn gap(s: &Session, id: NodeId) -> f64 {
    node(s, id).layout.map(|l| l.gap).unwrap_or(f64::NAN)
}

/// Variables Gap = 8 (Dark 12) and Round = 4; a 56 x 24 horizontal auto layout frame holding two
/// 24 x 24 rectangles, its gap bound to Gap and the first rectangle's corner radius to Round.
/// Returns the frame and the first rectangle.
fn bound_row(s: &mut Session) -> (NodeId, NodeId) {
    ex(s, "variable.createCollection", json!({"name": "Theme", "modes": ["Light", "Dark"]}));
    ex(s, "variable.create", json!({"collection": "Theme", "name": "Gap", "type": "number", "value": 8}));
    ex(s, "variable.set", json!({"variable": "Gap", "mode": "Dark", "value": 12}));
    ex(s, "variable.create", json!({"collection": "Theme", "name": "Round", "type": "number", "value": 4}));
    let f = idv(&ex(s, "frame.create", json!({"x": 100, "y": 0, "width": 56, "height": 24, "name": "Row"})));
    ex(s, "node.set", json!({"id": f, "props": {"layout": "horizontal", "padding": 0, "gap": 8, "fills": []}}));
    let a = idv(&ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 24, "height": 24, "parent": f})));
    ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 24, "height": 24, "parent": f}));
    ex(s, "variable.bind", json!({"ids": [f], "field": "gap", "variable": "Gap"}));
    ex(s, "variable.bind", json!({"ids": [a], "field": "cornerRadius", "variable": "Round"}));
    assert!(near(gap(s, f), 8.0), "{}", gap(s, f));
    assert_eq!(node(s, a).corners.radii, [4.0; 4]);
    (f, a)
}

/// The row as a component with one instance; returns (main, first rectangle in main, instance).
fn row_instance(s: &mut Session) -> (NodeId, NodeId, NodeId) {
    let (f, a) = bound_row(s);
    let c = idv(&ex(s, "component.create", json!({"ids": [f]})));
    let i = idv(&ex(s, "instance.create", json!({"component": c, "x": 300, "y": 0})));
    (c, a, i)
}

fn assert_bound_at_half(s: &Session, i: NodeId, a: NodeId, gap_now: f64, why: &str) {
    let ia = copy_of(s, i, a);
    assert!(near(gap(s, i), gap_now), "{why}: gap {}", gap(s, i));
    assert_eq!(node(s, ia).corners.radii, [2.0; 4], "{why}: radii");
    assert!(node(s, i).bound.contains_key("gap"), "{why}: gap binding kept");
    assert!(node(s, ia).bound.contains_key("cornerRadius"), "{why}: radius binding kept");
    let (n, ni) = (node(s, i), node(s, ia));
    assert!(!n.overrides.contains("gap") && !ni.overrides.contains("corners"), "{why}: {:?} {:?}", n.overrides, ni.overrides);
}

#[test]
fn a_scaled_instance_draws_bound_gap_and_radius_at_the_factor_and_keeps_the_bindings() {
    let mut s = Session::new();
    let (c, a, i) = row_instance(&mut s);
    ex(&mut s, "transform.scale", json!({"ids": [i], "factor": 0.5}));
    assert!(near(factor(&s, i), 0.5), "factor {}", factor(&s, i));
    assert_bound_at_half(&s, i, a, 4.0, "scaled");
    healthy(&s);
    ex(&mut s, "variable.setMode", json!({"ids": [i], "collection": "Theme", "mode": "Dark"}));
    assert_bound_at_half(&s, i, a, 6.0, "dark");
    ex(&mut s, "node.set", json!({"id": c, "props": {"fills": ["#ff0000"]}}));
    ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 2, "height": 2, "parent": c}));
    assert_bound_at_half(&s, i, a, 6.0, "rebuilt");
    assert!(near(factor(&s, i), 0.5), "factor after rebuild {}", factor(&s, i));
    healthy(&s);
}

#[test]
fn scaling_an_instance_with_the_tool_scales_bound_values_and_undo_takes_them_back() {
    use crate::tools::{PointerEvent, PointerKind};
    let mut s = Session::new();
    let (_c, a, i) = row_instance(&mut s);
    ex(&mut s, "select.set", json!({"ids": [i]}));
    ex(&mut s, "tool.scale", json!({}));
    // The bottom right handle of the 56 x 24 box at (300, 0), to half its size about the top left.
    for (kind, (x, y)) in
        [(PointerKind::Down, (356.0, 24.0)), (PointerKind::Move, (340.0, 18.0)), (PointerKind::Move, (328.0, 12.0)), (PointerKind::Up, (328.0, 12.0))]
    {
        s.pointer(&PointerEvent::at(kind, x, y, 1.0)).unwrap();
    }
    assert!(near(factor(&s, i), 0.5), "factor {}", factor(&s, i));
    assert_bound_at_half(&s, i, a, 4.0, "dragged");
    s.undo().unwrap();
    assert!(near(factor(&s, i), 1.0), "factor after undo {}", factor(&s, i));
    assert!(near(gap(&s, i), 8.0), "gap after undo {}", gap(&s, i));
    assert!(node(&s, i).overrides.is_empty() && node(&s, copy_of(&s, i, a)).overrides.is_empty());
    healthy(&s);
}

#[test]
fn a_file_with_a_scaled_instance_keeps_its_factor_and_bound_gap_after_save_and_open() {
    let mut s = Session::new();
    let (_c, a, i) = row_instance(&mut s);
    let plain = serde_json::to_value(node(&s, i)).unwrap();
    assert!(plain.get("scale_factor").is_none(), "an unscaled instance saves no factor: {plain}");
    ex(&mut s, "transform.scale", json!({"ids": [i], "factor": 0.5}));
    let dir = std::env::temp_dir().join(format!("figcraft-scale-bound-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("scaled.figcraft");
    ex(&mut s, "file.saveAs", json!({"path": path.to_string_lossy()}));
    let mut back = Session::empty();
    back.execute("file.open", &json!({"path": path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(near(factor(&back, i), 0.5), "factor {}", factor(&back, i));
    assert_bound_at_half(&back, i, a, 4.0, "reopened");
    healthy(&back);
}

#[test]
fn scaling_a_plain_frame_bakes_its_bound_gap_and_drops_the_binding() {
    let mut s = Session::new();
    let (f, a) = bound_row(&mut s);
    ex(&mut s, "transform.scale", json!({"ids": [f], "factor": 0.5}));
    assert!(near(gap(&s, f), 4.0), "gap {}", gap(&s, f));
    assert!(!node(&s, f).bound.contains_key("gap"), "{:?}", node(&s, f).bound);
    assert_eq!(node(&s, a).corners.radii, [2.0; 4]);
    assert!(!node(&s, a).bound.contains_key("cornerRadius"), "{:?}", node(&s, a).bound);
    assert!(near(factor(&s, f), 1.0));
    healthy(&s);
}
