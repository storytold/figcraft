//! `page.setOpening` (#1817): the page a file opens on, for everyone who opens or joins it, written
//! as a document change (so it syncs and undoes) without moving anyone's own view.

use figcraft_doc::NodeId;
use serde_json::{Value, json};

use crate::Session;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// Setting the opening page writes the document, leaves this viewer where they are, is one undo
/// step, and a file opened from that document lands on it.
#[test]
fn the_opening_page_is_written_without_moving_the_view_and_a_reopened_file_lands_on_it() {
    let mut s = Session::new();
    let first = s.page().unwrap();
    let cover = id_of(&ex(&mut s, "page.add", json!({"name": "Cover"})));
    ex(&mut s, "page.select", json!({"id": first}));
    let depth = s.undo_depth().0;
    ex(&mut s, "page.setOpening", json!({"id": cover}));
    assert_eq!(s.doc().unwrap().current_page, Some(cover), "written to the document");
    assert_eq!(s.page().unwrap(), first, "the view stays where it was");
    assert_eq!(s.undo_depth().0, depth + 1, "one undo step");
    let reopened = Session::with_document(s.doc().unwrap().clone());
    assert_eq!(reopened.page().unwrap(), cover, "a file opened from this document lands on it");
    ex(&mut s, "edit.undo", json!({}));
    assert_ne!(s.doc().unwrap().current_page, Some(cover), "undone");
    assert_eq!(Session::with_document(s.doc().unwrap().clone()).page().unwrap(), first);
}

/// Without an id the current page is meant; a divider and a layer are refused and change nothing.
#[test]
fn the_current_page_is_the_default_and_dividers_and_layers_are_refused() {
    let mut s = Session::new();
    let two = id_of(&ex(&mut s, "page.add", json!({"name": "Two"})));
    assert_eq!(s.page().unwrap(), two, "page.add goes to the new page");
    ex(&mut s, "page.setOpening", json!({}));
    assert_eq!(s.doc().unwrap().current_page, Some(two), "the current page when none is named");
    let divider = id_of(&ex(&mut s, "page.add", json!({"name": "-"})));
    assert!(s.execute("page.setOpening", &json!({"id": divider})).is_err(), "a divider isn't a page to open on");
    let r = id_of(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})));
    assert!(s.execute("page.setOpening", &json!({"id": r})).is_err(), "a layer isn't a page");
    assert_eq!(s.doc().unwrap().current_page, Some(two), "refusals change nothing");
}
