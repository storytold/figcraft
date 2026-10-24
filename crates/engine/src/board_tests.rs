//! Boards: the file kind, sticky notes, shapes with text, connectors that follow what they join
//! (locally, through undo and in live sessions), stamps, ink and the eraser, and refusals.

use figcraft_doc::{
    BOARD_PAGE, BoardItem, BoardShape, Connector, End, FileKind, Node, NodeId, NodeKind, Routing, Side, Stamp, StrokeCap, board_color,
};
use figcraft_geom::Point;
use serde_json::{Value, json};

use crate::board::{route, sticky_font_size};
use crate::collab_live_tests::Room;
use crate::tools::{PointerEvent, PointerKind, ToolId};
use crate::{EngineError, Session};

pub(crate) fn board() -> Session {
    let mut s = Session::new();
    s.execute("file.newBoard", &json!({"name": "Ideas"})).unwrap();
    s
}

pub(crate) fn id(v: &Value, key: &str) -> NodeId {
    v[key].as_u64().map(NodeId).unwrap_or_else(|| panic!("no {key} in {v}"))
}

pub(crate) fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().node(id).unwrap().clone()
}

pub(crate) fn connector(s: &Session, id: NodeId) -> Connector {
    match node(s, id).board {
        Some(BoardItem::Connector(c)) => c,
        other => panic!("not a connector: {other:?}"),
    }
}

/// Connector `id`'s points on the page.
pub(crate) fn drawn(s: &Session, id: NodeId) -> Vec<Point> {
    let doc = s.doc().unwrap();
    let NodeKind::Vector { network } = &doc.node(id).unwrap().kind else { panic!("not a vector") };
    let w = doc.world_transform(id);
    network.vertices.iter().map(|v| w * v.point()).collect()
}

pub(crate) fn near(a: Point, b: Point) -> bool {
    (a - b).hypot() < 1e-6
}

#[test]
fn a_board_is_a_kind_of_file_that_saves_and_reopens() {
    let s = board();
    let doc = s.doc().unwrap();
    assert_eq!(doc.kind, FileKind::Board);
    assert!(matches!(doc.node(doc.pages[0]).unwrap().kind, NodeKind::Page { background, .. } if background == BOARD_PAGE));
    let (back, _) = figcraft_format::from_json(&figcraft_format::to_json(doc).unwrap().to_string()).unwrap();
    assert_eq!(back.kind, FileKind::Board);
    // Files from before boards (no kind) are design files, and design files don't write one.
    let mut d = Session::new();
    d.execute("file.new", &json!({})).unwrap();
    let json = figcraft_format::to_json(d.doc().unwrap()).unwrap();
    assert!(json.get("kind").is_none());
    assert_eq!(figcraft_format::from_json(&json.to_string()).unwrap().0.kind, FileKind::Design);
    assert_eq!(crate::inspect::document(&s, 0, false)["kind"], "board", "inspect shows the kind");
}

#[test]
fn board_commands_refuse_on_design_files_and_prototyping_refuses_on_boards() {
    let mut d = Session::new();
    d.execute("file.new", &json!({})).unwrap();
    for (cmd, p) in [("board.sticky", json!({"x": 0, "y": 0})), ("tool.sticky", json!({})), ("tool.set", json!({"tool": "marker"}))] {
        assert!(matches!(d.execute(cmd, &p), Err(EngineError::Disabled(..))), "{cmd} on a design file");
    }
    let mut s = board();
    for cmd in ["view.devMode", "view.prototype", "proto.present", "proto.flow.add", "dev.focus"] {
        let e = s.execute(cmd, &json!({})).unwrap_err();
        assert!(e.to_string().contains("boards have no prototypes"), "{cmd}: {e}");
    }
}

#[test]
fn a_sticky_is_a_square_frame_whose_note_shrinks_as_it_grows() {
    let mut s = board();
    s.prefs.user_name = "Ada".into();
    let r = s.execute("board.sticky", &json!({"x": 10, "y": 20, "text": "Hi", "color": "blue"})).unwrap();
    let (st, note) = (id(&r, "id"), id(&r, "text"));
    let n = node(&s, st);
    assert!(n.is_board_atom() && matches!(&n.board, Some(BoardItem::Sticky { author }) if author == "Ada"));
    assert_eq!((n.width, n.height), (200.0, 200.0));
    assert_eq!(n.fills[0].solid_color(), board_color("blue"));
    assert!(n.layout.is_some() && !n.effects.is_empty());
    assert_eq!(n.children.len(), 2, "the note and its author");
    assert_eq!(node(&s, note).kind.text().unwrap().style.font_size, 24.0);
    // A click on the note picks the sticky whole.
    let doc = s.doc().unwrap();
    assert_eq!(figcraft_doc::geometry::select_target(doc, note, &[], false), st);
    // A long note takes a smaller size; a very long one grows the sticky past its square.
    let long = "word ".repeat(120);
    s.execute("node.set", &json!({"id": note, "props": {"characters": long}})).unwrap();
    assert_eq!(node(&s, note).kind.text().unwrap().style.font_size, sticky_font_size(&long));
    assert!(sticky_font_size(&long) < 16.0);
    assert!(node(&s, st).width == 200.0 && node(&s, st).height >= 200.0);
    // Undo puts the short note and its size back.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node(&s, note).kind.text().unwrap().style.font_size, 24.0);
}

#[test]
fn next_sticky_goes_beside_in_the_same_colour_and_emptied_notes_stay() {
    let mut s = board();
    let a = id(&s.execute("board.sticky", &json!({"x": 0, "y": 0, "color": "pink"})).unwrap(), "id");
    let r = s.execute("board.nextSticky", &json!({})).unwrap();
    let b = id(&r, "id");
    let (na, nb) = (node(&s, a), node(&s, b));
    assert_eq!(nb.transform.translation().x, na.width + 24.0);
    assert_eq!(nb.fills, na.fills);
    assert_eq!(s.selection(), vec![b]);
    // Typing in it and emptying it again keeps its note; ending selects the sticky.
    let note = id(&r, "text");
    crate::board::edit_text(&mut s, b).unwrap();
    crate::textedit::insert(&mut s, "x").unwrap();
    crate::textedit::key(&mut s, "Backspace", false, false, false).unwrap();
    crate::textedit::end(&mut s).unwrap();
    assert!(s.doc().unwrap().contains(note));
    assert_eq!(s.selection(), vec![b]);
}

#[test]
fn shapes_carry_text_and_change_outline() {
    let mut s = board();
    let r = s.execute("board.shape", &json!({"shape": "diamond", "x": 0, "y": 0, "text": "Decide", "color": "green"})).unwrap();
    let sh = id(&r, "id");
    let n = node(&s, sh);
    assert_eq!(n.board, Some(BoardItem::Shape { shape: BoardShape::Diamond }));
    assert_eq!(n.children.len(), 2);
    assert!(matches!(node(&s, n.children[0]).kind, NodeKind::Vector { .. }));
    s.execute("board.setStyle", &json!({"ids": [sh], "shape": "speechBubble", "fontSize": 24})).unwrap();
    assert_eq!(node(&s, sh).board, Some(BoardItem::Shape { shape: BoardShape::SpeechBubble }));
    assert_eq!(node(&s, id(&r, "text")).kind.text().unwrap().style.font_size, 24.0);
    s.execute("board.setColor", &json!({"ids": [sh], "color": "#FF0000"})).unwrap();
    assert_eq!(node(&s, n.children[0]).fills[0].solid_color().map(|c| c.to_hex()), Some("FF0000".into()));
    assert!(s.execute("board.shape", &json!({"shape": "blob", "x": 0, "y": 0})).is_err());
}

#[test]
fn connectors_follow_what_they_join_and_undo_with_it() {
    let mut s = board();
    let a = id(&s.execute("board.sticky", &json!({"x": 0, "y": 0})).unwrap(), "id");
    let b = id(&s.execute("board.sticky", &json!({"x": 400, "y": 0})).unwrap(), "id");
    let c = id(&s.execute("board.connector", &json!({"from": a, "to": b, "style": "straight", "label": "then"})).unwrap(), "id");
    let pts = drawn(&s, c);
    assert!(near(pts[0], Point::new(200.0, 100.0)) && near(pts[1], Point::new(400.0, 100.0)), "{pts:?}");
    assert_eq!(node(&s, c).stroke.cap_end, StrokeCap::TriangleArrow);
    // Its label sits at the middle.
    let label = crate::board::label_of(s.doc().unwrap(), c).unwrap();
    let lb = s.doc().unwrap().world_bounds(label).unwrap();
    assert!(near(lb.center(), Point::new(300.0, 100.0)), "{lb:?}");
    // Move one end: the connector re-routes (elbow now, out of the facing sides).
    s.execute("board.setStyle", &json!({"ids": [c], "style": "elbow"})).unwrap();
    s.execute("transform.move", &json!({"ids": [b], "dx": 0, "dy": 300})).unwrap();
    let pts = drawn(&s, c);
    assert!(near(pts[0], Point::new(200.0, 100.0)) && near(*pts.last().unwrap(), Point::new(400.0, 400.0)), "{pts:?}");
    assert!(pts.windows(2).all(|w| w[0].x == w[1].x || w[0].y == w[1].y), "right angles: {pts:?}");
    // Undo takes the move back and the route with it.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(near(*drawn(&s, c).last().unwrap(), Point::new(400.0, 100.0)));
    // A deleted end stays where it was, as a point.
    s.execute("edit.delete", &json!({"ids": [b]})).unwrap();
    assert!(matches!(connector(&s, c).to, End::Point { .. }));
    assert!(near(*drawn(&s, c).last().unwrap(), Point::new(400.0, 100.0)));
    // A connector moved on its own takes its free end along; the attached one stays.
    s.execute("transform.move", &json!({"ids": [c], "dx": 0, "dy": 50})).unwrap();
    assert!(matches!(connector(&s, c).to, End::Point { x, y } if x == 400.0 && y == 150.0), "{:?}", connector(&s, c));
    assert!(near(drawn(&s, c)[0], Point::new(200.0, 100.0)));
    // Deleting the connector takes its label.
    s.execute("edit.delete", &json!({"ids": [c]})).unwrap();
    assert!(!s.doc().unwrap().contains(label));
    assert!(s.execute("board.connector", &json!({"from": a, "to": a})).is_err());
}

#[test]
fn routes_are_the_same_for_the_same_document() {
    let mut s = board();
    let a = id(&s.execute("board.shape", &json!({"x": 0, "y": 0})).unwrap(), "id");
    let c = Connector { from: End::Node { node: a, side: Side::Bottom }, to: End::Point { x: 300.0, y: 500.0 }, routing: Routing::Elbow };
    let doc = s.doc().unwrap();
    let r = route(doc, &c).unwrap();
    assert_eq!(r, route(&doc.clone(), &c).unwrap());
    // Down out of the bottom, across halfway, down into the point.
    assert_eq!(r, vec![Point::new(80.0, 160.0), Point::new(80.0, 330.0), Point::new(300.0, 330.0), Point::new(300.0, 500.0)]);
}

#[test]
fn stamps_and_ink_and_the_eraser() {
    let mut s = board();
    s.prefs.user_name = "Grace".into();
    let st = id(&s.execute("board.stamp", &json!({"x": 100, "y": 100, "stamp": "heart"})).unwrap(), "id");
    let n = node(&s, st);
    assert_eq!(n.board, Some(BoardItem::Stamp { stamp: Stamp::Heart, author: "Grace".into() }));
    assert_eq!(s.doc().unwrap().world_bounds(st).unwrap().center(), Point::new(100.0, 100.0));
    assert!(!n.children.is_empty());
    // Marker and highlighter strokes through the tools; the eraser takes what it touches.
    let draw = |s: &mut Session, tool: &str, y: f64| {
        s.execute("tool.set", &json!({"tool": tool})).unwrap();
        for (k, x) in [(PointerKind::Down, 0.0), (PointerKind::Move, 50.0), (PointerKind::Move, 100.0), (PointerKind::Up, 150.0)] {
            s.pointer(&PointerEvent::at(k, x, y, 1.0)).unwrap();
        }
    };
    draw(&mut s, "marker", 300.0);
    draw(&mut s, "highlighter", 400.0);
    let inks: Vec<Node> = s.doc().unwrap().nodes().filter(|n| n.board == Some(BoardItem::Ink)).cloned().collect();
    assert_eq!(inks.len(), 2);
    assert!(inks.iter().any(|n| n.opacity < 1.0 && n.stroke.weight > 10.0), "a highlighter stroke");
    let undo = s.undo_depth().0;
    s.execute("tool.eraser", &json!({})).unwrap();
    for (k, x) in [(PointerKind::Down, 20.0), (PointerKind::Move, 60.0), (PointerKind::Up, 80.0)] {
        s.pointer(&PointerEvent::at(k, x, 300.0, 1.0)).unwrap();
    }
    assert_eq!(s.doc().unwrap().nodes().filter(|n| n.board == Some(BoardItem::Ink)).count(), 1, "the marker stroke is gone");
    assert_eq!(s.undo_depth().0, undo + 1, "one step");
    assert_eq!(s.tool.tool, ToolId::Eraser);
}

#[test]
fn the_sticky_and_connector_tools() {
    let mut s = board();
    s.execute("tool.sticky", &json!({"color": "orange"})).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Down, 100.0, 100.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, 100.0, 100.0, 1.0)).unwrap();
    let te = s.ds().unwrap().text_edit.clone().expect("typing in the new sticky");
    let a = crate::board::owner_of_text(s.doc().unwrap(), te.id).unwrap();
    assert_eq!(node(&s, a).fills[0].solid_color(), board_color("orange"));
    crate::textedit::insert(&mut s, "First").unwrap();
    crate::textedit::end(&mut s).unwrap();
    let b = id(&s.execute("board.sticky", &json!({"x": 500, "y": 0})).unwrap(), "id");
    s.execute("tool.connector", &json!({})).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Down, 100.0, 100.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, 300.0, 100.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, 600.0, 100.0, 1.0)).unwrap();
    let c = s.doc().unwrap().nodes().find(|n| matches!(n.board, Some(BoardItem::Connector(_)))).map(|n| n.id).unwrap();
    let k = connector(&s, c);
    assert_eq!((k.from.node(), k.to.node()), (Some(a), Some(b)));
}

#[test]
fn boards_sync_in_live_sessions() {
    let mut s = board();
    let a = id(&s.execute("board.sticky", &json!({"x": 0, "y": 0, "text": "A"})).unwrap(), "id");
    let mut room = Room::new(s.doc().unwrap().clone(), 2, 7);
    assert_eq!(room.s(1).doc().unwrap().kind, FileKind::Board, "joiners get a board");
    let b = id(&room.s(1).execute("board.sticky", &json!({"x": 400, "y": 0, "text": "B"})).unwrap(), "id");
    room.settle();
    let c = id(&room.s(0).execute("board.connector", &json!({"from": a, "to": b})).unwrap(), "id");
    room.s(1).execute("board.stamp", &json!({"x": 50, "y": 50, "stamp": "star"})).unwrap();
    room.settle();
    room.check();
    // Person 1 moves the sticky person 0 connected: both see the connector follow it.
    room.s(1).execute("transform.move", &json!({"ids": [b], "dx": 0, "dy": 250})).unwrap();
    room.s(0).execute("board.setColor", &json!({"ids": [a], "color": "violet"})).unwrap();
    room.settle();
    room.check();
    let doc = room.server.document();
    let NodeKind::Vector { network } = &doc.node(c).unwrap().kind else { panic!("not a vector") };
    let end = doc.world_transform(c) * network.vertices.last().unwrap().point();
    assert_eq!(end, Point::new(400.0, 350.0), "to the moved sticky's facing side");
    assert_eq!(doc.node(a).unwrap().fills[0].solid_color(), board_color("violet"));
}

#[test]
fn board_commands_survive_junk() {
    let junk = [
        json!({}),
        json!(null),
        json!([1]),
        json!({"x": "NaN", "y": 1e308, "text": 5, "color": [1], "author": {"x": 1}, "parent": "p"}),
        json!({"x": -1e308, "y": 0, "color": "neon", "shape": 7, "width": -5, "height": 1e308, "stamp": "skull"}),
        json!({"from": {"x": "a"}, "to": 99_999, "fromSide": "inside", "style": 3, "startCap": "spike", "endCap": 7, "label": [1]}),
        json!({"from": {"x": 1e308, "y": -1e308}, "to": {"x": 0, "y": 0}, "style": "elbow", "label": "\u{0}"}),
        json!({"ids": ["x", -1, 99_999], "color": "#GGG", "fontSize": -1, "side": "up", "edit": "yes"}),
        json!({"id": 2, "text": "", "path": "M0 0 L", "points": [[1]], "highlighter": "yes"}),
        json!({"path": "M0 0 L 1e308 1e308", "points": [[0, 0], [1e308, 1]]}),
        json!({"end": "from", "target": {"x": 1e308, "y": -1e308}, "side": "inside", "style": "curved"}),
        json!({"id": 99_999, "end": 7, "target": [1], "style": "wavy"}),
    ];
    let ids: Vec<&str> = crate::cmd::all_commands()
        .iter()
        .map(|c| c.id)
        .filter(|i| {
            i.starts_with("board.")
                || ["tool.sticky", "tool.shape", "tool.connector", "tool.marker", "tool.highlighter", "tool.eraser", "tool.stamp", "file.newBoard"]
                    .contains(i)
        })
        .collect();
    assert!(ids.len() >= 17, "{ids:?}");
    for cmd in ids {
        for j in &junk {
            let mut s = board();
            let a = s.execute("board.sticky", &json!({"x": 0, "y": 0})).unwrap()["id"].clone();
            let b = s.execute("board.shape", &json!({"x": 300, "y": 0})).unwrap()["id"].clone();
            let _ = s.execute("board.connector", &json!({"from": a, "to": b, "label": "x"}));
            if let Err(EngineError::Internal { cmd, msg }) = s.execute(cmd, j) {
                panic!("{cmd} panicked on {j}: {msg}");
            }
        }
    }
    // Every other command on a board with objects on it.
    for spec in crate::cmd::all_commands() {
        if spec.id.starts_with("file.") && spec.id != "file.newBoard" || spec.id == "text.rescanFonts" || spec.id == "image.place" {
            continue;
        }
        for j in &junk[..3] {
            let mut s = board();
            let a = s.execute("board.sticky", &json!({"x": 0, "y": 0})).unwrap()["id"].clone();
            let b = s.execute("board.stamp", &json!({"x": 300, "y": 0})).unwrap()["id"].clone();
            let _ = s.execute("board.connector", &json!({"from": a, "to": b}));
            if let Err(EngineError::Internal { cmd, msg }) = s.execute(spec.id, j) {
                panic!("{cmd} panicked on {j}: {msg}");
            }
        }
    }
}

#[test]
fn a_press_on_a_stickys_background_picks_and_drags_it() {
    let mut s = board();
    let a = id(&s.execute("board.sticky", &json!({"x": 0, "y": 0, "text": "Hi"})).unwrap(), "id");
    let b = id(&s.execute("board.sticky", &json!({"x": 400, "y": 0})).unwrap(), "id");
    let c = id(&s.execute("board.connector", &json!({"from": a, "to": b})).unwrap(), "id");
    s.execute("select.none", &json!({})).unwrap();
    // Below the note, on the sticky's own fill (a top-level frame's background is the canvas).
    s.pointer(&PointerEvent::at(PointerKind::Down, 100.0, 150.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, 100.0, 250.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, 100.0, 450.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, 100.0, 450.0, 1.0)).unwrap();
    assert_eq!(s.selection(), vec![a]);
    assert_eq!(node(&s, a).transform.translation().y, 300.0);
    assert!(near(drawn(&s, c)[0], Point::new(200.0, 400.0)), "the connector followed the drag: {:?}", drawn(&s, c));
}

#[test]
fn a_copy_of_joined_objects_joins_the_copies() {
    let mut s = board();
    let a = id(&s.execute("board.sticky", &json!({"x": 0, "y": 0})).unwrap(), "id");
    let b = id(&s.execute("board.sticky", &json!({"x": 400, "y": 0})).unwrap(), "id");
    let c = id(&s.execute("board.connector", &json!({"from": a, "to": b, "label": "x"})).unwrap(), "id");
    let label = crate::board::label_of(s.doc().unwrap(), c).unwrap();
    s.execute("edit.duplicate", &json!({"ids": [a, b, c, label]})).unwrap();
    let copies = s.selection();
    assert_eq!(copies.len(), 4, "{copies:?}");
    let k = copies.iter().find_map(|i| match node(&s, *i).board {
        Some(BoardItem::Connector(k)) => Some(k),
        _ => None,
    });
    let k = k.unwrap();
    assert!(copies.contains(&k.from.node().unwrap()) && copies.contains(&k.to.node().unwrap()), "{k:?} {copies:?}");
    // Copy and paste too.
    s.execute("select.set", &json!({"ids": [a, b, c]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    s.execute("edit.paste", &json!({})).unwrap();
    let pasted = s.selection();
    let k = pasted.iter().find_map(|i| match node(&s, *i).board {
        Some(BoardItem::Connector(k)) => Some(k),
        _ => None,
    });
    let k = k.unwrap();
    assert!(pasted.contains(&k.from.node().unwrap()) && pasted.contains(&k.to.node().unwrap()), "{k:?} {pasted:?}");
}
