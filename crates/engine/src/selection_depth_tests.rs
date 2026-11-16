//! Selection behaviours: Cmd/Ctrl-click and Cmd/Ctrl-marquee into nested layers, Shift-marquee
//! toggling, Enter / Shift+Enter / Tab / Shift+Tab, and the layers under a point (Select layer).

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind};
use figcraft_doc::NodeId;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn rect_in(s: &mut Session, x: f64, y: f64, parent: Option<NodeId>) -> NodeId {
    let mut p = json!({"x": x, "y": y, "width": 40, "height": 40});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id_of(&ex(s, "shape.rectangle", p)["id"]).unwrap()
}

/// A frame (0,0 300×200) holding a group of two rectangles and a loose one; and one rectangle
/// on the page at (400, 0).
fn scene(s: &mut Session) -> (NodeId, NodeId, [NodeId; 3], NodeId) {
    let f = id_of(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 200}))["id"]).unwrap();
    let a = rect_in(s, 20.0, 20.0, Some(f));
    let b = rect_in(s, 80.0, 20.0, Some(f));
    let g = id_of(&ex(s, "object.group", json!({"ids": [a, b]}))["id"]).unwrap();
    let c = rect_in(s, 20.0, 120.0, Some(f));
    let out = rect_in(s, 400.0, 0.0, None);
    (f, g, [a, b, c], out)
}

fn ev(kind: PointerKind, x: f64, y: f64, shift: bool, cmd: bool) -> PointerEvent {
    let mut e = PointerEvent::at(kind, x, y, 1.0);
    (e.shift, e.cmd) = (shift, cmd);
    e
}

fn marquee(s: &mut Session, a: (f64, f64), b: (f64, f64), shift: bool, cmd: bool) -> Vec<NodeId> {
    s.pointer(&ev(PointerKind::Down, a.0, a.1, shift, cmd)).unwrap();
    s.pointer(&ev(PointerKind::Move, b.0, b.1, shift, cmd)).unwrap();
    s.pointer(&ev(PointerKind::Up, b.0, b.1, shift, cmd)).unwrap();
    s.selection()
}

fn sorted(mut v: Vec<NodeId>) -> Vec<NodeId> {
    v.sort();
    v
}

#[test]
fn cmd_click_selects_the_nested_layer() {
    let mut s = Session::new();
    let (_, g, [a, ..], _) = scene(&mut s);
    s.pointer(&ev(PointerKind::Down, 30.0, 30.0, false, false)).unwrap();
    s.pointer(&ev(PointerKind::Up, 30.0, 30.0, false, false)).unwrap();
    assert_eq!(s.selection(), vec![g], "a click selects the frame's child");
    ex(&mut s, "select.none", json!({}));
    s.pointer(&ev(PointerKind::Down, 30.0, 30.0, false, true)).unwrap();
    s.pointer(&ev(PointerKind::Up, 30.0, 30.0, false, true)).unwrap();
    assert_eq!(s.selection(), vec![a], "Cmd/Ctrl-click goes straight to the layer");
}

#[test]
fn cmd_marquee_selects_nested_layers() {
    let mut s = Session::new();
    let (f, g, [a, b, c], out) = scene(&mut s);
    // From empty canvas, a plain marquee across the frame (not fully covered) gives the layers in it
    // it touches (the group whole), plus the top-level layer.
    assert_eq!(sorted(marquee(&mut s, (-10.0, 10.0), (500.0, 70.0), false, false)), sorted(vec![g, out]));
    // Cmd/Ctrl: the innermost layers it touches, through the frame and the group.
    assert_eq!(sorted(marquee(&mut s, (-10.0, 10.0), (500.0, 70.0), false, true)), sorted(vec![a, b, out]));
    assert_eq!(sorted(marquee(&mut s, (-10.0, 10.0), (100.0, 190.0), false, true)), sorted(vec![a, b, c]));
    // Only the frame's own background touched: the frame itself.
    assert_eq!(marquee(&mut s, (350.0, 180.0), (250.0, 150.0), false, true), vec![f]);
}

#[test]
fn shift_marquee_toggles() {
    let mut s = Session::new();
    let (_, _, _, out) = scene(&mut s);
    let other = rect_in(&mut s, 400.0, 100.0, None);
    ex(&mut s, "select.set", json!({"ids": [out]}));
    // Touching both (starting clear of the selection's handles): the selected one comes out, the
    // other goes in.
    assert_eq!(marquee(&mut s, (370.0, -30.0), (450.0, 150.0), true, false), vec![other]);
    assert_eq!(sorted(marquee(&mut s, (370.0, -30.0), (450.0, 20.0), true, false)), sorted(vec![out, other]));
}

#[test]
fn keyboard_selection_moves_through_the_tree() {
    let mut s = Session::new();
    let (f, g, [a, b, c], _) = scene(&mut s);
    ex(&mut s, "select.set", json!({"ids": [f]}));
    ex(&mut s, "select.children", json!({}));
    assert_eq!(sorted(s.selection()), sorted(vec![g, c]), "Enter");
    ex(&mut s, "select.set", json!({"ids": [a]}));
    ex(&mut s, "select.parent", json!({}));
    assert_eq!(s.selection(), vec![g], "Shift+Enter");
    ex(&mut s, "select.set", json!({"ids": [b]}));
    // Tab goes down the layers list (back in z order), wrapping; Shift+Tab goes up.
    ex(&mut s, "select.next", json!({}));
    assert_eq!(s.selection(), vec![a], "Tab");
    ex(&mut s, "select.previous", json!({}));
    assert_eq!(s.selection(), vec![b], "Shift+Tab");
    ex(&mut s, "select.previous", json!({}));
    assert_eq!(s.selection(), vec![a], "wraps");
}

#[test]
fn layers_at_a_point_list_the_stack_front_most_first() {
    let mut s = Session::new();
    let (f, g, [a, ..], _) = scene(&mut s);
    let over = rect_in(&mut s, 10.0, 10.0, Some(f));
    let out = ex(&mut s, "select.layersAt", json!({"x": 30, "y": 30}));
    let got: Vec<(NodeId, u64)> =
        out["layers"].as_array().unwrap().iter().map(|l| (id_of(&l["id"]).unwrap(), l["depth"].as_u64().unwrap())).collect();
    assert_eq!(got, vec![(f, 0), (over, 1), (g, 1), (a, 2)]);
    assert!(out["layers"].as_array().unwrap().iter().all(|l| l["locked"] == json!(false)), "none is locked");
    assert_eq!(out["layers"][0]["type"], json!("FRAME"));
    // Nothing under empty canvas; junk is refused.
    assert_eq!(ex(&mut s, "select.layersAt", json!({"x": 2000, "y": 2000}))["layers"], json!([]));
    assert!(s.execute("select.layersAt", &json!({"x": "left"})).is_err());
}

/// Measured in Figma's web app, 2026-11-10 (Select layer lists a locked layer, and picking it selects it).
#[test]
fn layers_at_a_point_list_locked_layers_with_a_flag_and_select_set_takes_them() {
    let mut s = Session::new();
    let (f, g, [a, ..], _) = scene(&mut s);
    let over = rect_in(&mut s, 10.0, 10.0, Some(f));
    let lk = rect_in(&mut s, 10.0, 10.0, Some(f));
    ex(&mut s, "object.toggleLock", json!({"ids": [lk], "locked": true}));
    let out = ex(&mut s, "select.layersAt", json!({"x": 30, "y": 30}));
    let got: Vec<(NodeId, u64, bool)> = out["layers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| (id_of(&l["id"]).unwrap(), l["depth"].as_u64().unwrap(), l["locked"].as_bool().unwrap()))
        .collect();
    assert_eq!(got, vec![(f, 0, false), (lk, 1, true), (over, 1, false), (g, 1, false), (a, 2, false)]);
    ex(&mut s, "select.set", json!({"ids": [lk]}));
    assert_eq!(s.selection(), vec![lk]);
    ex(&mut s, "select.none", json!({}));
    s.pointer(&ev(PointerKind::Down, 30.0, 30.0, false, false)).unwrap();
    s.pointer(&ev(PointerKind::Up, 30.0, 30.0, false, false)).unwrap();
    assert_eq!(s.selection(), vec![over], "a click still skips the locked layer");
}
