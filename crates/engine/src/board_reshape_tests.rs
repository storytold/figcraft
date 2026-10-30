//! Reshaping a board connector by hand: dragging an elbow's runs and a curve's middle, which keep
//! their shape as the objects move; the start and end runs gain an extra run to stay on their
//! ends; one undo step; saved with the file. Written from the public help on connectors.

use figcraft_doc::{BoardItem, ConnectorPath, End, NodeId, Routing, Side};
use figcraft_geom::Point;
use serde_json::{Value, json};

use crate::Session;
use crate::board::{route, selected_connector_handles};
use crate::board_tests::{board, connector, drawn, id, near};
use crate::tools::{PointerEvent, PointerKind};

const A: Point = Point::new(100.0, 50.0);
const B: Point = Point::new(300.0, 250.0);

/// Shapes a (at the origin) and b (down and to the right), 100 × 100, joined side to side.
fn joined(style: &str) -> (Session, NodeId, NodeId, NodeId) {
    let mut s = board();
    let mut shape = |x: f64, y: f64| id(&s.execute("board.shape", &json!({"x": x, "y": y, "width": 100, "height": 100})).unwrap(), "id");
    let (a, b) = (shape(0.0, 0.0), shape(300.0, 200.0));
    let k = id(&s.execute("board.connector", &json!({"from": a, "to": b, "fromSide": "right", "toSide": "left", "style": style})).unwrap(), "id");
    (s, a, b, k)
}

fn reshape(s: &mut Session, k: NodeId, portion: f64, x: f64, y: f64) -> crate::Result<Value> {
    s.execute("board.reshape", &json!({"id": k, "portion": portion, "x": x, "y": y}))
}

fn pt(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

/// Whether every step of `pts` runs straight across or down.
fn right_angled(pts: &[Point]) -> bool {
    pts.windows(2).all(|w| w[0].x == w[1].x || w[0].y == w[1].y)
}

/// The middle of connector `k`, a curve, as routed now.
fn the_curves_middle(s: &Session, k: NodeId) -> Point {
    let pts = route(s.doc().unwrap(), &connector(s, k)).unwrap();
    let [a, b, c, d] = pts.as_slice() else { panic!("not a curve: {pts:?}") };
    ((a.to_vec2() + (b.to_vec2() + c.to_vec2()) * 3.0 + d.to_vec2()) / 8.0).to_point()
}

#[test]
fn dragging_a_run_moves_it_and_it_stays_when_the_objects_move() {
    let (mut s, a, b, k) = joined("elbow");
    assert_eq!(drawn(&s, k), [A, pt(200.0, 50.0), pt(200.0, 250.0), B]);
    let undo = s.undo_depth().0;
    reshape(&mut s, k, 1.0, 250.0, 170.0).unwrap();
    assert_eq!(drawn(&s, k), [A, pt(250.0, 50.0), pt(250.0, 250.0), B], "the middle run moved across, only");
    assert_eq!(s.undo_depth().0, undo + 1, "one undo step");
    // The shape travels with what it joins: both objects moved, it moves with them.
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    s.execute("transform.nudge", &json!({"dx": 40, "dy": 10})).unwrap();
    let moved = |p: Point| pt(p.x + 40.0, p.y + 10.0);
    assert_eq!(drawn(&s, k), [A, pt(250.0, 50.0), pt(250.0, 250.0), B].map(moved));
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(drawn(&s, k), [A, pt(200.0, 50.0), pt(200.0, 250.0), B], "and undone");
    assert_eq!(connector(&s, k).path, None);
}

/// A reshaped path comes from the sides as they were: they stay, whatever the objects do next.
#[test]
fn a_reshaped_connector_keeps_the_sides_it_left_by() {
    let (mut s, a, b, _) = joined("elbow");
    let k = id(&s.execute("board.connector", &json!({"from": a, "to": b, "style": "elbow"})).unwrap(), "id");
    let handles = selected_connector_handles(s.doc().unwrap(), &[k], 1.0);
    let (portion, at) = handles[handles.len() / 2];
    reshape(&mut s, k, portion as f64, at.x + 30.0, at.y + 30.0).unwrap();
    let c = connector(&s, k);
    assert!(matches!(c.from, End::Node { node, side } if node == a && side != Side::Auto), "{c:?}");
    assert!(matches!(c.to, End::Node { node, side } if node == b && side != Side::Auto), "{c:?}");
}

/// "For the start and end portions of a connector, dragging can add an extra portion to the path."
#[test]
fn the_first_and_last_runs_gain_a_run_and_stay_on_their_ends() {
    let (mut s, _, _, k) = joined("elbow");
    reshape(&mut s, k, 0.0, 150.0, 120.0).unwrap();
    assert_eq!(drawn(&s, k), [A, pt(124.0, 50.0), pt(124.0, 120.0), pt(200.0, 120.0), pt(200.0, 250.0), B]);
    let last = drawn(&s, k).len() - 2;
    reshape(&mut s, k, last as f64, 250.0, 200.0).unwrap();
    let pts = drawn(&s, k);
    assert_eq!(pts, [A, pt(124.0, 50.0), pt(124.0, 120.0), pt(200.0, 120.0), pt(200.0, 200.0), pt(276.0, 200.0), pt(276.0, 250.0), B]);
    assert!(right_angled(&pts));
}

/// Two objects level with each other are joined by one run: it gains a run at both ends.
#[test]
fn a_lone_run_gains_a_run_at_each_end() {
    let mut s = board();
    let mut shape = |x: f64| id(&s.execute("board.shape", &json!({"x": x, "y": 0, "width": 100, "height": 100})).unwrap(), "id");
    let (a, b) = (shape(0.0), shape(300.0));
    let k = id(&s.execute("board.connector", &json!({"from": a, "to": b, "style": "elbow"})).unwrap(), "id");
    assert_eq!(drawn(&s, k).len(), 2);
    reshape(&mut s, k, 0.0, 200.0, -40.0).unwrap();
    let (l, r) = (pt(100.0, 50.0), pt(300.0, 50.0));
    assert_eq!(drawn(&s, k), [l, pt(124.0, 50.0), pt(124.0, -40.0), pt(276.0, -40.0), pt(276.0, 50.0), r]);
}

#[test]
fn a_curves_middle_goes_where_it_is_dragged_and_keeps_its_bow() {
    let (mut s, a, b, k) = joined("curved");
    assert_eq!(selected_connector_handles(s.doc().unwrap(), &[k], 1.0).len(), 1, "one handle");
    reshape(&mut s, k, 0.0, 300.0, 40.0).unwrap();
    let m = the_curves_middle(&s, k);
    assert!(near(m, pt(300.0, 40.0)), "{m:?}");
    let pts = route(s.doc().unwrap(), &connector(&s, k)).unwrap();
    assert_eq!((pts[0], pts[3]), (A, B), "the ends stay");
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    s.execute("transform.nudge", &json!({"dx": 20, "dy": 0})).unwrap();
    let m = the_curves_middle(&s, k);
    assert!(near(m, pt(320.0, 40.0)), "it moves with them: {m:?}");
    // Dragged again it goes there, not by as much again.
    reshape(&mut s, k, 0.0, 200.0, 300.0).unwrap();
    assert!(near(the_curves_middle(&s, k), pt(200.0, 300.0)));
}

#[test]
fn a_straight_connector_has_no_path_to_reshape() {
    let (mut s, _, _, k) = joined("straight");
    assert!(selected_connector_handles(s.doc().unwrap(), &[k], 1.0).is_empty());
    assert!(reshape(&mut s, k, 0.0, 10.0, 10.0).is_err());
}

#[test]
fn only_a_connectors_portions_move() {
    let (mut s, a, _, k) = joined("elbow");
    let before = connector(&s, k);
    for (id, portion) in [(k, 3.0), (k, -1.0), (k, 0.5), (k, 1e300), (a, 0.0)] {
        assert!(reshape(&mut s, id, portion, 10.0, 10.0).is_err(), "{portion}");
    }
    for junk in [json!({"id": k}), json!({"id": k, "portion": "one", "x": 0, "y": 0}), json!({"id": k, "portion": 1, "x": f64::MAX})] {
        assert!(s.execute("board.reshape", &junk).is_err(), "{junk}");
    }
    assert_eq!(connector(&s, k), before);
}

/// A first run dragged again and again adds runs each time, but not without end.
#[test]
fn a_path_has_a_limit_of_runs() {
    let (mut s, _, _, k) = joined("elbow");
    let tries = (0..80).map(|i| reshape(&mut s, k, 0.0, 150.0, 60.0 + 3.0 * f64::from(i))).filter(Result::is_ok).count();
    assert!((20..40).contains(&tries), "{tries}");
    assert!(drawn(&s, k).len() <= 66);
}

/// A path shaped by hand is for one style and for where the ends were.
#[test]
fn a_new_style_or_a_moved_end_goes_back_to_routing_itself() {
    let (mut s, _, b, k) = joined("elbow");
    reshape(&mut s, k, 1.0, 250.0, 170.0).unwrap();
    s.execute("board.setStyle", &json!({"ids": [k], "style": "curved"})).unwrap();
    assert_eq!(connector(&s, k).path, None);
    reshape(&mut s, k, 0.0, 300.0, 40.0).unwrap();
    assert!(connector(&s, k).path.is_some());
    s.execute("board.reattach", &json!({"id": k, "end": "to", "target": b, "side": "top"})).unwrap();
    assert_eq!(connector(&s, k).path, None);
}

#[test]
fn a_reshaped_path_is_saved_and_older_files_open() {
    let (mut s, _, _, k) = joined("elbow");
    reshape(&mut s, k, 1.0, 250.0, 170.0).unwrap();
    let c = connector(&s, k);
    assert_eq!(c.path, Some(ConnectorPath::Runs { across: true, at: vec![50.0] }));
    let saved = serde_json::to_value(BoardItem::Connector(c.clone())).unwrap();
    assert_eq!(serde_json::from_value::<BoardItem>(saved).unwrap(), BoardItem::Connector(c.clone()));
    // Saved without one (every older file), it routes itself; none isn't written out.
    let plain = figcraft_doc::Connector { path: None, ..c };
    let saved = serde_json::to_value(&plain).unwrap();
    assert!(saved.get("path").is_none(), "{saved}");
    assert_eq!(serde_json::from_value::<figcraft_doc::Connector>(saved).unwrap(), plain);
    assert!(route(s.doc().unwrap(), &plain).is_some());
}

#[test]
fn a_path_handle_drags_with_a_preview_and_one_undo_step() {
    let (mut s, _, _, k) = joined("elbow");
    s.execute("select.set", &json!({"ids": [k]})).unwrap();
    let ev = |kind, p: Point| PointerEvent::at(kind, p.x, p.y, 1.0);
    let handles = selected_connector_handles(s.doc().unwrap(), &[k], 1.0);
    assert_eq!(handles.iter().map(|h| h.0).collect::<Vec<_>>(), [0, 1, 2], "one for each run");
    let (portion, at) = handles[1];
    assert_eq!((portion, at), (1, pt(200.0, 150.0)));
    let undo = s.undo_depth().0;
    // A click changes nothing.
    s.pointer(&ev(PointerKind::Down, at)).unwrap();
    s.pointer(&ev(PointerKind::Up, at)).unwrap();
    assert_eq!((s.undo_depth().0, connector(&s, k).path), (undo, None));
    // Dragged, it shows as it will be; Escape puts it back.
    s.pointer(&ev(PointerKind::Down, at)).unwrap();
    s.pointer(&ev(PointerKind::Move, pt(240.0, 160.0))).unwrap();
    assert_eq!(drawn(&s, k)[1], pt(240.0, 50.0));
    s.escape().unwrap();
    assert_eq!(drawn(&s, k)[1], pt(200.0, 50.0));
    // Let go, it stays: one undo step, however far it went on the way.
    s.pointer(&ev(PointerKind::Down, at)).unwrap();
    s.pointer(&ev(PointerKind::Move, pt(240.0, 160.0))).unwrap();
    s.pointer(&ev(PointerKind::Move, pt(260.0, 90.0))).unwrap();
    s.pointer(&ev(PointerKind::Up, pt(260.0, 90.0))).unwrap();
    assert_eq!(drawn(&s, k), [A, pt(260.0, 50.0), pt(260.0, 250.0), B]);
    assert_eq!(s.undo_depth().0, undo + 1);
    s.undo().unwrap();
    assert_eq!(drawn(&s, k)[1], pt(200.0, 50.0));
}

/// Dev Mode only inspects, a locked connector stays, and a short run shows no handle (it would
/// sit on what's next to it) until zoomed in; the ends' handles win over a path handle.
#[test]
fn path_handles_show_where_they_can_be_used() {
    let (mut s, _, _, k) = joined("elbow");
    let shown = |s: &Session, zoom: f64| selected_connector_handles(s.doc().unwrap(), &[k], zoom).len();
    assert_eq!(shown(&s, 1.0), 3);
    assert_eq!(shown(&s, 0.2), 1, "only the long run, zoomed out");
    s.execute("node.set", &json!({"ids": [k], "props": {"locked": true}})).unwrap();
    assert_eq!(shown(&s, 1.0), 0);
    assert_eq!(connector(&s, k).routing, Routing::Elbow);
}
