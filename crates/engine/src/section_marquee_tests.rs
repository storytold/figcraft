use figcraft_doc::NodeId;
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind};

fn ex(s: &mut Session, cmd: &str, p: Value) -> NodeId {
    id_of(&s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))["id"]).unwrap()
}

fn rect(s: &mut Session, x: f64, y: f64, parent: NodeId) -> NodeId {
    ex(s, "shape.rectangle", json!({"x": x, "y": y, "width": 40, "height": 40, "parent": parent}))
}

struct Scene {
    section: NodeId,
    left: NodeId,
    right: NodeId,
    in_left: [NodeId; 2],
    in_right: NodeId,
}

fn scene(s: &mut Session) -> Scene {
    let section = ex(s, "node.create", json!({"type": "section", "x": 0, "y": 0, "width": 1000, "height": 600}));
    let left = ex(s, "frame.create", json!({"x": 50, "y": 50, "width": 300, "height": 300, "parent": section}));
    let right = ex(s, "frame.create", json!({"x": 400, "y": 50, "width": 300, "height": 300, "parent": section}));
    let in_left = [rect(s, 20.0, 20.0, left), rect(s, 120.0, 120.0, left)];
    let in_right = rect(s, 20.0, 20.0, right);
    s.execute("select.none", &json!({})).unwrap();
    Scene { section, left, right, in_left, in_right }
}

fn ev(kind: PointerKind, p: (f64, f64), cmd: bool) -> PointerEvent {
    PointerEvent { cmd, ..PointerEvent::at(kind, p.0, p.1, 1.0) }
}

fn drag(s: &mut Session, a: (f64, f64), b: (f64, f64), cmd: bool) -> Vec<NodeId> {
    s.pointer(&ev(PointerKind::Down, a, cmd)).unwrap();
    s.pointer(&ev(PointerKind::Move, b, cmd)).unwrap();
    s.pointer(&ev(PointerKind::Up, b, cmd)).unwrap();
    let mut sel = s.selection();
    sel.sort();
    sel
}

fn x_of(s: &Session, id: NodeId) -> f64 {
    s.doc().unwrap().world_bounds(id).unwrap().x0
}

fn sorted(mut v: Vec<NodeId>) -> Vec<NodeId> {
    v.sort();
    v
}

#[test]
fn a_drag_on_an_artboards_background_in_a_section_selects_what_it_touches() {
    let mut s = Session::new();
    let k = scene(&mut s);
    assert_eq!(drag(&mut s, (60.0, 60.0), (110.0, 110.0), false), vec![k.in_left[0]]);
    assert_eq!(drag(&mut s, (340.0, 340.0), (60.0, 60.0), false), sorted(k.in_left.to_vec()));
    assert_eq!((x_of(&s, k.section), x_of(&s, k.left)), (0.0, 50.0), "nothing moved");
}

#[test]
fn a_drag_on_a_sections_background_selects_the_artboards_layers_it_touches() {
    let mut s = Session::new();
    let k = scene(&mut s);
    assert_eq!(drag(&mut s, (20.0, 400.0), (480.0, 80.0), false), sorted(vec![k.in_left[0], k.in_left[1], k.in_right]));
    assert_eq!(drag(&mut s, (20.0, 20.0), (720.0, 380.0), false), sorted(vec![k.left, k.right]), "artboards it encloses go whole");
    assert_eq!(x_of(&s, k.section), 0.0, "the section stays put");
}

#[test]
fn a_selected_section_still_drags_and_a_click_on_its_background_steps_out_to_it() {
    let mut s = Session::new();
    let k = scene(&mut s);
    s.execute("select.set", &json!({"ids": [k.section]})).unwrap();
    drag(&mut s, (20.0, 400.0), (120.0, 400.0), false);
    assert_eq!(x_of(&s, k.section), 100.0);
    s.execute("select.set", &json!({"ids": [k.in_right]})).unwrap();
    assert_eq!(drag(&mut s, (900.0, 500.0), (900.0, 500.0), false), vec![k.section]);
}

#[test]
fn cmd_drag_from_a_background_marks_out_nested_layers_and_cmd_click_still_picks_the_frame() {
    let mut s = Session::new();
    let k = scene(&mut s);
    assert_eq!(drag(&mut s, (60.0, 60.0), (110.0, 110.0), true), vec![k.in_left[0]]);
    assert_eq!(drag(&mut s, (20.0, 400.0), (480.0, 80.0), true), sorted(vec![k.in_left[0], k.in_left[1], k.in_right]));
    assert_eq!((x_of(&s, k.section), x_of(&s, k.left)), (0.0, 50.0), "nothing moved");
    assert_eq!(drag(&mut s, (300.0, 300.0), (300.0, 300.0), true), vec![k.left]);
    assert_eq!(drag(&mut s, (900.0, 500.0), (900.0, 500.0), true), vec![k.section]);
}

#[test]
fn cmd_drag_inside_a_selected_artboard_or_section_marks_out_nested_layers_instead_of_moving_it() {
    let mut s = Session::new();
    let k = scene(&mut s);
    s.execute("select.set", &json!({"ids": [k.left]})).unwrap();
    assert_eq!(drag(&mut s, (130.0, 75.0), (90.0, 90.0), true), vec![k.in_left[0]]);
    s.execute("select.set", &json!({"ids": [k.section]})).unwrap();
    assert_eq!(drag(&mut s, (20.0, 400.0), (480.0, 80.0), true), sorted(vec![k.in_left[0], k.in_left[1], k.in_right]));
    assert_eq!((x_of(&s, k.section), x_of(&s, k.left)), (0.0, 50.0), "nothing moved");
    s.execute("select.set", &json!({"ids": [k.left]})).unwrap();
    assert_eq!(drag(&mut s, (300.0, 300.0), (300.0, 300.0), true), vec![k.left], "a Cmd click keeps the artboard");
    assert_eq!(drag(&mut s, (80.0, 80.0), (80.0, 80.0), true), vec![k.in_left[0]], "a Cmd click on a layer in it picks the layer");

    let mut s = Session::new();
    let board = ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 400, "height": 400}));
    let inner = [rect(&mut s, 20.0, 20.0, board), rect(&mut s, 200.0, 200.0, board)];
    s.execute("select.set", &json!({"ids": [board]})).unwrap();
    assert_eq!(drag(&mut s, (330.0, 30.0), (10.0, 300.0), true), sorted(inner.to_vec()), "a top-level artboard too");
    assert_eq!(x_of(&s, board), 0.0);
}
