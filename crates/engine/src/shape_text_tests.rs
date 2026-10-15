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
