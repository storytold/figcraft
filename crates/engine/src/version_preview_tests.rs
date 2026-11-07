//! Viewing a version on the canvas: read-only, restore from it, back to the current version, and
//! the document (and its unsaved state) untouched by the preview.

use serde_json::json;

use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind};
use crate::{EngineError, Session};

fn rect(s: &mut Session, x: f64) -> figcraft_doc::NodeId {
    id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 10, "height": 10})).unwrap()["id"]).unwrap()
}

fn count(s: &Session) -> usize {
    s.doc().unwrap().node_count()
}

/// A session with versions "One" (one box) and "Two" (two), then a third box and a selection.
fn setup() -> (Session, figcraft_doc::NodeId) {
    let mut s = Session::new();
    rect(&mut s, 0.0);
    s.execute("file.saveVersion", &json!({"name": "One"})).unwrap();
    rect(&mut s, 20.0);
    s.execute("file.saveVersion", &json!({"name": "Two"})).unwrap();
    let r = rect(&mut s, 40.0);
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    (s, r)
}

#[test]
fn preview_shows_the_version_read_only_and_goes_back() {
    let (mut s, r) = setup();
    let three = count(&s);
    let undo_depth = s.ds().unwrap().history.undo.len();
    let v = s.execute("version.preview", &json!({"index": 0})).unwrap();
    assert_eq!(v["name"], "One");
    assert_eq!(count(&s), three - 2);
    assert!(s.selection().is_empty());
    let l = s.execute("version.list", &json!({})).unwrap();
    assert_eq!(l[0]["previewing"], true);
    assert!(l[1].get("previewing").is_none());
    // Edits are refused and leave it as it was; so do undo and gestures.
    let err = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap_err();
    assert!(matches!(err, EngineError::Disabled(..)), "{err:?}");
    assert_eq!(count(&s), three - 2);
    assert!(s.execute("edit.undo", &json!({})).is_err());
    assert_eq!(s.ds().unwrap().history.undo.len(), undo_depth, "history untouched");
    assert!(s.begin_interaction("Move").is_err());
    let _ = s.pointer(&PointerEvent::at(PointerKind::Down, 5.0, 5.0, 1.0));
    let _ = s.pointer(&PointerEvent::at(PointerKind::Move, 50.0, 50.0, 1.0));
    let _ = s.pointer(&PointerEvent::at(PointerKind::Up, 50.0, 50.0, 1.0));
    assert_eq!(count(&s), three - 2);
    // Switching to another version, then back to the current one.
    s.execute("version.preview", &json!({"index": 1})).unwrap();
    assert_eq!(count(&s), three - 1);
    assert_eq!(s.execute("version.exitPreview", &json!({})).unwrap()["ended"], true);
    assert_eq!(count(&s), three);
    assert_eq!(s.selection(), vec![r], "selection comes back");
    assert_eq!(s.execute("version.exitPreview", &json!({})).unwrap()["ended"], false);
    // Editing works again.
    rect(&mut s, 60.0);
    assert_eq!(count(&s), three + 1);
}

#[test]
fn restore_from_a_preview_is_one_undo_step() {
    let (mut s, _) = setup();
    let three = count(&s);
    s.execute("version.preview", &json!({"index": 0})).unwrap();
    s.execute("version.restore", &json!({"index": 0})).unwrap();
    assert!(s.ds().unwrap().preview.is_none());
    assert_eq!(count(&s), three - 2);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(count(&s), three, "undo goes back to the document, not the preview");
}

#[test]
fn preview_keeps_the_unsaved_state_and_saving_saves_the_document() {
    let dir = std::env::temp_dir().join(format!("figcraft-vpreview-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("preview.figcraft").to_string_lossy().to_string();
    let (mut s, _) = setup();
    let three = count(&s);
    s.execute("file.save", &json!({"path": path})).unwrap();
    s.execute("version.preview", &json!({"index": 0})).unwrap();
    assert!(!s.ds().unwrap().is_dirty(), "looking at a version isn't a change");
    // Saving ends the preview and saves the document itself.
    s.execute("file.save", &json!({})).unwrap();
    assert!(s.ds().unwrap().preview.is_none());
    let mut t = Session::empty();
    t.execute("file.open", &json!({"path": path})).unwrap();
    assert_eq!(count(&t), three);
    // Deleting the version being viewed ends the preview.
    s.execute("version.preview", &json!({"index": 1})).unwrap();
    s.execute("version.delete", &json!({"index": 1})).unwrap();
    assert!(s.ds().unwrap().preview.is_none());
    assert_eq!(count(&s), three);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_refused_cut_leaves_the_clipboard_as_it_was() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0);
    s.execute("file.saveVersion", &json!({"name": "One"})).unwrap();
    let b = rect(&mut s, 20.0);
    s.execute("edit.copy", &json!({"ids": [b]})).unwrap();
    let roots = |s: &Session| s.clipboard.nodes.iter().map(|(r, _)| *r).collect::<Vec<_>>();
    assert_eq!(roots(&s), vec![b]);
    let source = s.clipboard.source.clone().unwrap();
    let depth = s.undo_depth();
    s.execute("version.preview", &json!({"index": 0})).unwrap();
    let err = s.execute("edit.cut", &json!({"ids": [a]})).unwrap_err();
    assert!(matches!(err, EngineError::Disabled(..)), "{err:?}");
    assert_eq!(roots(&s), vec![b], "failed edit.cut changed the clipboard");
    assert!(std::sync::Arc::ptr_eq(s.clipboard.source.as_ref().unwrap(), &source));
    assert_eq!(s.undo_depth(), depth, "history untouched");
}
