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

#[test]
fn detaching_a_scaled_instance_keeps_its_lengths_and_drops_their_variables() {
    // Measured: detaching a 0.5x instance whose gap is bound leaves a plain frame with the gap
    // at the scaled value and no variable.
    let mut s = Session::new();
    let (_c, a, i) = row_instance(&mut s);
    ex(&mut s, "transform.scale", json!({"ids": [i], "factor": 0.5}));
    let ia = copy_of(&s, i, a);
    ex(&mut s, "component.detach", json!({"ids": [i]}));
    assert!(near(gap(&s, i), 4.0), "gap {}", gap(&s, i));
    assert_eq!(node(&s, ia).corners.radii, [2.0; 4]);
    assert!(!node(&s, i).bound.contains_key("gap"), "the gap's variable goes");
    assert!(!node(&s, ia).bound.contains_key("cornerRadius"), "the radius's variable goes");
    assert!(near(factor(&s, i), 1.0), "a frame has no scale factor: {}", factor(&s, i));
    // The variables no longer reach it.
    ex(&mut s, "variable.set", json!({"variable": "Gap", "mode": "Light", "value": 20}));
    assert!(near(gap(&s, i), 4.0), "gap after the variable changed {}", gap(&s, i));
    healthy(&s);
}

/// Main A holds an instance of main B (the bound row); returns (A, the instance of B in A, an
/// instance of A, the copy of that B instance inside it).
fn nested_instance(s: &mut Session) -> (NodeId, NodeId, NodeId, NodeId) {
    let (b, _a, _i) = row_instance(s);
    let fa = idv(&ex(s, "frame.create", json!({"x": 0, "y": 200, "width": 100, "height": 60, "name": "Holder"})));
    let ib = idv(&ex(s, "instance.create", json!({"component": b, "x": 0, "y": 0, "parent": fa})));
    let a_main = idv(&ex(s, "component.create", json!({"ids": [fa]})));
    let ia = idv(&ex(s, "instance.create", json!({"component": a_main, "x": 500, "y": 0})));
    let inner = copy_of(s, ia, ib);
    (a_main, ib, ia, inner)
}

#[test]
fn an_instance_inside_a_scaled_instance_is_drawn_at_the_outer_factor_once() {
    let mut s = Session::new();
    let (a_main, _ib, ia, inner) = nested_instance(&mut s);
    assert!(near(gap(&s, inner), 8.0), "{}", gap(&s, inner));
    ex(&mut s, "transform.scale", json!({"ids": [ia], "factor": 0.5}));
    assert!(near(factor(&s, ia), 0.5), "outer factor {}", factor(&s, ia));
    assert!(near(gap(&s, inner), 4.0), "the nested copy draws at 0.5 once: gap {}", gap(&s, inner));
    // A change to main A rebuilds the instance: the nested copy is still at 0.5.
    ex(&mut s, "node.set", json!({"id": a_main, "props": {"fills": ["#ff0000"]}}));
    ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 2, "height": 2, "parent": a_main}));
    let inner =
        s.doc().unwrap().subtree(ia).into_iter().find(|d| *d != ia && matches!(node(&s, *d).kind, figcraft_doc::NodeKind::Instance { .. })).unwrap();
    assert!(near(gap(&s, inner), 4.0), "after the main changed: gap {}", gap(&s, inner));
    healthy(&s);
}

#[test]
fn restoring_the_main_from_a_scaled_instance_keeps_its_lengths_and_drops_their_variables() {
    let mut s = Session::new();
    let (c, _a, i) = row_instance(&mut s);
    let j = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 600, "y": 0})));
    ex(&mut s, "transform.scale", json!({"ids": [i], "factor": 0.5}));
    ex(&mut s, "edit.delete", json!({"ids": [c]}));
    let r = idv(&ex(&mut s, "component.restoreMain", json!({"id": i})));
    assert_eq!(r, c);
    assert!(near(gap(&s, c), 4.0), "the restored main's gap {}", gap(&s, c));
    assert!(!node(&s, c).bound.contains_key("gap"), "the gap's variable goes: {:?}", node(&s, c).bound);
    assert!(near(factor(&s, c), 1.0), "a main has no scale factor: {}", factor(&s, c));
    assert!(near(gap(&s, j), 4.0), "another instance re-links and draws the main's gap: {}", gap(&s, j));
    healthy(&s);
}

#[test]
fn a_library_instance_whose_main_is_missing_lands_as_a_frame_with_the_lengths_as_drawn() {
    let dir = std::env::temp_dir().join(format!("figcraft-scale-lib-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let lib_path = dir.join("kit.figcraft").to_string_lossy().to_string();
    let mut lib = Session::new();
    let (b, _a, _i) = row_instance(&mut lib);
    let fa = idv(&ex(&mut lib, "frame.create", json!({"x": 0, "y": 200, "width": 100, "height": 60, "name": "Holder"})));
    let ib = idv(&ex(&mut lib, "instance.create", json!({"component": b, "x": 0, "y": 0, "parent": fa})));
    ex(&mut lib, "transform.scale", json!({"ids": [ib], "factor": 0.5}));
    ex(&mut lib, "component.create", json!({"ids": [fa]}));
    // The row's main goes: the library's instance has no main to bring along.
    ex(&mut lib, "edit.delete", json!({"ids": [b]}));
    ex(&mut lib, "library.publish", json!({"description": "Kit"}));
    ex(&mut lib, "file.saveAs", json!({"path": lib_path}));
    let mut s = Session::new();
    let r = ex(&mut s, "library.add", json!({"path": lib_path}));
    let lid = r["id"].as_str().unwrap().to_string();
    let placed = idv(&ex(&mut s, "library.place", json!({"library": lid, "component": "Holder", "x": 300, "y": 300})));
    let main = match node(&s, placed).kind {
        figcraft_doc::NodeKind::Component { .. } => placed,
        figcraft_doc::NodeKind::Instance { component, .. } => component,
        _ => panic!("not a component or instance"),
    };
    let _ = std::fs::remove_dir_all(&dir);
    let doc = s.doc().unwrap();
    let row = doc.subtree(main).into_iter().find(|d| *d != main && node(&s, *d).layout.is_some()).expect("the row frame");
    assert!(matches!(node(&s, row).kind, figcraft_doc::NodeKind::Frame), "{:?}", node(&s, row).kind);
    assert!(near(gap(&s, row), 4.0), "gap {}", gap(&s, row));
    assert!(near(factor(&s, row), 1.0), "factor {}", factor(&s, row));
    assert!(!node(&s, row).bound.contains_key("gap"), "binding dropped: {:?}", node(&s, row).bound);
}

#[test]
fn scaling_a_frame_holding_an_instance_drops_its_own_binding_and_scales_the_instance_once() {
    let mut s = Session::new();
    let (c, a, _i) = row_instance(&mut s);
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 400, "width": 200, "height": 60, "name": "Outer"})));
    ex(&mut s, "node.set", json!({"id": f, "props": {"layout": "horizontal", "padding": 0, "gap": 8, "fills": []}}));
    let inner = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 0, "y": 0, "parent": f})));
    ex(&mut s, "variable.bind", json!({"ids": [f], "field": "gap", "variable": "Gap"}));
    ex(&mut s, "transform.scale", json!({"ids": [f], "factor": 0.5}));
    assert!(!node(&s, f).bound.contains_key("gap"), "{:?}", node(&s, f).bound);
    assert!(near(gap(&s, f), 4.0), "frame gap {}", gap(&s, f));
    assert!(near(factor(&s, inner), 0.5), "inner factor {}", factor(&s, inner));
    assert_bound_at_half(&s, inner, a, 4.0, "inner");
    // Back to 1: no factor is saved.
    ex(&mut s, "transform.scale", json!({"ids": [f], "factor": 2.0}));
    let saved = serde_json::to_value(node(&s, inner)).unwrap();
    assert!(saved.get("scale_factor").is_none(), "{saved}");
    healthy(&s);
}
