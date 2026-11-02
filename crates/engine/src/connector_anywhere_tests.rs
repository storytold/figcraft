//! A connector end placed on any part of an object: holding Cmd/Ctrl while moving it puts it right
//! where the pointer is, and it stays at that part of the object when the object moves or resizes.

use figcraft_doc::{End, NodeId, Side};
use figcraft_geom::Point;
use serde_json::json;

use crate::Session;
use crate::board_tests::{board, connector, drawn, id, near};
use crate::tools::{PointerEvent, PointerKind};

/// A board with two 200 × 100 shapes and a straight connector between them: (a, b, connector).
fn scene() -> (Session, NodeId, NodeId, NodeId) {
    let mut s = board();
    let mut shape = |x: f64| id(&s.execute("board.shape", &json!({"x": x, "y": 0, "width": 200, "height": 100})).unwrap(), "id");
    let (a, b) = (shape(0.0), shape(500.0));
    let k = id(&s.execute("board.connector", &json!({"from": a, "to": b, "style": "straight"})).unwrap(), "id");
    s.execute("select.set", &json!({"ids": [k]})).unwrap();
    (s, a, b, k)
}

/// Drag the connector's end at `from` to `to`, with Cmd/Ctrl held or not.
fn drag_end(s: &mut Session, from: Point, to: Point, cmd: bool) {
    let ev = |kind, p: Point| PointerEvent { cmd, ..PointerEvent::at(kind, p.x, p.y, 1.0) };
    s.pointer(&ev(PointerKind::Down, from)).unwrap();
    s.pointer(&ev(PointerKind::Move, Point::new(300.0, 300.0))).unwrap();
    s.pointer(&ev(PointerKind::Move, to)).unwrap();
    s.pointer(&ev(PointerKind::Up, to)).unwrap();
}

#[test]
fn cmd_puts_an_end_on_any_part_of_the_object() {
    let (mut s, _, b, k) = scene();
    let end = *drawn(&s, k).last().unwrap();
    drag_end(&mut s, end, Point::new(650.0, 25.0), true);
    assert_eq!(connector(&s, k).to, End::Node { node: b, side: Side::Auto, at: Some([0.75, 0.25]) });
    assert!(near(*drawn(&s, k).last().unwrap(), Point::new(650.0, 25.0)), "{:?}", drawn(&s, k));
    // It undoes as one step, back to the side.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(connector(&s, k).to, End::Node { node: b, side: Side::Auto, at: None });
}

#[test]
fn without_cmd_an_end_keeps_snapping_to_a_side() {
    let (mut s, _, b, k) = scene();
    let end = *drawn(&s, k).last().unwrap();
    drag_end(&mut s, end, Point::new(650.0, 25.0), false);
    assert_eq!(connector(&s, k).to, End::Node { node: b, side: Side::Right, at: None });
    assert!(near(*drawn(&s, k).last().unwrap(), Point::new(700.0, 50.0)));
}

#[test]
fn the_end_stays_on_that_part_when_the_object_moves_or_resizes() {
    let (mut s, _, b, k) = scene();
    s.execute("board.reattach", &json!({"id": k, "end": "to", "target": b, "at": [0.5, 0.5]})).unwrap();
    assert!(near(*drawn(&s, k).last().unwrap(), Point::new(600.0, 50.0)));
    s.execute("node.set", &json!({"id": b, "props": {"x": 600, "y": 100, "width": 400, "height": 200}})).unwrap();
    assert!(near(*drawn(&s, k).last().unwrap(), Point::new(800.0, 200.0)), "{:?}", drawn(&s, k));
}

#[test]
fn an_end_moved_back_without_cmd_leaves_its_place() {
    let (mut s, _, b, k) = scene();
    s.execute("board.reattach", &json!({"id": k, "end": "to", "target": b, "at": [0.5, 0.5]})).unwrap();
    s.execute("board.reattach", &json!({"id": k, "end": "to", "target": b, "side": "top"})).unwrap();
    assert_eq!(connector(&s, k).to, End::Node { node: b, side: Side::Top, at: None });
}

#[test]
fn the_commands_take_a_place_and_refuse_junk() {
    let (mut s, a, b, _) = scene();
    let k = id(&s.execute("board.connector", &json!({"from": a, "to": b, "fromAt": [0.25, 0.5], "toAt": [9, -1]})).unwrap(), "id");
    assert!(matches!(connector(&s, k).from, End::Node { at: Some([0.25, 0.5]), .. }));
    assert!(matches!(connector(&s, k).to, End::Node { at: Some([1.0, 0.0]), .. }), "kept on the object");
    for junk in [json!("middle"), json!([1]), json!([1, 2, 3]), json!(["x", 0]), json!({"x": 0, "y": 0})] {
        assert!(s.execute("board.reattach", &json!({"id": k, "end": "to", "target": b, "at": junk})).is_err());
    }
}

#[test]
fn a_reshaped_path_keeps_the_end_where_it_is() {
    let (mut s, a, b, _) = scene();
    let k = id(&s.execute("board.connector", &json!({"from": a, "to": b, "style": "curved", "toAt": [0.5, 0.5]})).unwrap(), "id");
    s.execute("board.reshape", &json!({"id": k, "portion": 0, "x": 350, "y": 200})).unwrap();
    assert!(matches!(connector(&s, k).to, End::Node { at: Some([0.5, 0.5]), .. }));
    assert!(near(*drawn(&s, k).last().unwrap(), Point::new(600.0, 50.0)), "{:?}", drawn(&s, k));
}

#[test]
fn it_saves_only_when_placed_by_hand() {
    let (mut s, _, b, k) = scene();
    let saved = |s: &Session| figcraft_format::to_json(s.doc().unwrap()).unwrap().to_string();
    assert!(!saved(&s).contains("\"at\":"), "older files stay as they are");
    s.execute("board.reattach", &json!({"id": k, "end": "to", "target": b, "at": [0.5, 0.25]})).unwrap();
    assert!(saved(&s).contains("\"at\":[0.5,0.25]"));
    let bytes = figcraft_format::save_bytes(s.doc().unwrap(), None).unwrap();
    let (open, _) = figcraft_format::load_bytes(&bytes).unwrap();
    let kept = open.node(k).and_then(|n| n.board.clone());
    assert!(matches!(kept, Some(figcraft_doc::BoardItem::Connector(c)) if matches!(c.to, End::Node { at: Some([0.5, 0.25]), .. })));
}
