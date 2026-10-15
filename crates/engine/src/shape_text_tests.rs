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

#[test]
fn changing_a_shapes_type_keeps_its_text_and_connectors() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    let b = rect(&mut s, 300.0, 0.0);
    type_into(&mut s, 80.0, 40.0, "Decide");
    let c = id(s.execute("connector.create", &json!({"start": {"node": a}, "end": {"node": b}})).unwrap());
    s.execute("shape.setType", &json!({"ids": [a], "type": "diamond"})).unwrap();
    let doc = s.doc().unwrap();
    assert!(matches!(doc.node(a).unwrap().kind, NodeKind::Polygon { count: 4 }));
    let t = shape_text::text_of(doc, a).expect("its text stays");
    assert_eq!(words(&s, t), "Decide");
    let NodeKind::Connector { start, .. } = &doc.node(c).unwrap().kind else { panic!() };
    assert_eq!(start.node, Some(a), "its connector stays attached");
    // The text moves in: a diamond's text sits further inside.
    let tb = doc.world_bounds(t).unwrap();
    assert!(tb.x0 > 30.0, "{tb:?}");
    assert!(s.execute("shape.setType", &json!({"ids": [c], "type": "ellipse"})).is_err(), "a connector isn't a shape");
}

#[test]
fn library_shapes_are_drawn_hold_text_and_swap() {
    let mut s = Session::new();
    s.execute("file.newBoard", &json!({})).unwrap();
    // Every library shape has an outline that fills its box.
    for p in crate::board_shapes::PRESETS {
        let path = crate::board_shapes::path(p.id, 200.0, 100.0).unwrap_or_else(|| panic!("{} has no outline", p.id));
        use figcraft_geom::Shape;
        let b = path.bounding_box();
        assert!(b.x0 >= -0.01 && b.y0 >= -0.01 && b.x1 <= 200.01 && b.y1 <= 100.01, "{}: {b:?}", p.id);
        assert!(b.width() > 150.0 && b.height() > 60.0, "{} fills its box: {b:?}", p.id);
    }
    // Made by name, drawn by the rectangle tool with a preset, and typed into with a double-click.
    let cyl = id(s.execute("node.create", &json!({"type": "cylinder", "x": 0, "y": 0, "width": 160, "height": 80})).unwrap());
    assert_eq!(crate::board_shapes::preset_of(s.doc().unwrap().node(cyl).unwrap()).map(|p| p.id), Some("cylinder"));
    assert!(!s.doc().unwrap().node(cyl).unwrap().strokes.is_empty(), "its rim shows");
    type_into(&mut s, 80.0, 40.0, "Users");
    assert!(shape_text::text_of(s.doc().unwrap(), cyl).is_some());
    s.execute("tool.set", &json!({"tool": "rectangle", "preset": "document"})).unwrap();
    click(&mut s, PointerKind::Down, 300.0, 0.0);
    click(&mut s, PointerKind::Move, 400.0, 60.0);
    click(&mut s, PointerKind::Up, 460.0, 80.0);
    let doc_shape = s.selection()[0];
    assert_eq!(crate::board_shapes::preset_of(s.doc().unwrap().node(doc_shape).unwrap()).map(|p| p.id), Some("document"));
    // Swapped to a plain kind and back to another library shape, keeping the text.
    s.execute("shape.setType", &json!({"ids": [cyl], "type": "parallelogram"})).unwrap();
    assert_eq!(crate::board_shapes::preset_of(s.doc().unwrap().node(cyl).unwrap()).map(|p| p.id), Some("parallelogram"));
    s.execute("shape.setType", &json!({"ids": [cyl], "type": "rectangle"})).unwrap();
    let n = s.doc().unwrap().node(cyl).unwrap();
    assert!(matches!(n.kind, NodeKind::Rectangle) && crate::board_shapes::preset_of(n).is_none());
    assert!(shape_text::text_of(s.doc().unwrap(), cyl).is_some());
    assert!(s.execute("tool.set", &json!({"tool": "rectangle", "preset": "nope"})).is_err());
}

#[test]
fn a_connector_label_sits_halfway_and_follows_the_line() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    let b = rect(&mut s, 400.0, 0.0);
    let c = id(s.execute("connector.create", &json!({"start": {"node": a}, "end": {"node": b}, "lineType": "straight"})).unwrap());
    // A double-click on the selected connector types its label.
    s.set_selection(vec![c]);
    click(&mut s, PointerKind::DoubleClick, 280.0, 40.0);
    textedit::insert(&mut s, "yes").unwrap();
    textedit::end(&mut s).unwrap();
    let doc = s.doc().unwrap();
    let t = shape_text::text_of(doc, c).expect("a label");
    assert_eq!(words(&s, t), "yes");
    assert_eq!(s.selection(), vec![c]);
    // Centred halfway along the line (from x 160 to 400, at y 40).
    let lb = doc.world_bounds(t).unwrap();
    assert!((lb.center().x - 280.0).abs() < 0.5 && (lb.center().y - 40.0).abs() < 0.5, "{lb:?}");
    // The line moves: the label follows; a click on it is a click on the connector.
    s.execute("node.set", &json!({"ids": [b], "props": {"y": 200}})).unwrap();
    let doc = s.doc().unwrap();
    let lb = doc.world_bounds(t).unwrap();
    let NodeKind::Connector { line_type, route, .. } = &doc.node(c).unwrap().kind else { panic!() };
    let mid = shape_text::label_point(doc.node(c).unwrap()).unwrap();
    assert!((lb.center() - mid).hypot() < 0.5, "{lb:?} {mid:?} {line_type:?} {route:?}");
    assert_eq!(s.hit(doc, s.page().unwrap(), lb.center(), 1.0), Some(c));
    // The label goes with its connector.
    s.execute("edit.delete", &json!({"ids": [c]})).unwrap();
    assert!(!s.doc().unwrap().contains(t));
    // shape.editText starts one from a command.
    let c2 = id(s.execute("connector.create", &json!({"start": {"node": a}, "end": {"node": b}})).unwrap());
    s.execute("shape.editText", &json!({"id": c2})).unwrap();
    assert_eq!(s.ds().unwrap().text_edit.as_ref().and_then(|t| t.host), Some(c2));
}

#[test]
fn sticky_notes_grow_with_their_words_and_are_signed() {
    let mut s = Session::new();
    s.execute("file.newBoard", &json!({})).unwrap();
    s.prefs.user_name = "Ada".into();
    let made = s.execute("sticky.create", &json!({"x": 120, "y": 120, "text": "Short"})).unwrap();
    let n = id(made);
    let doc = s.doc().unwrap();
    let note = doc.node(n).unwrap();
    assert!(shape_text::is_sticky(note));
    assert_eq!((note.width, note.height), (240.0, 240.0), "a square");
    assert_eq!(shape_text::sticky_author(note), Some("Ada"));
    assert_eq!(doc.world_bounds(n).map(|b| (b.x0, b.y0)), Some((0.0, 0.0)), "centred on the point");
    let t = shape_text::text_of(doc, n).unwrap();
    assert_eq!(doc.world_bounds(t).map(|b| (b.x0, b.y0)), Some((16.0, 16.0)), "its text starts at the top left");
    // Many words: it grows down, its text with it.
    s.execute("node.set", &json!({"ids": [t], "props": {"characters": "word ".repeat(120)}})).unwrap();
    let doc = s.doc().unwrap();
    let tall = doc.node(n).unwrap().height;
    assert!(tall > 240.0, "{tall}");
    let tb = doc.world_bounds(t).unwrap();
    assert!((tall - (tb.height() + 32.0 + shape_text::STICKY_AUTHOR)).abs() < 1.0);
    // Wide, unsigned, another colour.
    s.execute("sticky.set", &json!({"ids": [n], "wide": true, "author": false, "color": "blue"})).unwrap();
    let note = s.doc().unwrap().node(n).unwrap();
    assert_eq!(note.width, 480.0);
    assert_eq!(shape_text::sticky_author(note), None);
    // The Sticky tool: a click puts one down and starts typing in it.
    s.execute("tool.set", &json!({"tool": "sticky", "color": "green"})).unwrap();
    click(&mut s, PointerKind::Down, 800.0, 800.0);
    let te = s.ds().unwrap().text_edit.clone().expect("typing");
    let host = te.host.unwrap();
    let note = s.doc().unwrap().node(host).unwrap();
    assert!(shape_text::is_sticky(note));
    assert_eq!(note.fills, vec![figcraft_color::Paint::solid(crate::board::STICKY_COLORS[7].color)]);
    assert!(s.execute("tool.set", &json!({"tool": "sticky", "color": "plaid"})).is_err());
}

#[test]
fn mind_maps_branch_by_keyboard_commands_and_lay_themselves_out() {
    use crate::mindmap;
    let mut s = Session::new();
    s.execute("file.newBoard", &json!({})).unwrap();
    let root = id(s.execute("mindmap.create", &json!({"x": 0, "y": 0, "text": "Launch plan"})).unwrap());
    let add = |s: &mut Session, cmd: &str, at: NodeId, text: &str| -> NodeId {
        let n = id(s.execute(cmd, &json!({"id": at, "edit": true})).unwrap());
        textedit::insert(s, text).unwrap();
        textedit::end(s).unwrap();
        n
    };
    let research = add(&mut s, "mindmap.addChild", root, "Research");
    let design = add(&mut s, "mindmap.addSibling", research, "Design");
    let build = add(&mut s, "mindmap.addSibling", design, "Build");
    let tests = add(&mut s, "mindmap.addChild", build, "Tests");
    let doc = s.doc().unwrap();
    assert_eq!(mindmap::children(doc, root), vec![research, design, build]);
    assert_eq!(mindmap::root_of(doc, tests), Some(root));
    // To the right of their parent, stacked top to bottom and centred on it.
    let b = |id: NodeId| doc.world_bounds(id).unwrap();
    let (r, re, de, bu, te) = (b(root), b(research), b(design), b(build), b(tests));
    assert!((re.x0 - (r.x1 + mindmap::HGAP)).abs() < 0.5, "{r:?} {re:?}");
    assert!(re.y1 <= de.y0 && de.y1 <= bu.y0, "in order");
    assert!(((re.y0 + bu.y1) / 2.0 - r.center().y).abs() < 0.5, "centred on the root");
    assert!((te.x0 - (bu.x1 + mindmap::HGAP)).abs() < 0.5 && (te.center().y - bu.center().y).abs() < 0.5);
    // Each topic is as wide as its words; each branch has its own colour, kept below it.
    assert!(b(research).width() >= 96.0);
    let stroke = |id: NodeId| doc.node(id).unwrap().strokes.first().cloned();
    assert_ne!(stroke(research), stroke(design));
    assert_eq!(stroke(build), stroke(tests));
    // Curved lines from each topic to the ones branching off it.
    let edges = doc.nodes().filter(|n| n.plugin_data.contains_key(mindmap::EDGE)).count();
    assert_eq!(edges, 4);
    // A sibling inserted in the middle takes its place.
    let between = add(&mut s, "mindmap.addSibling", research, "Budget");
    assert_eq!(mindmap::children(s.doc().unwrap(), root), vec![research, between, design, build]);
    // Collapsed: what branches off hides (and its line); shown again.
    s.execute("mindmap.collapse", &json!({"id": build})).unwrap();
    assert!(!s.doc().unwrap().node(tests).unwrap().visible);
    s.execute("mindmap.collapse", &json!({"id": build})).unwrap();
    assert!(s.doc().unwrap().node(tests).unwrap().visible);
    // Moving the central topic moves the whole map.
    s.execute("node.set", &json!({"ids": [root], "props": {"x": 500}})).unwrap();
    let moved = s.doc().unwrap().world_bounds(research).unwrap();
    assert!((moved.x0 - (s.doc().unwrap().world_bounds(root).unwrap().x1 + mindmap::HGAP)).abs() < 0.5);
    // Deleting a topic deletes what branches off it, and their lines.
    s.execute("edit.delete", &json!({"ids": [build]})).unwrap();
    let doc = s.doc().unwrap();
    assert!(!doc.contains(tests));
    assert_eq!(doc.nodes().filter(|n| n.plugin_data.contains_key(mindmap::EDGE)).count(), 3);
    assert!(s.execute("mindmap.addChild", &json!({"id": figcraft_doc::NodeId(99_999)})).is_err());
}

#[test]
fn tables_lay_out_their_cells_and_grow_rows_with_their_words() {
    use crate::table;
    let mut s = Session::new();
    s.execute("file.newBoard", &json!({})).unwrap();
    let t = id(s.execute("table.create", &json!({"rows": 2, "cols": 3, "x": 210, "y": 44})).unwrap());
    let doc = s.doc().unwrap();
    assert_eq!(table::cells(doc, t).len(), 6);
    assert_eq!(doc.world_bounds(t).map(|b| (b.x0, b.y0, b.width(), b.height())), Some((0.0, 0.0, 420.0, 88.0)));
    let cell =
        |s: &Session, r: usize, c: usize| table::cells(s.doc().unwrap(), t).into_iter().find(|(rc, _)| *rc == (r, c)).map(|(_, id)| id).unwrap();
    // Typing in a cell: its row grows with the words, the others stay.
    let b1 = cell(&s, 1, 1);
    textedit::edit_shape_text(&mut s, b1, None).unwrap();
    textedit::insert(&mut s, &"many words in one cell ".repeat(6)).unwrap();
    textedit::end(&mut s).unwrap();
    let doc = s.doc().unwrap();
    let row1 = doc.world_bounds(cell(&s, 1, 0)).unwrap();
    assert!(row1.height() > table::ROW_H, "{row1:?}");
    assert_eq!(doc.world_bounds(cell(&s, 0, 0)).unwrap().height(), table::ROW_H);
    assert!((doc.world_bounds(t).unwrap().height() - (table::ROW_H + row1.height())).abs() < 0.5);
    // Rows and columns in and out; the words move with their cells.
    s.execute("table.insertRow", &json!({"id": b1, "above": true})).unwrap();
    assert_eq!(table::cell_of(s.doc().unwrap().node(b1).unwrap()), Some((2, 1)));
    s.execute("table.insertColumn", &json!({"id": b1, "left": true})).unwrap();
    assert_eq!(table::cell_of(s.doc().unwrap().node(b1).unwrap()), Some((2, 2)));
    assert_eq!(table::cells(s.doc().unwrap(), t).len(), 12);
    let words = shape_text::text_of(s.doc().unwrap(), b1).unwrap();
    s.execute("table.deleteRow", &json!({"id": b1})).unwrap();
    assert!(!s.doc().unwrap().contains(b1) && !s.doc().unwrap().contains(words), "the row and its words went");
    assert_eq!(table::rows(s.doc().unwrap(), t), 2);
    s.execute("table.deleteColumn", &json!({"id": cell(&s, 0, 0)})).unwrap();
    assert_eq!(table::widths(s.doc().unwrap().node(t).unwrap()).len(), 3);
    // No connection points on cells; refused on what isn't a cell.
    s.set_selection(vec![cell(&s, 0, 0)]);
    assert!(s.connection_points().is_empty());
    assert!(s.execute("table.insertRow", &json!({"id": t})).is_err());
}
