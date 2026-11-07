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
    let r = id_of(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})));
    let (view, depth) = (s.page().unwrap(), s.undo_depth());
    assert!(s.execute("page.setOpening", &json!({"id": divider})).is_err(), "a divider isn't a page to open on");
    assert!(s.execute("page.setOpening", &json!({"id": r})).is_err(), "a layer isn't a page");
    assert_eq!(s.doc().unwrap().current_page, Some(two), "refusals change nothing");
    assert_eq!(s.page().unwrap(), view, "the viewer's page is where it was");
    assert_eq!(s.undo_depth(), depth, "and no undo step was added");
}

/// An id that is no node at all errors and changes nothing.
#[test]
fn an_unknown_id_is_refused_and_changes_nothing() {
    let mut s = Session::new();
    let first = s.page().unwrap();
    let depth = s.undo_depth();
    assert!(s.execute("page.setOpening", &json!({"id": 987_654_321u64})).is_err(), "no such node");
    assert_eq!(s.doc().unwrap().current_page, Some(first));
    assert_eq!(s.undo_depth(), depth);
}

/// Setting the page that already is the opening page changes nothing and adds no undo step.
#[test]
fn setting_the_page_that_already_opens_the_file_adds_no_undo_step() {
    let mut s = Session::new();
    let first = s.page().unwrap();
    assert_eq!(s.doc().unwrap().current_page, Some(first));
    let depth = s.undo_depth();
    ex(&mut s, "page.setOpening", json!({"id": first}));
    assert_eq!(s.undo_depth(), depth, "already the opening page: no step");
    assert_eq!(s.doc().unwrap().current_page, Some(first));
}

/// Deleting the opening page leaves the file opening on a page that exists.
#[test]
fn deleting_the_opening_page_falls_back_to_a_page_that_exists() {
    let mut s = Session::new();
    let first = s.page().unwrap();
    let cover = id_of(&ex(&mut s, "page.add", json!({"name": "Cover"})));
    ex(&mut s, "page.setOpening", json!({"id": cover}));
    ex(&mut s, "page.delete", json!({"id": cover}));
    let doc = s.doc().unwrap().clone();
    let now = doc.current_page.expect("an opening page remains");
    assert!(doc.pages.contains(&now) && now != cover, "a page that exists");
    assert_eq!(now, first);
    assert_eq!(Session::with_document(doc).page().unwrap(), first, "a reopened file lands on it");
}
