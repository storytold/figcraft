//! On the channel the user opened in Preferences, nothing a client launches drops the user's
//! unsaved changes: `file.close {force: true}` and `file.revert` are refused on a modified document.

use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::control::handle;
use crate::layers_drag_tests::Ui;

fn call(u: &mut Ui, method: &str, params: Value) -> Result<Value, String> {
    let ctx = u.ctx.clone();
    handle(&mut u.app, &ctx, method, &params)
}

fn draw(s: &mut Session) {
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
}

/// Two documents, the second one (the returned index) modified.
fn two_documents() -> (Ui, usize) {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    draw(&mut s);
    let index = s.active;
    assert_eq!(s.docs.len(), 2);
    (Ui::new(s), index)
}

/// Whether the document at `index` is still open and modified.
fn open_and_modified(u: &Ui, index: usize) -> bool {
    u.app.session.docs.len() == 2 && u.app.session.docs.get(index).is_some_and(|d| d.is_dirty())
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("figcraft-unsaved-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn a_client_of_the_channel_opened_in_preferences_cannot_close_unsaved_work_by_force() {
    let (mut u, index) = two_documents();
    let answer = call(&mut u, "engine.execute", json!({"command": "file.close", "params": {"index": index, "force": true}}));
    assert!(answer.is_err(), "a control client dropped the user's unsaved changes: {answer:?}");
    assert!(open_and_modified(&u, index), "the document stays open and modified");
}

#[test]
fn nor_as_a_step_of_a_batch() {
    let (mut u, index) = two_documents();
    let batch = json!({"commands": [{"command": "file.close", "params": {"index": index, "force": true}}]});
    let answer = call(&mut u, "engine.execute", json!({"command": "command.batch", "params": batch}));
    assert!(answer.is_err(), "a control client dropped the user's unsaved changes in a batch: {answer:?}");
    assert!(open_and_modified(&u, index), "the document stays open and modified");
}

#[test]
fn nor_revert_it_to_the_saved_file() {
    let d = temp_dir("revert");
    let mut s = Session::new();
    s.execute("file.saveAs", &json!({"path": d.join("a.figcraft").to_string_lossy()})).unwrap();
    draw(&mut s);
    let mut u = Ui::new(s);
    let dirty = |u: &Ui| u.app.session.docs.iter().any(|doc| doc.is_dirty());
    assert!(dirty(&u), "the document starts modified");

    let answer = call(&mut u, "engine.execute", json!({"command": "file.revert", "params": {}}));
    assert!(answer.is_err() && dirty(&u), "a control client reverted the user's unsaved changes: {answer:?}");

    // The menu answers `Ok` with `lastError`; only the document's state matters.
    let _ = call(&mut u, "ui.menu.invoke", json!({"id": "file.revert"}));
    assert!(dirty(&u), "a menu invoked by a control client reverted the user's unsaved changes");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn a_frame_that_applies_injected_input_cannot_close_unsaved_work_by_force() {
    let (mut u, index) = two_documents();
    u.app.control_synthetic = true;
    let injected = u.app.run("file.close", json!({"index": index, "force": true}));
    assert!(injected.is_none(), "an injected input dropped the user's unsaved changes: {injected:?}");
    assert!(open_and_modified(&u, index), "the document stays open and modified");

    u.app.control_synthetic = false;
    let own = u.app.run("file.close", json!({"index": index, "force": true}));
    assert!(own.is_some() && u.app.session.docs.len() == 1, "the user's own Don't Save still closes: {own:?}");
}

/// A guard, not the proof of a defect (it passes without the fix): a launch that named its
/// control port keeps its right to close by force.
#[test]
fn a_launch_that_named_its_port_still_closes_by_force() {
    let (mut u, index) = two_documents();
    u.app.agent_control_fixed = true;
    let answer = call(&mut u, "engine.execute", json!({"command": "file.close", "params": {"index": index, "force": true}}));
    assert!(answer.is_ok() && u.app.session.docs.len() == 1, "{answer:?}");
}
