//! Boards, continued: copies and pastes (within a file and from another), routes with named sides
//! and free points, connectors inside moving sections, styles and labels with their refusals, and
//! the shape, stamp, highlighter and connector tools with double-clicks.

use figcraft_doc::{BoardItem, BoardShape, End, NodeId, Side, Stamp, StrokeCap, board_color};
use figcraft_geom::Point;
use serde_json::json;

use crate::Session;
use crate::board::route;
use crate::board_tests::{board, connector, drawn, id, near, node};
use crate::tools::{PointerEvent, PointerKind, ToolId};

/// The connector among `ids`, if any.
fn connector_in(s: &Session, ids: &[NodeId]) -> Option<figcraft_doc::Connector> {
    let doc = s.doc().ok()?;
    ids.iter().find_map(|i| match &doc.node(*i)?.board {
        Some(BoardItem::Connector(k)) => Some(k.clone()),
        _ => None,
    })
}

#[test]
fn pasting_into_another_board_detaches_what_was_left_behind() {
    let mut s = board();
    let a = id(&s.execute("board.sticky", &json!({"x": 0, "y": 0})).unwrap(), "id");
    let b = id(&s.execute("board.sticky", &json!({"x": 400, "y": 0})).unwrap(), "id");
    let r = s.execute("board.connector", &json!({"from": a, "to": b, "style": "straight", "label": "x"})).unwrap();
    let (c, label) = (id(&r, "id"), id(&r, "label"));
    let first = s.active;
    // A second board whose own ids are the same small numbers as the first's.
    let second = s.execute("file.newBoard", &json!({})).unwrap()["index"].clone();
    for x in [0, 300, 600, 900] {
        s.execute("board.sticky", &json!({"x": x, "y": 500})).unwrap();
    }
    let others: Vec<NodeId> = s.doc().unwrap().nodes().map(|n| n.id).collect();
    s.execute("doc.switch", &json!({"index": first})).unwrap();
    s.execute("select.set", &json!({"ids": [a, c, label]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    s.execute("doc.switch", &json!({"index": second})).unwrap();
    s.execute("edit.paste", &json!({})).unwrap();
    let doc = s.doc().unwrap();
    let pasted = doc.nodes().find(|n| matches!(n.board, Some(BoardItem::Connector(_))) && !others.contains(&n.id)).unwrap().clone();
    let k = connector(&s, pasted.id);
    let from = k.from.node().unwrap();
    assert!(!others.contains(&from) && matches!(doc.node(from).unwrap().board, Some(BoardItem::Sticky { .. })), "joins the pasted sticky");
    let to = drawn(&s, pasted.id).last().copied().unwrap();
    assert!(matches!(k.to, End::Point { x, y } if near(Point::new(x, y), to)), "the end left behind stays where it is drawn: {k:?}");
    let pasted_label = crate::board::label_of(doc, pasted.id).expect("the label came with its connector");
    assert!(!others.contains(&pasted_label));
    // A label copied without its connector doesn't come.
    s.execute("doc.switch", &json!({"index": first})).unwrap();
    s.execute("select.set", &json!({"ids": [label]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    s.execute("doc.switch", &json!({"index": second})).unwrap();
    let labels = |s: &Session| s.doc().unwrap().nodes().filter(|n| matches!(n.board, Some(BoardItem::Label { .. }))).count();
    let before = labels(&s);
    s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!(labels(&s), before);
    // Within its own file, a connector pasted alone stays joined to the same layers.
    s.execute("doc.switch", &json!({"index": first})).unwrap();
    s.execute("select.set", &json!({"ids": [c]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    s.execute("edit.paste", &json!({})).unwrap();
    let k = connector_in(&s, &s.selection()).unwrap();
    assert_eq!((k.from.node(), k.to.node()), (Some(a), Some(b)));
}

#[test]
fn duplicating_and_alt_dragging_keep_connections_inside_copied_sections() {
    let mut s = board();
    let sec = id(&s.execute("node.create", &json!({"type": "section", "x": -50, "y": -50, "width": 400, "height": 400})).unwrap(), "id");
    let a = id(&s.execute("board.sticky", &json!({"x": 50, "y": 50, "parent": sec})).unwrap(), "id");
    let b = id(&s.execute("board.sticky", &json!({"x": 600, "y": 0})).unwrap(), "id");
    let c = id(&s.execute("board.connector", &json!({"from": a, "to": b})).unwrap(), "id");
    s.execute("edit.duplicate", &json!({"ids": [sec, c]})).unwrap();
    let copies = s.selection();
    let from = connector_in(&s, &copies).unwrap().from.node().unwrap();
    let doc = s.doc().unwrap();
    assert!(from != a && copies.iter().any(|r| doc.is_ancestor(*r, from)), "the copy joins the sticky inside the copied section");
    // Alt-drag copies the same way.
    s.execute("select.set", &json!({"ids": [b, c]})).unwrap();
    let press = |k, x| PointerEvent { alt: true, ..PointerEvent::at(k, x, 100.0, 1.0) };
    for (k, x) in [(PointerKind::Down, 700.0), (PointerKind::Move, 760.0), (PointerKind::Move, 800.0), (PointerKind::Up, 800.0)] {
        s.pointer(&press(k, x)).unwrap();
    }
    let copies = s.selection();
    assert_eq!(copies.len(), 2);
    let k = connector_in(&s, &copies).unwrap();
    assert!(copies.contains(&k.to.node().unwrap()), "{k:?}");
}

#[test]
fn routes_take_named_sides_and_free_points() {
    let mut s = board();
    let a = id(&s.execute("board.shape", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap(), "id");
    let b = id(&s.execute("board.shape", &json!({"x": 300, "y": 300, "width": 100, "height": 100})).unwrap(), "id");
    let doc = s.doc().unwrap();
    let elbow = |from, to| route(doc, &figcraft_doc::Connector { from, to, routing: figcraft_doc::Routing::Elbow, path: None }).unwrap();
    let side = |n, side| End::Node { node: n, side, at: None };
    let p = |x, y| End::Point { x, y };
    let pt = Point::new;
    // Side to side: out, across halfway, in.
    assert_eq!(elbow(side(a, Side::Right), side(b, Side::Left)), vec![pt(100.0, 50.0), pt(200.0, 50.0), pt(200.0, 350.0), pt(300.0, 350.0)]);
    // A side and a top or bottom: one turn.
    assert_eq!(elbow(side(a, Side::Right), side(b, Side::Top)), vec![pt(100.0, 50.0), pt(350.0, 50.0), pt(350.0, 300.0)]);
    assert_eq!(elbow(side(a, Side::Bottom), side(b, Side::Left)), vec![pt(50.0, 100.0), pt(50.0, 350.0), pt(300.0, 350.0)]);
    // Two free points: the longer way first; in a line, no turns.
    assert_eq!(elbow(p(0.0, 0.0), p(100.0, 40.0)), vec![pt(0.0, 0.0), pt(50.0, 0.0), pt(50.0, 40.0), pt(100.0, 40.0)]);
    assert_eq!(elbow(p(0.0, 0.0), p(100.0, 0.0)), vec![pt(0.0, 0.0), pt(100.0, 0.0)]);
    // An automatic side faces the other end.
    let straight =
        route(doc, &figcraft_doc::Connector { from: side(a, Side::Auto), to: p(50.0, 500.0), routing: figcraft_doc::Routing::Straight, path: None });
    assert_eq!(straight.unwrap(), vec![pt(50.0, 100.0), pt(50.0, 500.0)]);
    assert_eq!(Side::from_id("TOP"), Some(Side::Top));
    assert_eq!(Side::from_id("inside"), None);
    // The same place twice is refused, as are a side that doesn't exist and a point without numbers.
    assert!(s.execute("board.connector", &json!({"from": {"x": 5, "y": 5}, "to": {"x": 5, "y": 5}})).is_err());
    assert!(s.execute("board.connector", &json!({"from": a, "to": b, "fromSide": "inside"})).is_err());
    assert!(s.execute("board.connector", &json!({"from": a, "to": {"x": "a"}})).is_err());
    assert!(s.execute("board.connector", &json!({"from": a})).is_err());
}

#[test]
fn a_connector_follows_a_layer_whose_section_moves_and_ignores_unrelated_edits() {
    let mut s = board();
    let sec = id(&s.execute("node.create", &json!({"type": "section", "x": 0, "y": 0, "width": 300, "height": 300})).unwrap(), "id");
    let a = id(&s.execute("board.sticky", &json!({"x": 50, "y": 50, "parent": sec})).unwrap(), "id");
    let b = id(&s.execute("board.stamp", &json!({"x": 800, "y": 150})).unwrap(), "id");
    let c = id(&s.execute("board.connector", &json!({"from": a, "to": b, "style": "straight"})).unwrap(), "id");
    assert!(near(drawn(&s, c)[0], Point::new(250.0, 150.0)));
    // Only the section is edited; the sticky inside it moves with it, and so does the connector.
    s.execute("transform.move", &json!({"ids": [sec], "dx": 0, "dy": 100})).unwrap();
    assert!(near(drawn(&s, c)[0], Point::new(250.0, 250.0)), "{:?}", drawn(&s, c));
    // An edit that touches neither end leaves the connector as it was.
    let before = s.doc().unwrap().node_arc(c).cloned().unwrap();
    s.execute("board.sticky", &json!({"x": 0, "y": 900})).unwrap();
    assert!(std::sync::Arc::ptr_eq(&before, s.doc().unwrap().node_arc(c).unwrap()));
}

#[test]
fn styles_labels_and_their_refusals() {
    let mut s = board();
    let a = id(&s.execute("board.sticky", &json!({"x": 0, "y": 0, "author": false})).unwrap(), "id");
    assert_eq!(node(&s, a).children.len(), 1, "no author line");
    let b = id(&s.execute("board.sticky", &json!({"x": 400, "y": 0, "author": "  Lin  "})).unwrap(), "id");
    assert!(matches!(node(&s, b).board, Some(BoardItem::Sticky { ref author }) if author == "Lin"));
    let c = id(&s.execute("board.connector", &json!({"from": a, "to": b, "startCap": "circleFilled", "endCap": "none"})).unwrap(), "id");
    assert_eq!((node(&s, c).stroke.cap_start, node(&s, c).stroke.cap_end), (StrokeCap::CircleFilled, StrokeCap::None));
    // Labels: set, change, size, remove; named by the label itself too.
    let l = id(&s.execute("board.label", &json!({"id": c, "text": "first"})).unwrap(), "label");
    assert_eq!(id(&s.execute("board.label", &json!({"id": l, "text": "second"})).unwrap(), "label"), l, "the same label, new text");
    let text = crate::board::text_of(s.doc().unwrap(), l).unwrap();
    assert_eq!(node(&s, text).kind.text().unwrap().characters, "second");
    s.execute("board.setStyle", &json!({"ids": [l], "fontSize": 36})).unwrap();
    assert_eq!(node(&s, text).kind.text().unwrap().style.font_size, 36.0);
    assert!(s.execute("board.label", &json!({"id": c, "text": ""})).unwrap()["label"].is_null());
    assert!(!s.doc().unwrap().contains(l));
    // What doesn't take a style or colour says so.
    assert!(s.execute("board.setStyle", &json!({"ids": [a], "fontSize": 30})).is_err(), "stickies size their own text");
    assert!(s.execute("board.setStyle", &json!({"ids": [c], "fontSize": 0})).is_err());
    let st = id(&s.execute("board.stamp", &json!({"x": 0, "y": 0, "stamp": "check"})).unwrap(), "id");
    assert!(s.execute("board.setColor", &json!({"ids": [st], "color": "blue"})).is_err());
    assert!(s.execute("board.setColor", &json!({"ids": [a]})).is_err());
    assert!(s.execute("board.label", &json!({"id": a, "text": "x"})).is_err());
    // A palette colour on a line draws in its deeper shade; a hex colour as given.
    s.execute("board.setColor", &json!({"ids": [c], "color": "blue"})).unwrap();
    assert!(node(&s, c).strokes[0].solid_color().is_some() && node(&s, c).strokes[0].solid_color() != board_color("blue"));
    s.execute("board.setColor", &json!({"ids": [c], "color": "#123456"})).unwrap();
    assert_eq!(node(&s, c).strokes[0].solid_color().map(|c| c.to_hex()), Some("123456".into()));
    // Next stickies on each side, and refusals.
    for (side, dx, dy) in [("below", 0.0, 224.0), ("left", -224.0, 0.0), ("above", 0.0, -224.0)] {
        let n = id(&s.execute("board.nextSticky", &json!({"id": a, "side": side})).unwrap(), "id");
        let t = node(&s, n).transform.translation();
        assert_eq!((t.x, t.y), (dx, dy), "{side}");
    }
    assert!(s.execute("board.nextSticky", &json!({"id": a, "side": "diagonal"})).is_err());
    assert!(s.execute("board.nextSticky", &json!({"id": c})).is_err());
    assert!(s.execute("board.ink", &json!({})).is_err());
    assert!(s.execute("board.sticky", &json!({"x": 0, "y": 0, "parent": c})).is_err(), "a connector holds no layers");
}

#[test]
fn the_shape_stamp_highlighter_and_connector_tools_and_double_clicks() {
    let mut s = board();
    let click = |s: &mut Session, x, y| {
        s.pointer(&PointerEvent::at(PointerKind::Down, x, y, 1.0)).unwrap();
        s.pointer(&PointerEvent::at(PointerKind::Up, x, y, 1.0)).unwrap();
    };
    s.execute("tool.shape", &json!({"shape": "roundedRectangle"})).unwrap();
    click(&mut s, 100.0, 100.0);
    let sh = s.selection()[0];
    assert_eq!(node(&s, sh).board, Some(BoardItem::Shape { shape: BoardShape::RoundedRectangle }));
    assert_eq!(s.tool.tool, ToolId::Move, "back to Move after placing");
    s.execute("tool.stamp", &json!({"stamp": "question"})).unwrap();
    click(&mut s, 400.0, 100.0);
    click(&mut s, 450.0, 100.0);
    let stamps = s.doc().unwrap().nodes().filter(|n| matches!(n.board, Some(BoardItem::Stamp { stamp: Stamp::Question, .. }))).count();
    assert_eq!(stamps, 2);
    assert_eq!(s.tool.tool, ToolId::Stamp, "stamps go on one after another");
    assert!(s.execute("tool.stamp", &json!({"stamp": "skull"})).is_err());
    s.execute("tool.highlighter", &json!({"color": "pink"})).unwrap();
    assert_eq!(Some(s.tool.board.highlighter), board_color("pink"));
    s.execute("tool.marker", &json!({"color": "#336699"})).unwrap();
    assert_eq!(s.tool.board.marker.to_hex(), "336699");
    // A connector drag that is only a click, or from an object to itself, makes nothing.
    s.execute("tool.connector", &json!({"style": "straight"})).unwrap();
    click(&mut s, 100.0, 100.0);
    s.pointer(&PointerEvent::at(PointerKind::Down, 60.0, 120.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, 100.0, 140.0, 1.0)).unwrap();
    assert!(!s.doc().unwrap().nodes().any(|n| matches!(n.board, Some(BoardItem::Connector(_)))), "nothing previewed: none would be made");
    s.pointer(&PointerEvent::at(PointerKind::Up, 100.0, 140.0, 1.0)).unwrap();
    assert!(!s.doc().unwrap().nodes().any(|n| matches!(n.board, Some(BoardItem::Connector(_)))));
    // From the shape out to empty canvas: a connector to a free point.
    s.pointer(&PointerEvent::at(PointerKind::Down, 80.0, 140.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, 80.0, 600.0, 1.0)).unwrap();
    let c = s.doc().unwrap().nodes().find(|n| matches!(n.board, Some(BoardItem::Connector(_)))).map(|n| n.id).unwrap();
    assert!(matches!(connector(&s, c).to, End::Point { .. }));
    // Double-clicks: on the shape its text, on the connector a new label, both typed into.
    s.execute("tool.move", &json!({})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::DoubleClick, 80.0, 140.0, 1.0)).unwrap();
    let te = s.ds().unwrap().text_edit.clone().expect("typing in the shape");
    assert_eq!(crate::board::owner_of_text(s.doc().unwrap(), te.id), Some(sh));
    crate::textedit::end(&mut s).unwrap();
    let line = drawn(&s, c);
    let m = line[0].midpoint(line[1]);
    s.pointer(&PointerEvent::at(PointerKind::DoubleClick, m.x, m.y, 1.0)).unwrap();
    let label = crate::board::label_of(s.doc().unwrap(), c).expect("a label was added");
    let te = s.ds().unwrap().text_edit.clone().expect("typing in the label");
    assert_eq!(crate::board::owner_of_text(s.doc().unwrap(), te.id), Some(label));
}

#[test]
fn an_ink_stroke_ends_where_the_button_comes_up() {
    let mut s = board();
    s.execute("tool.marker", &json!({})).unwrap();
    // Down and up with no move between: the stroke still runs from the press to the release.
    s.pointer(&PointerEvent::at(PointerKind::Down, 0.0, 0.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, 150.0, 0.0, 1.0)).unwrap();
    let doc = s.doc().unwrap();
    let ink = doc.nodes().find(|n| matches!(n.board, Some(BoardItem::Ink))).map(|n| n.id).expect("a stroke");
    let b = doc.world_bounds(ink).unwrap();
    assert!(b.x0.abs() < 6.0 && (b.x1 - 150.0).abs() < 6.0, "from 0 to 150 (stroke width aside): {b:?}");
}

#[test]
fn a_second_sticky_ends_the_first_ones_typing_and_undoes_alone() {
    let mut s = board();
    s.execute("tool.sticky", &json!({})).unwrap();
    s.prefs.keep_tool = true;
    s.pointer(&PointerEvent::at(PointerKind::Down, 100.0, 100.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, 100.0, 100.0, 1.0)).unwrap();
    let text = s.ds().unwrap().text_edit.as_ref().expect("typing in the new sticky").id;
    crate::textedit::insert(&mut s, "first note").unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Down, 500.0, 100.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, 500.0, 100.0, 1.0)).unwrap();
    let stickies = |s: &Session| s.doc().unwrap().nodes().filter(|n| matches!(n.board, Some(BoardItem::Sticky { .. }))).count();
    let typed = |s: &Session| s.doc().unwrap().node(text).unwrap().kind.text().unwrap().characters.clone();
    assert_eq!(stickies(&s), 2);
    crate::textedit::end(&mut s).unwrap();
    s.undo().unwrap();
    assert_eq!(stickies(&s), 1, "the second sticky is its own undo step");
    assert_eq!(typed(&s), "first note", "the first sticky keeps what was typed in it");
}

#[test]
fn a_board_tool_left_on_presses_as_move_in_a_design_file() {
    let mut s = board();
    s.execute("tool.sticky", &json!({})).unwrap();
    s.execute("file.new", &json!({})).unwrap();
    let r = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap(), "id");
    s.execute("select.none", &json!({})).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Down, 50.0, 50.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, 50.0, 50.0, 1.0)).unwrap();
    assert_eq!(s.tool.tool, ToolId::Move);
    assert_eq!(s.selection(), vec![r], "the press selected as Move does");
}
