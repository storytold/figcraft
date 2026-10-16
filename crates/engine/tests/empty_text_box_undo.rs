#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use figcraft_engine::{
    Session,
    doc::NodeKind,
    tools::{PointerEvent, PointerKind},
};
use serde_json::json;

fn text_count(s: &Session) -> usize {
    let doc = s.doc().unwrap();
    doc.node(s.page().unwrap()).unwrap().children.iter().filter(|id| matches!(doc.node(**id).unwrap().kind, NodeKind::Text(_))).count()
}

fn rect_count(s: &Session) -> usize {
    let doc = s.doc().unwrap();
    doc.node(s.page().unwrap()).unwrap().children.len() - text_count(s)
}

fn click_and_abandon(s: &mut Session) {
    s.execute("tool.text", &json!({})).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Down, 300.0, 300.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, 300.0, 300.0, 1.0)).unwrap();
    assert_eq!(text_count(s), 1);
    s.execute("text.edit.end", &json!({})).unwrap();
    assert_eq!(text_count(s), 0);
}

#[test]
fn abandoning_empty_text_does_not_create_an_undo_step() {
    let mut s = Session::new();
    assert_eq!(s.undo_depth(), (0, 0));
    click_and_abandon(&mut s);
    assert_eq!(s.undo_depth(), (0, 0));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(text_count(&s), 0, "Undo restored the abandoned empty layer");
}

#[test]
fn abandoning_empty_text_keeps_the_previous_undo_step_first() {
    let mut s = Session::new();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    click_and_abandon(&mut s);
    assert_eq!(s.undo_labels(2), vec!["Rectangle".to_string()]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(text_count(&s), 0);
    assert_eq!(rect_count(&s), 0, "the first Undo must remove the rectangle");
}

#[test]
fn abandoning_empty_text_keeps_the_redo_stack() {
    let mut s = Session::new();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.undo_depth(), (0, 1));
    click_and_abandon(&mut s);
    assert_eq!(s.undo_depth(), (0, 1), "the document is back where it was, so Redo still applies");
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(rect_count(&s), 1);
}
