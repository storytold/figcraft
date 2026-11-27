//! Tab and Shift+Tab while editing a text layer move editing to the next or previous text layer
//! of the same top-level frame, in reading order (rows top to bottom, each left to right), with
//! all of its text selected; hidden and locked text is passed over, and at either end the key
//! does nothing.

use figcraft_doc::NodeId;
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn text(s: &mut Session, chars: &str, x: f64, y: f64, parent: NodeId) -> NodeId {
    id_of(&ex(s, "text.create", json!({"text": chars, "x": x, "y": y, "parent": parent}))["id"]).unwrap()
}

fn frame(s: &mut Session, x: f64, y: f64) -> NodeId {
    id_of(&ex(s, "frame.create", json!({"x": x, "y": y, "width": 400, "height": 300}))["id"]).unwrap()
}

fn tab(s: &mut Session, shift: bool) {
    ex(s, "text.edit.key", json!({"key": "Tab", "shift": shift}));
}

/// The text being edited and whether all of it is selected.
fn editing(s: &Session) -> (NodeId, bool) {
    let te = s.ds().unwrap().text_edit.clone().expect("still editing");
    let n = s.doc().unwrap().node(te.id).unwrap().kind.text().unwrap().char_count();
    (te.id, te.anchor.min(te.caret) == 0 && te.anchor.max(te.caret) == n)
}

fn chars(s: &Session, id: NodeId) -> String {
    s.doc().unwrap().node(id).unwrap().kind.text().unwrap().characters.clone()
}

/// A frame with a row of three texts whose tops differ by a few pixels, out of order in the
/// layer list, and a second row below; a text in another frame beside it.
fn two_rows(s: &mut Session) -> Vec<NodeId> {
    let f = frame(s, 0.0, 0.0);
    let b = text(s, "B", 150.0, 12.0, f);
    let a = text(s, "A", 10.0, 10.0, f);
    let e = text(s, "E", 150.0, 100.0, f);
    let c = text(s, "C", 290.0, 6.0, f);
    let d = text(s, "D", 10.0, 101.0, f);
    let other = frame(s, 500.0, 0.0);
    text(s, "Elsewhere", 10.0, 10.0, other);
    vec![a, b, c, d, e]
}

#[test]
fn tab_reads_a_row_left_to_right_then_the_next_row() {
    let mut s = Session::new();
    let order = two_rows(&mut s);
    ex(&mut s, "text.edit.begin", json!({"id": order[0], "caret": 0}));
    for want in &order[1..] {
        tab(&mut s, false);
        assert_eq!(editing(&s), (*want, true), "on to {} with all of it selected", chars(&s, *want));
    }
    assert!(order.iter().all(|t| !chars(&s, *t).contains('\t')), "no tab typed");
}

#[test]
fn shift_tab_goes_back() {
    let mut s = Session::new();
    let order = two_rows(&mut s);
    ex(&mut s, "text.edit.begin", json!({"id": order[4], "caret": 0}));
    for want in order[..4].iter().rev() {
        tab(&mut s, true);
        assert_eq!(editing(&s).0, *want, "back to {}", chars(&s, *want));
    }
}

#[test]
fn tab_on_the_last_text_keeps_editing_it() {
    let mut s = Session::new();
    let order = two_rows(&mut s);
    let undo = s.ds().unwrap().history.undo.len();
    ex(&mut s, "text.edit.begin", json!({"id": order[4], "caret": 1}));
    tab(&mut s, false);
    let te = s.ds().unwrap().text_edit.clone().expect("still editing");
    assert_eq!((te.id, te.caret, te.anchor), (order[4], 1, 1), "the last one: nothing moves");
    assert_eq!(chars(&s, order[4]), "E", "and nothing is typed");
    // Shift+Tab on the first one does nothing either.
    ex(&mut s, "text.edit.end", json!({}));
    ex(&mut s, "text.edit.begin", json!({"id": order[0], "caret": 0}));
    tab(&mut s, true);
    assert_eq!(editing(&s).0, order[0]);
    ex(&mut s, "text.edit.end", json!({}));
    assert_eq!(s.ds().unwrap().history.undo.len(), undo, "no undo step for it");
}

#[test]
fn hidden_and_locked_text_is_skipped() {
    let mut s = Session::new();
    let f = frame(&mut s, 0.0, 0.0);
    let first = text(&mut s, "First", 10.0, 10.0, f);
    let hidden = text(&mut s, "Hidden", 10.0, 50.0, f);
    ex(&mut s, "node.set", json!({"id": hidden, "props": {"visible": false}}));
    let locked = text(&mut s, "Locked", 10.0, 90.0, f);
    ex(&mut s, "node.set", json!({"id": locked, "props": {"locked": true}}));
    let inner = id_of(&ex(&mut s, "frame.create", json!({"x": 10, "y": 130, "width": 200, "height": 60, "parent": f}))["id"]).unwrap();
    text(&mut s, "In a locked frame", 0.0, 0.0, inner);
    ex(&mut s, "node.set", json!({"id": inner, "props": {"locked": true}}));
    let last = text(&mut s, "Last", 10.0, 220.0, f);
    ex(&mut s, "text.edit.begin", json!({"id": first, "caret": 0}));
    tab(&mut s, false);
    assert_eq!(editing(&s).0, last, "past the hidden text, the locked one and the text in a locked frame");
    tab(&mut s, true);
    assert_eq!(editing(&s).0, first);
}
