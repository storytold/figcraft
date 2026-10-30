//! `board.addConnected` (#1597): the next sticky or shape on a side of the selected one, of the
//! same kind, size and colour, joined to it by a connector, in one undo step.

use figcraft_doc::{BoardItem, BoardShape, Connector, End, NodeId, Side};
use serde_json::json;

use crate::Session;
use crate::board::CONNECTED_GAP;
use crate::board_tests::{board, id, node};

fn connector(s: &Session, c: NodeId) -> Connector {
    match &node(s, c).board {
        Some(BoardItem::Connector(k)) => k.clone(),
        other => panic!("not a connector: {other:?}"),
    }
}

fn at(s: &Session, n: NodeId) -> (f64, f64) {
    let t = node(s, n).transform.translation();
    (t.x, t.y)
}

#[test]
fn a_shape_gets_the_next_one_joined_on_each_side() {
    let mut s = board();
    let a = id(&s.execute("board.shape", &json!({"shape": "diamond", "x": 0, "y": 0, "width": 200, "height": 120, "color": "blue"})).unwrap(), "id");
    let colour = |s: &Session, n: NodeId| node(s, node(s, n).children[0]).fills[0].solid_color();
    let g = CONNECTED_GAP;
    for (side, want, ends) in [
        ("right", (200.0 + g, 0.0), (Side::Right, Side::Left)),
        ("below", (0.0, 120.0 + g), (Side::Bottom, Side::Top)),
        ("left", (-200.0 - g, 0.0), (Side::Left, Side::Right)),
        ("above", (0.0, -120.0 - g), (Side::Top, Side::Bottom)),
    ] {
        let r = s.execute("board.addConnected", &json!({"id": a, "side": side})).unwrap();
        let (b, c) = (id(&r, "id"), id(&r, "connector"));
        assert_eq!(at(&s, b), want, "{side}");
        let nb = node(&s, b);
        assert_eq!((nb.width, nb.height), (200.0, 120.0), "{side}: the same size");
        assert!(matches!(nb.board, Some(BoardItem::Shape { shape: BoardShape::Diamond })), "{side}: the same shape");
        assert_eq!(colour(&s, b), colour(&s, a), "{side}: the same colour");
        let k = connector(&s, c);
        assert_eq!((k.from, k.to), (End::Node { node: a, side: ends.0 }, End::Node { node: b, side: ends.1 }), "{side}");
        assert_eq!(k.routing, s.tool.board.routing, "{side}: as the connector tool makes one");
        assert_eq!(s.selection(), vec![b], "{side}: the new one is selected");
    }
}

#[test]
fn a_sticky_gets_a_sticky_of_its_colour_and_typing_goes_on_in_it() {
    let mut s = board();
    let a = id(&s.execute("board.sticky", &json!({"x": 0, "y": 0, "color": "pink", "text": "First"})).unwrap(), "id");
    s.ui_requests.clear();
    // The selection is the default, and editing is asked for.
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    let r = s.execute("board.addConnected", &json!({"side": "below", "edit": true})).unwrap();
    let b = id(&r, "id");
    assert!(matches!(node(&s, b).board, Some(BoardItem::Sticky { .. })));
    assert_eq!(node(&s, b).fills[0].solid_color(), node(&s, a).fills[0].solid_color());
    assert_eq!(at(&s, b).1, node(&s, a).height + CONNECTED_GAP);
    assert!(s.ui_requests.iter().any(|q| q.kind == "board.editText" && q.data["id"] == json!(b)));
}

#[test]
fn it_is_one_undo_step_and_refuses_what_it_cant_join() {
    let mut s = board();
    let a = id(&s.execute("board.shape", &json!({"shape": "ellipse", "x": 0, "y": 0})).unwrap(), "id");
    let before = s.doc().unwrap().ids().count();
    let r = s.execute("board.addConnected", &json!({"id": a})).unwrap();
    assert_eq!(at(&s, id(&r, "id")).0, 160.0 + CONNECTED_GAP, "right by default");
    assert!(s.doc().unwrap().ids().count() > before);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().ids().count(), before, "the shape and its connector go together");
    // Not a sticky or shape, not a side, nothing selected.
    let c = id(&s.execute("board.stamp", &json!({"x": 400, "y": 0, "stamp": "star"})).unwrap(), "id");
    assert!(s.execute("board.addConnected", &json!({"id": c})).is_err());
    assert!(s.execute("board.addConnected", &json!({"id": a, "side": "diagonal"})).is_err());
    s.execute("select.set", &json!({"ids": []})).unwrap();
    assert!(s.execute("board.addConnected", &json!({})).is_err());
    // Not on a design file.
    let mut d = Session::new();
    assert!(d.execute("board.addConnected", &json!({})).is_err());
}

#[test]
fn the_next_sticky_shortcut_joins_the_next_shape_and_leaves_stickies_as_they_were() {
    let mut s = board();
    let a = id(&s.execute("board.shape", &json!({"shape": "rectangle", "x": 0, "y": 0, "width": 200, "height": 120})).unwrap(), "id");
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    let undo = s.ds().unwrap().history.undo.len();
    // Shift+Cmd/Ctrl+Enter on a shape: the next shape on the right, joined by a connector (#1597).
    let r = s.execute("board.nextSticky", &json!({})).unwrap();
    let (b, c) = (id(&r, "id"), id(&r, "connector"));
    assert!(matches!(node(&s, b).board, Some(BoardItem::Shape { shape: BoardShape::Rectangle })));
    assert_eq!(at(&s, b), (200.0 + CONNECTED_GAP, 0.0));
    assert_eq!(connector(&s, c).from, End::Node { node: a, side: Side::Right });
    assert_eq!(s.ds().unwrap().history.undo.len(), undo + 1, "one undo step");
    // Another side, as the command takes one.
    let r = s.execute("board.nextSticky", &json!({"id": a, "side": "below"})).unwrap();
    assert_eq!(at(&s, id(&r, "id")), (0.0, 120.0 + CONNECTED_GAP));
    // A sticky still gets a sticky beside it, without a connector.
    let st = id(&s.execute("board.sticky", &json!({"x": 1000, "y": 0})).unwrap(), "id");
    let r = s.execute("board.nextSticky", &json!({"id": st})).unwrap();
    assert!(r.get("connector").is_none(), "{r}");
    assert!(matches!(node(&s, id(&r, "id")).board, Some(BoardItem::Sticky { .. })));
}
