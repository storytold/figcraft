//! A panic while normalising what a command changed is caught like one in the command itself
//! (#1085): the command fails with `EngineError::Internal`, the document is as it was, no undo step
//! is pushed, and the session keeps working.

use serde_json::json;

use crate::{EngineError, Session, derived};

/// Run `f` with normalisation panicking.
fn panicking<T>(f: impl FnOnce() -> T) -> T {
    derived::PANIC.with(|p| p.set(true));
    let out = f();
    derived::PANIC.with(|p| p.set(false));
    out
}

/// The layer id a creating command returned.
fn id(v: &serde_json::Value) -> u64 {
    v["id"].as_u64().unwrap()
}

/// A command whose edit panics while normalising fails with `EngineError::Internal`, leaves the
/// document and the undo history as they were, and the same command then runs as usual.
#[test]
fn a_panic_while_normalising_rolls_the_command_back() {
    let mut s = Session::new();
    let r = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
    let before = s.doc().unwrap().clone();
    let undo = s.undo_depth();
    let result = panicking(|| s.execute("node.set", &json!({"ids": [r], "props": {"name": "Renamed"}})));
    assert!(matches!(result, Err(EngineError::Internal { .. })), "{result:?}");
    assert_eq!(s.doc().unwrap(), &before, "the edit is rolled back");
    assert_eq!(s.undo_depth(), undo, "no undo step for the failed command");
    // The next command runs as usual.
    s.execute("node.set", &json!({"ids": [r], "props": {"name": "Renamed"}})).unwrap();
    assert_eq!(s.doc().unwrap().node(figcraft_doc::NodeId(r)).unwrap().name, "Renamed");
    assert_eq!(s.undo_depth().0, undo.0 + 1);
}

/// The step fails like any other failed step: the batch's group restores what it set aside for
/// its steps (the history limit, the group depth), which a panic unwinding through it skipped.
#[test]
fn a_panic_while_normalising_a_batch_step_rolls_the_batch_back() {
    let mut s = Session::new();
    let before = s.doc().unwrap().clone();
    let undo = s.undo_depth();
    let limit = s.ds().unwrap().history.limit;
    let result = panicking(|| {
        s.execute("command.batch", &json!({"commands": [{"command": "shape.rectangle", "params": {"x": 0, "y": 0, "width": 10, "height": 10}}]}))
    });
    assert!(result.is_err(), "{result:?}");
    assert_eq!(s.doc().unwrap(), &before);
    assert_eq!(s.undo_depth(), undo);
    assert_eq!(s.ds().unwrap().history.limit, limit, "the undo limit is back");
    assert_eq!(s.group_depth, 0, "the group ended");
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
}

/// A live text preview whose normalising panics fails and shows nothing: the document is as it
/// was, and the same preview then shows.
#[test]
fn a_panic_while_normalising_a_text_preview_shows_none() {
    let mut s = Session::new();
    let t = id(&s.execute("text.create", &json!({"x": 0, "y": 0, "text": "Preview"})).unwrap());
    let before = s.doc().unwrap().clone();
    let result = panicking(|| s.execute("text.preview", &json!({"command": "text.style", "params": {"ids": [t], "fontSize": 32}})));
    assert!(matches!(result, Err(EngineError::Internal { .. })), "{result:?}");
    assert_eq!(s.doc().unwrap(), &before);
    assert!(!crate::textpreview::active(&s), "no preview shows");
    s.execute("text.preview", &json!({"command": "text.style", "params": {"ids": [t], "fontSize": 32}})).unwrap();
    assert!(crate::textpreview::active(&s));
}
