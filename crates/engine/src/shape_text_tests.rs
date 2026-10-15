//! Shapes with text: double-click to type inside a shape; the text stays inside it.

use figcraft_doc::{NodeId, NodeKind, shape_text};
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::textedit;
use crate::tools::{PointerEvent, PointerKind};

fn id(v: Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn rect(s: &mut Session, x: f64, y: f64) -> NodeId {
    id(s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": 160, "height": 80})).unwrap())
}

fn click(s: &mut Session, kind: PointerKind, x: f64, y: f64) {
    s.pointer(&PointerEvent::at(kind, x, y, 1.0)).unwrap();
}

/// Double-click at (x, y), type `text` and press Escape.
fn type_into(s: &mut Session, x: f64, y: f64, text: &str) {
    click(s, PointerKind::Down, x, y);
    click(s, PointerKind::Up, x, y);
    click(s, PointerKind::DoubleClick, x, y);
    textedit::insert(s, text).unwrap();
    textedit::end(s).unwrap();
}

fn words(s: &Session, t: NodeId) -> String {
    match &s.doc().unwrap().node(t).unwrap().kind {
        NodeKind::Text(d) => d.characters.clone(),
        _ => panic!("not a text"),
    }
}

#[test]
fn double_clicking_a_shape_types_inside_it() {
    let mut s = Session::new();
    let r = rect(&mut s, 0.0, 0.0);
    type_into(&mut s, 80.0, 40.0, "Start");
    let doc = s.doc().unwrap();
    let t = shape_text::text_of(doc, r).expect("the shape has a text");
    assert_eq!(words(&s, t), "Start");
    assert_eq!(s.selection(), vec![r], "the shape is selected again");
    // Inside the shape's box, centred both ways, right above it.
    let b = doc.world_bounds(t).unwrap();
    assert_eq!((b.x0, b.y0, b.x1, b.y1), (12.0, 12.0, 148.0, 68.0));
    let NodeKind::Text(d) = &doc.node(t).unwrap().kind else { panic!() };
    assert_eq!((d.align_h, d.align_v), (figcraft_doc::TextAlignH::Center, figcraft_doc::TextAlignV::Middle));
    assert_eq!(doc.index_in_parent(t), doc.index_in_parent(r).map(|i| i + 1));
    // A click on the words is a click on the shape.
    assert_eq!(s.hit(doc, s.page().unwrap(), figcraft_geom::Point::new(80.0, 40.0), 1.0), Some(r));
    // Double-clicking again edits the same text; one undo step per edit.
    type_into(&mut s, 80.0, 40.0, "!");
    assert_eq!(shape_text::text_of(s.doc().unwrap(), r), Some(t));
    s.undo().unwrap();
    assert_eq!(words(&s, t), "Start");
    s.undo().unwrap();
    assert_eq!(shape_text::text_of(s.doc().unwrap(), r), None, "undo takes the new text away");
}

#[test]
fn the_text_follows_its_shape_and_goes_with_it() {
    let mut s = Session::new();
    let r = rect(&mut s, 0.0, 0.0);
    type_into(&mut s, 80.0, 40.0, "Process");
    let t = shape_text::text_of(s.doc().unwrap(), r).unwrap();
    // Moved and resized: the text keeps its font size and gets the new room.
    s.execute("node.set", &json!({"ids": [r], "props": {"x": 300, "y": 100, "width": 240}})).unwrap();
    let b = s.doc().unwrap().world_bounds(t).unwrap();
    assert_eq!((b.x0, b.y0, b.x1, b.y1), (312.0, 112.0, 528.0, 168.0));
    // Duplicated: the copy has its own text.
    let copy = id_of(&s.execute("edit.duplicate", &json!({"ids": [r]})).unwrap()["ids"][0]).unwrap();
    let doc = s.doc().unwrap();
    let ct = shape_text::text_of(doc, copy).expect("the copy's text");
    assert_ne!(ct, t);
    assert_eq!(words(&s, ct), "Process");
    assert_eq!(s.selection(), vec![copy], "the copy is selected, not its text");
    // Deleted: its text goes too.
    s.execute("edit.delete", &json!({"ids": [r]})).unwrap();
    assert!(!s.doc().unwrap().contains(t));
    assert!(s.doc().unwrap().contains(ct));
}

#[test]
fn a_text_left_empty_is_removed_and_ellipses_keep_text_inside() {
    let mut s = Session::new();
    let r = rect(&mut s, 0.0, 0.0);
    click(&mut s, PointerKind::Down, 80.0, 40.0);
    click(&mut s, PointerKind::Up, 80.0, 40.0);
    click(&mut s, PointerKind::DoubleClick, 80.0, 40.0);
    textedit::end(&mut s).unwrap();
    assert_eq!(shape_text::text_of(s.doc().unwrap(), r), None);
    assert_eq!(s.selection(), vec![r]);
    let e = id(s.execute("shape.ellipse", &json!({"x": 0, "y": 200, "width": 200, "height": 100})).unwrap());
    textedit::edit_shape_text(&mut s, e, None).unwrap();
    textedit::insert(&mut s, "Decision").unwrap();
    textedit::end(&mut s).unwrap();
    let t = shape_text::text_of(s.doc().unwrap(), e).unwrap();
    let b = s.doc().unwrap().world_bounds(t).unwrap();
    assert!(b.x0 > 25.0 && b.x1 < 175.0 && b.y0 > 210.0 && b.y1 < 290.0, "{b:?}");
}

#[test]
fn a_connection_point_click_adds_a_connected_copy_and_a_drag_draws_a_connector() {
    use figcraft_doc::Magnet;
    let mut s = Session::new();
    let r = rect(&mut s, 0.0, 0.0);
    type_into(&mut s, 80.0, 40.0, "Start");
    s.set_selection(vec![r]);
    // Four points just outside the middle of each side.
    let pts = s.connection_points();
    assert_eq!(pts.len(), 4);
    assert!(pts.contains(&(Magnet::Right, [160.0 + crate::tools::CONNECT_POINT_PX, 40.0])));
    // Hovering one shows a "+" and where the copy would go.
    let (rx, ry) = (160.0 + crate::tools::CONNECT_POINT_PX, 40.0);
    click(&mut s, PointerKind::Hover, rx, ry);
    assert_eq!(s.tool.overlay.connect_hover, Some(Magnet::Right));
    assert_eq!(s.tool.overlay.ghost.map(|g| g[0]), Some([240.0, 0.0]));
    // A click adds it, joined by a connector, and selects it; its text stays behind.
    click(&mut s, PointerKind::Down, rx, ry);
    click(&mut s, PointerKind::Up, rx, ry);
    let copy = s.selection()[0];
    let doc = s.doc().unwrap();
    assert_ne!(copy, r);
    assert_eq!(doc.world_bounds(copy).map(|b| (b.x0, b.y0)), Some((240.0, 0.0)));
    assert_eq!(shape_text::text_of(doc, copy), None, "a new shape is empty");
    let joined = doc.nodes().any(|n| matches!(&n.kind, NodeKind::Connector { start, end, .. } if start.node == Some(r) && end.node == Some(copy)));
    assert!(joined);
    // The same side again fans out: the place on the right is taken.
    s.execute("connector.next", &json!({"id": r, "side": "right"})).unwrap();
    let third = s.selection()[0];
    assert_eq!(s.doc().unwrap().world_bounds(third).map(|b| (b.x0, b.y0)), Some((240.0, 120.0)));
    // One undo step each.
    s.undo().unwrap();
    assert!(!s.doc().unwrap().contains(third));
    // Dragging from a point draws a connector from that side to what it's dropped on.
    s.set_selection(vec![r]);
    let (bx, by) = (80.0, 80.0 + crate::tools::CONNECT_POINT_PX);
    click(&mut s, PointerKind::Down, bx, by);
    click(&mut s, PointerKind::Move, 100.0, 200.0);
    click(&mut s, PointerKind::Move, 280.0, 40.0);
    assert_eq!(s.tool.overlay.drop_target, Some(copy));
    click(&mut s, PointerKind::Up, 280.0, 40.0);
    let c = s.selection()[0];
    let NodeKind::Connector { start, end, .. } = &s.doc().unwrap().node(c).unwrap().kind else { panic!("a connector") };
    assert_eq!((start.node, start.magnet, end.node), (Some(r), Magnet::Bottom, Some(copy)));
}

#[test]
fn a_board_dresses_new_shapes_and_texts_and_shows_no_corner_handles() {
    let mut s = Session::new();
    s.execute("file.newBoard", &json!({})).unwrap();
    assert!(s.doc().unwrap().board);
    let r = rect(&mut s, 0.0, 0.0);
    let n = s.doc().unwrap().node(r).unwrap().clone();
    assert_eq!(n.fills, vec![figcraft_color::Paint::solid(crate::board::SHAPE_FILL)], "pastel");
    assert!(n.strokes.is_empty(), "no border");
    type_into(&mut s, 80.0, 40.0, "Idea");
    let t = shape_text::text_of(s.doc().unwrap(), r).unwrap();
    let NodeKind::Text(d) = &s.doc().unwrap().node(t).unwrap().kind else { panic!() };
    assert_eq!(d.style.font_size, crate::board::TextSize::Small.px());
    s.set_selection(vec![r]);
    assert!(s.shape_handles().is_empty());
    // Saved and opened again, it's still a board.
    let json = serde_json::to_string(s.doc().unwrap()).unwrap();
    let back: figcraft_doc::Document = serde_json::from_str(&json).unwrap();
    assert!(back.board);
    // Diamonds are polygons with four points.
    s.execute("tool.set", &json!({"tool": "polygon", "points": 4})).unwrap();
    assert_eq!(s.tool.polygon_points, Some(4));
}
