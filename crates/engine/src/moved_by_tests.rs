//! `Interaction::moved_by` (#1571): set while a drag has only moved the selected layers, each by the
//! same offset from the document the gesture started from, so the UI can shift what it worked out
//! from that document; `None` for anything else (a layer in an auto layout flow, a copy made with
//! Alt), and gone with the gesture.

use figcraft_doc::NodeId;
use figcraft_geom::Point;
use serde_json::json;

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind};

fn rect(s: &mut Session, parent: Option<NodeId>, x: f64, y: f64) -> NodeId {
    let mut p = json!({"x": x, "y": y, "width": 40, "height": 40});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id_of(&s.execute("shape.rectangle", &p).unwrap()["id"]).unwrap()
}

fn ev(kind: PointerKind, x: f64, y: f64, alt: bool) -> PointerEvent {
    PointerEvent { alt, ..PointerEvent::at(kind, x, y, 1.0) }
}

/// Press at `from` and move to `to`, holding the button: what the interaction says of it.
fn moved_by(s: &mut Session, from: (f64, f64), to: (f64, f64), alt: bool) -> Option<(f64, f64)> {
    s.pointer(&ev(PointerKind::Down, from.0, from.1, alt)).unwrap();
    s.pointer(&ev(PointerKind::Move, (from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0, alt)).unwrap();
    s.pointer(&ev(PointerKind::Move, to.0, to.1, alt)).unwrap();
    s.ds().unwrap().interaction.as_ref().and_then(|it| it.moved_by)
}

fn release(s: &mut Session, at: (f64, f64)) {
    s.pointer(&ev(PointerKind::Up, at.0, at.1, false)).unwrap();
}

fn world(s: &Session, id: NodeId) -> Point {
    let t = s.doc().unwrap().world_transform(id).translation();
    Point::new(t.x, t.y)
}

#[test]
fn a_plain_move_says_how_far_and_matches_the_document() {
    let mut s = Session::new();
    let a = rect(&mut s, None, 0.0, 0.0);
    let b = rect(&mut s, None, 100.0, 0.0);
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let (before_a, before_b) = (world(&s, a), world(&s, b));
    let d = moved_by(&mut s, (20.0, 20.0), (57.0, 81.0), false).expect("a plain move");
    assert_eq!(d, (37.0, 61.0));
    for (id, was) in [(a, before_a), (b, before_b)] {
        assert_eq!(world(&s, id), Point::new(was.x + d.0, was.y + d.1), "each layer moved by it");
    }
    release(&mut s, (57.0, 81.0));
    assert!(s.ds().unwrap().interaction.is_none(), "gone with the gesture");
}

#[test]
fn a_layer_selected_with_its_frame_moves_with_it_once() {
    let mut s = Session::new();
    let g = id_of(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 200})).unwrap()["id"]).unwrap();
    let inner = rect(&mut s, Some(g), 20.0, 20.0);
    s.execute("select.set", &json!({"ids": [g, inner]})).unwrap();
    let was = world(&s, inner);
    let d = moved_by(&mut s, (25.0, 25.0), (80.0, 90.0), false).expect("the frame moves, and the layer in it with it");
    assert_eq!(world(&s, inner), Point::new(was.x + d.0, was.y + d.1));
}

#[test]
fn other_moves_say_nothing() {
    // A layer placed by its frame's auto layout.
    let mut s = Session::new();
    let f = id_of(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 300, "height": 300})).unwrap()["id"]).unwrap();
    let a = rect(&mut s, Some(f), 10.0, 10.0);
    rect(&mut s, Some(f), 60.0, 10.0);
    s.execute("layout.add", &json!({"ids": [f]})).unwrap();
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    let at = world(&s, a);
    assert_eq!(moved_by(&mut s, (at.x + 5.0, at.y + 5.0), (at.x + 9.0, at.y + 200.0), false), None, "in an auto layout flow");
    release(&mut s, (at.x + 9.0, at.y + 200.0));
    // A copy dragged out with Alt.
    let mut s = Session::new();
    let a = rect(&mut s, None, 0.0, 0.0);
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    assert_eq!(moved_by(&mut s, (20.0, 20.0), (90.0, 20.0), true), None, "a copy");
}
