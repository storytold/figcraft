//! An AI agent's command that edits the document is refused while a gesture of the window is open
//! on it: the gesture would fold the edit into its own step, or lose it when cancelled. Read-only
//! agent commands, and the window's own commands, keep working during a gesture. Once the gesture
//! ends, the agent's edit is its own undo step.

use std::sync::Arc;

use figcraft_doc::NodeId;
use serde_json::{Value, json};

use crate::{EngineError, Session};

fn id(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// A session with one rectangle, and a window gesture open on it, moved by a preview.
fn gesture() -> (Session, NodeId) {
    let mut s = Session::new();
    let r = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 80, "height": 60})).unwrap());
    s.begin_interaction("Move").unwrap();
    let props = serde_json::Map::from_iter([("x".to_string(), json!(20.0))]);
    s.preview(|d, _| crate::cmd::props::apply("node.set", d, r, &props)).unwrap();
    (s, r)
}

fn gesture_open(s: &Session) -> bool {
    s.ds().unwrap().interaction.is_some()
}

#[test]
fn an_agent_edit_during_a_gesture_is_refused_and_the_document_is_unchanged() {
    let (mut s, _) = gesture();
    // Calibration: the gesture is open, and the window's own command goes through it.
    assert!(gesture_open(&s));
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    assert!(gesture_open(&s));

    let doc = Arc::clone(&s.ds().unwrap().doc);
    let depth = s.undo_depth();
    let r = s.execute_as(Some("a"), "shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 40}));
    assert!(matches!(r, Err(EngineError::Disabled(ref c, ref why)) if c == "shape.rectangle" && why.contains("gesture")), "{r:?}");
    assert!(Arc::ptr_eq(&doc, &s.ds().unwrap().doc), "the refused edit left the document as it was");
    assert_eq!(s.undo_depth(), depth);
    assert!(gesture_open(&s), "the gesture is still open");
}

#[test]
fn an_agent_undo_during_a_gesture_is_refused_and_keeps_the_history() {
    let mut s = Session::new();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    s.begin_interaction("Move").unwrap();
    assert!(gesture_open(&s));

    let doc = Arc::clone(&s.ds().unwrap().doc);
    let depth = s.undo_depth();
    let r = s.execute_as(Some("a"), "edit.undo", &json!({}));
    assert!(matches!(r, Err(EngineError::Disabled(ref c, _)) if c == "edit.undo"), "{r:?}");
    assert!(Arc::ptr_eq(&doc, &s.ds().unwrap().doc));
    assert_eq!(s.undo_depth(), depth, "the undo step is still there");
    assert!(gesture_open(&s));
}

#[test]
fn a_read_only_agent_command_during_a_gesture_keeps_working() {
    let (mut s, _) = gesture();
    assert!(gesture_open(&s));
    let view = s.execute_as(Some("a"), "view.get", &json!({})).unwrap();
    assert!(view.is_object());
    assert!(gesture_open(&s));
}

#[test]
fn an_agent_edit_after_the_gesture_is_its_own_undo_step() {
    let (mut s, _) = gesture();
    assert!(gesture_open(&s));
    s.commit_interaction().unwrap();
    assert!(!gesture_open(&s));
    let depth = s.undo_depth();

    let r = id(&s.execute_as(Some("a"), "shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap());
    assert_eq!(s.undo_depth(), (depth.0 + 1, 0), "the agent's edit is a step of its own");
    assert_eq!(s.undo_labels(1), vec![crate::cmd::find_command("shape.rectangle").unwrap().label.to_string()]);
    s.undo().unwrap();
    assert!(s.doc().unwrap().node(r).is_none());
}

/// Editing text opens a gesture too, and it belongs to the agent whose call opened it: that agent
/// types into its text, while another agent's edit waits like during a gesture of the window.
#[test]
fn an_agent_types_in_the_text_it_is_editing_while_another_agents_edit_is_refused() {
    let mut s = Session::new();
    let t = id(&s.execute_as(Some("a"), "node.create", &json!({"type": "text", "text": "Hi"})).unwrap());
    s.execute_as(Some("a"), "text.edit.begin", &json!({"id": t.0})).unwrap();
    assert!(gesture_open(&s), "calibration: editing text opened a gesture");
    s.execute_as(Some("a"), "text.edit.insert", &json!({"text": "Hello"})).unwrap();
    let r = s.execute_as(Some("b"), "shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10}));
    assert!(matches!(r, Err(EngineError::Disabled(ref c, ref why)) if c == "shape.rectangle" && why.contains("gesture")), "{r:?}");
}

#[test]
fn a_window_command_during_a_gesture_is_not_refused() {
    let (mut s, _) = gesture();
    assert!(gesture_open(&s));
    let before = s.ds().unwrap().doc.node_count();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    assert!(gesture_open(&s));
    assert!(s.ds().unwrap().doc.node_count() > before, "the window's edit went through");
}
