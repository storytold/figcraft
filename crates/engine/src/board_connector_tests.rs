//! Board connectors, continued: moving an end onto another object or off into the canvas (the
//! command and a drag of the selected connector's end), curved connectors, and elbows that go
//! around the objects they join.

use figcraft_doc::{BoardItem, End, NodeKind, Routing, Side};
use figcraft_geom::{Point, Rect};
use serde_json::json;

use crate::Session;
use crate::board::route;
use crate::board_tests::{board, connector, drawn, id, near, node};
use crate::tools::{PointerEvent, PointerKind, handle_at};

/// A board with three 100 × 100 shapes in a row 300 apart, and an elbow connector from the
/// first to the second: (a, b, c, connector).
fn three() -> (Session, [figcraft_doc::NodeId; 4]) {
    let mut s = board();
    let mut shape = |x: f64| id(&s.execute("board.shape", &json!({"x": x, "y": 0, "width": 100, "height": 100})).unwrap(), "id");
    let (a, b, c) = (shape(0.0), shape(300.0), shape(600.0));
    let k = id(&s.execute("board.connector", &json!({"from": a, "to": b, "style": "elbow"})).unwrap(), "id");
    (s, [a, b, c, k])
}

#[test]
fn reattach_moves_one_end_and_undoes() {
    let (mut s, [a, b, c, k]) = three();
    s.execute("board.reattach", &json!({"id": k, "end": "to", "target": c, "side": "top"})).unwrap();
    assert_eq!(connector(&s, k).to, End::Node { node: c, side: Side::Top });
    assert_eq!(connector(&s, k).from.node(), Some(a));
    assert!(near(*drawn(&s, k).last().unwrap(), Point::new(650.0, 0.0)), "arrives on c's top: {:?}", drawn(&s, k));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(connector(&s, k).to.node(), Some(b), "one undo step");
    // Off into the canvas: the end stays there, free. The selection is the default connector.
    s.execute("select.set", &json!({"ids": [k]})).unwrap();
    s.execute("board.reattach", &json!({"end": "from", "target": {"x": -200, "y": 400}})).unwrap();
    assert_eq!(connector(&s, k).from, End::Point { x: -200.0, y: 400.0 });
    assert!(near(drawn(&s, k)[0], Point::new(-200.0, 400.0)));
    // A label stands for its connector.
    let label = id(&s.execute("board.label", &json!({"id": k, "text": "next"})).unwrap(), "label");
    s.execute("board.reattach", &json!({"id": label, "end": "from", "target": c})).unwrap();
    assert_eq!(connector(&s, k).from.node(), Some(c));
}

#[test]
fn reattach_refuses_what_it_cant_join() {
    let (mut s, [a, b, _, k]) = three();
    let refused = |s: &mut Session, p| s.execute("board.reattach", &p).is_err();
    assert!(refused(&mut s, json!({"id": k, "end": "to", "target": a})), "both ends on one layer");
    assert!(refused(&mut s, json!({"id": k, "end": "middle", "target": b})));
    assert!(refused(&mut s, json!({"id": k, "end": "to"})), "no target");
    assert!(refused(&mut s, json!({"id": a, "end": "to", "target": b})), "not a connector");
    assert!(refused(&mut s, json!({"id": k, "end": "to", "target": k})), "a connector isn't an end");
    assert!(refused(&mut s, json!({"id": k, "end": "to", "target": {"x": "a", "y": 1}})));
    assert!(refused(&mut s, json!({"id": k, "end": "to", "target": b, "side": "inside"})));
    let mut d = Session::new();
    d.execute("file.new", &json!({})).unwrap();
    assert!(d.execute("board.reattach", &json!({"end": "to", "target": {"x": 0, "y": 0}})).is_err(), "boards only");
    assert_eq!(connector(&s, k).to.node(), Some(b), "nothing changed");
}

#[test]
fn dragging_a_selected_connectors_end_reattaches_it() {
    let (mut s, [a, b, c, k]) = three();
    s.execute("select.set", &json!({"ids": [k]})).unwrap();
    let doc = s.doc().unwrap();
    assert_eq!(handle_at(doc, &[k], Point::new(100.0, 50.0), 1.0), None, "no box handles on a connector");
    let end = *drawn(&s, k).last().unwrap();
    let ev = |k, p: Point| PointerEvent::at(k, p.x, p.y, 1.0);
    // Onto c.
    s.pointer(&ev(PointerKind::Down, end)).unwrap();
    s.pointer(&ev(PointerKind::Move, Point::new(500.0, 50.0))).unwrap();
    s.pointer(&ev(PointerKind::Move, Point::new(650.0, 50.0))).unwrap();
    assert_eq!(s.tool.overlay.drop_target, Some(c), "the object it would attach to shows");
    assert!(s.tool.overlay.rubber.is_some());
    s.pointer(&ev(PointerKind::Up, Point::new(650.0, 50.0))).unwrap();
    assert_eq!(connector(&s, k).to.node(), Some(c));
    assert_eq!(connector(&s, k).from.node(), Some(a));
    assert_eq!((s.tool.overlay.drop_target, s.tool.overlay.rubber), (None, None));
    assert_eq!(s.selection(), vec![k], "still selected");
    // The start into empty canvas: free where it was let go.
    let start = drawn(&s, k)[0];
    s.pointer(&ev(PointerKind::Down, start + figcraft_geom::Vec2::new(3.0, 0.0))).unwrap();
    s.pointer(&ev(PointerKind::Move, Point::new(100.0, 400.0))).unwrap();
    s.pointer(&ev(PointerKind::Up, Point::new(100.0, 400.0))).unwrap();
    assert_eq!(connector(&s, k).from, End::Point { x: 100.0, y: 400.0 });
    // A click on an end, or a drop on the other end's object, changes nothing.
    let before = connector(&s, k);
    let end = *drawn(&s, k).last().unwrap();
    s.pointer(&ev(PointerKind::Down, end)).unwrap();
    s.pointer(&ev(PointerKind::Up, end)).unwrap();
    assert_eq!(connector(&s, k), before);
    s.execute("board.reattach", &json!({"id": k, "end": "from", "target": b})).unwrap();
    let before = connector(&s, k);
    s.pointer(&ev(PointerKind::Down, *drawn(&s, k).last().unwrap())).unwrap();
    s.pointer(&ev(PointerKind::Move, Point::new(350.0, 50.0))).unwrap();
    assert_eq!(s.tool.overlay.drop_target, None, "not onto the other end's object");
    s.pointer(&ev(PointerKind::Up, Point::new(350.0, 50.0))).unwrap();
    assert_eq!(connector(&s, k), before);
}

#[test]
fn curved_connectors_leave_and_arrive_along_the_sides() {
    let (mut s, [a, _, c, k]) = three();
    s.execute("board.setStyle", &json!({"ids": [k], "style": "curved"})).unwrap();
    assert_eq!(connector(&s, k).routing, Routing::Curved);
    let n = node(&s, k);
    let NodeKind::Vector { network } = &n.kind else { panic!("a vector") };
    assert_eq!((network.vertices.len(), network.segments.len()), (2, 1));
    assert!(!network.segments[0].is_straight(), "one curve");
    assert_eq!(drawn(&s, k), vec![Point::new(100.0, 50.0), Point::new(300.0, 50.0)]);
    // Side to side in a line: the curve's control points lie on that line, so its box is flat.
    assert!(n.height.abs() < 1e-9 && (n.width - 200.0).abs() < 1e-9, "{} × {}", n.width, n.height);
    // From a's bottom to c's top: out downwards and in from above; the box holds the whole curve.
    let k2 = id(
        &s.execute("board.connector", &json!({"from": a, "to": c, "fromSide": "bottom", "toSide": "top", "style": "curved", "label": "x"})).unwrap(),
        "id",
    );
    let pts = route(s.doc().unwrap(), &connector(&s, k2)).unwrap();
    assert_eq!(pts.len(), 4);
    assert!(pts[1].y > pts[0].y && pts[2].y < pts[3].y, "{pts:?}");
    let b = s.doc().unwrap().world_bounds(k2).unwrap();
    assert!(b.y0 <= 0.0 + 1e-9 && b.y1 >= 100.0 - 1e-9, "{b:?}");
    // The label sits on the curve's middle (by length): here, by symmetry, the middle of the box.
    let label = crate::board::label_of(s.doc().unwrap(), k2).unwrap();
    let lc = s.doc().unwrap().world_bounds(label).unwrap().center();
    assert!((lc - b.center()).hypot() < 0.5, "{lc:?} vs {:?}", b.center());
    assert!(s.execute("board.setStyle", &json!({"ids": [k], "style": "wavy"})).is_err());
    s.execute("tool.connector", &json!({"style": "curved"})).unwrap();
    assert_eq!(s.tool.board.routing, Routing::Curved);
}

/// Whether `pts` (a right-angled route) runs through the inside of `r`.
fn through(pts: &[Point], r: Rect) -> bool {
    pts.windows(2).any(|w| {
        let (a, b) = (w[0], w[1]);
        a.x.min(b.x) < r.x1 - 0.5 && a.x.max(b.x) > r.x0 + 0.5 && a.y.min(b.y) < r.y1 - 0.5 && a.y.max(b.y) > r.y0 + 0.5
    })
}

#[test]
fn elbows_go_around_the_objects_they_join() {
    let (mut s, [a, b, _, k]) = three();
    // Out of a's left and b's right: the short way runs through both, so it goes over them.
    s.execute("board.connector", &json!({"from": a, "to": b, "fromSide": "left", "toSide": "right", "style": "elbow"})).unwrap();
    let k2 = s.selection()[0];
    let pts = drawn(&s, k2);
    let (ra, rb) = (Rect::new(0.0, 0.0, 100.0, 100.0), Rect::new(300.0, 0.0, 400.0, 100.0));
    assert!(!through(&pts, ra) && !through(&pts, rb), "{pts:?}");
    assert_eq!(pts.first().copied(), Some(Point::new(0.0, 50.0)));
    assert_eq!(pts.last().copied(), Some(Point::new(400.0, 50.0)));
    assert_eq!(pts.len(), 6, "out, over, down and in: {pts:?}");
    assert!(pts.iter().all(|p| p.y <= 50.0), "over the top, the nearer way round: {pts:?}");
    assert!(pts.windows(2).all(|w| w[0].x == w[1].x || w[0].y == w[1].y), "right angles only");
    // Facing each other, the short way stays.
    assert_eq!(drawn(&s, k), vec![Point::new(100.0, 50.0), Point::new(300.0, 50.0)]);
    // An end at the bottom of b leaving towards a above it: around b's side and up.
    s.execute("board.reattach", &json!({"id": k2, "end": "to", "target": b, "side": "bottom"})).unwrap();
    let pts = drawn(&s, k2);
    assert!(!through(&pts, ra) && !through(&pts, rb), "{pts:?}");
    assert_eq!(pts.last().copied(), Some(Point::new(350.0, 100.0)));
}

#[test]
fn reattach_survives_junk() {
    let (mut s, [_, _, _, k]) = three();
    for p in [
        json!({"id": k, "end": "to", "target": {"x": 1e308, "y": -1e308}}),
        json!({"id": k, "end": "from", "target": {"x": f64::MAX, "y": 0}, "side": 5}),
        json!({"id": [k], "end": ["to"], "target": [1]}),
        json!({"ids": "x", "end": null, "target": null}),
        json!({"id": k, "end": "to", "target": 99_999}),
    ] {
        let _ = s.execute("board.reattach", &p);
        let _ = s.execute("board.setStyle", &json!({"ids": [k], "style": "curved"}));
        let _ = s.execute("board.reattach", &p);
    }
    assert!(matches!(node(&s, k).board, Some(BoardItem::Connector(_))));
    assert!(drawn(&s, k).iter().all(|p| p.x.is_finite() && p.y.is_finite()));
}
