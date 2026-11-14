//! A shortcut or a menu run through `ui.key` or `ui.menu.invoke` for an agent is made under its
//! name, as `engine.execute` is: with another agent at work, its Cmd+Z is refused as its
//! `edit.undo` is. The user's, with no agent named, never is.

use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::control::handle;
use crate::layers_drag_tests::Ui;

fn call(u: &mut Ui, method: &str, params: Value) -> Result<Value, String> {
    let ctx = u.ctx.clone();
    handle(&mut u.app, &ctx, method, &params)
}

/// `a` and `b` are there, with their keys, and `b` drew a rectangle: the history holds an edit of
/// `b`'s, which an undo of `a` would take away.
fn two_agents() -> Ui {
    let mut u = Ui::new(Session::new());
    for name in ["a", "b"] {
        call(&mut u, "agent.join", json!({"agent": name, "agentKey": format!("key {name}")})).unwrap();
    }
    let rect = json!({"agent": "b", "agentKey": "key b", "command": "shape.rectangle", "params": {"x": 0, "y": 0, "width": 10, "height": 10}});
    call(&mut u, "engine.execute", rect).unwrap();
    u
}

fn drawn(u: &mut Ui) -> usize {
    let doc = call(u, "document.inspect", json!({})).unwrap();
    doc["pages"][0]["children"].as_array().map_or(0, Vec::len)
}

/// Before an interface entry: `a` and `b` are both there and the last step of the history is the
/// rectangle `b` drew (the only edit made), which an undo would take away.
fn calibrate(u: &mut Ui) {
    let list = call(u, "agent.list", json!({})).unwrap();
    assert_eq!(list["agents"].as_array().map(Vec::len), Some(2), "a and b are both present: {list}");
    assert_eq!(drawn(u), 1, "b's rectangle is in the document");
    assert_eq!(u.app.session.undo_labels(2).len(), 1, "b's rectangle is the only step, so the last one");
}

/// An undo of `a` through `method` is refused with `b` there, and `b`'s rectangle is intact.
fn undo_refused(method: &str, p: Value) {
    let mut u = two_agents();
    calibrate(&mut u);
    let r = call(&mut u, method, p);
    assert!(r.as_ref().is_err_and(|e| e.contains("shared by everyone")), "{method}: a's undo was not refused: {r:?}");
    assert_eq!(drawn(&mut u), 1, "b's rectangle is still there");
    assert_eq!(u.app.session.undo_labels(2).len(), 1, "the history is untouched");
}

/// `a`'s Cmd+Z on the keyboard path (`ui.key` -> `run_binding` -> `edit.undo`, which ran with no
/// agent): refused while `b` is there, `b`'s rectangle stays.
#[test]
fn an_agents_undo_key_is_refused_while_another_agent_is_there() {
    undo_refused("ui.key", json!({"agent": "a", "agentKey": "key a", "key": "Z", "cmd": true}));
}

/// The same undo from the Edit menu (`ui.menu.invoke` -> `menus::invoke` -> `app.run`).
#[test]
fn an_agents_undo_from_the_menu_is_refused_while_another_agent_is_there() {
    undo_refused("ui.menu.invoke", json!({"agent": "a", "agentKey": "key a", "id": "edit.undo"}));
}

/// The same undo as raw key events through `ui.input`: refused before anything is queued.
#[test]
fn an_agents_undo_in_raw_input_is_refused_while_another_agent_is_there() {
    let mut u = two_agents();
    calibrate(&mut u);
    let p = json!({"agent": "a", "agentKey": "key a", "events": [{"key": "Z", "cmd": true}]});
    let r = call(&mut u, "ui.input", p);
    assert!(r.as_ref().is_err_and(|e| e.contains("shared by everyone")), "a's raw Cmd+Z was not refused: {r:?}");
    assert!(u.app.control_input.is_empty(), "nothing of the batch was queued");
    assert_eq!(drawn(&mut u), 1, "b's rectangle is still there");
}

/// An agent alone is free to undo: the rule is about another agent's work.
#[test]
fn an_agent_alone_may_undo_by_key() {
    let mut u = Ui::new(Session::new());
    call(&mut u, "agent.join", json!({"agent": "a", "agentKey": "key a"})).unwrap();
    let rect = json!({"agent": "a", "agentKey": "key a", "command": "shape.rectangle", "params": {"x": 0, "y": 0, "width": 10, "height": 10}});
    call(&mut u, "engine.execute", rect).unwrap();
    assert_eq!(drawn(&mut u), 1);
    call(&mut u, "ui.key", json!({"agent": "a", "agentKey": "key a", "key": "Z", "cmd": true})).unwrap();
    assert_eq!(drawn(&mut u), 0, "a alone undid its own rectangle");
}

/// The user's Cmd+Z (no agent), and its menu, are never refused with agents at work.
#[test]
fn the_users_undo_key_and_menu_still_work_while_agents_are_there() {
    let mut u = two_agents();
    call(&mut u, "ui.key", json!({"key": "Z", "cmd": true})).unwrap();
    assert_eq!(drawn(&mut u), 0, "the user's undo took b's rectangle away, as it may");
    call(&mut u, "ui.menu.invoke", json!({"id": "edit.redo"})).unwrap();
    assert_eq!(drawn(&mut u), 1);
}

/// A `ui.*` request under a name another client holds is refused before it does anything.
#[test]
fn a_ui_request_under_a_held_name_needs_its_key() {
    let mut u = two_agents();
    for (method, p) in [
        ("ui.key", json!({"key": "Z", "cmd": true})),
        ("ui.menu.invoke", json!({"id": "edit.undo"})),
        ("ui.type", json!({"text": "x"})),
        ("ui.input", json!({"events": [{"key": "Z", "cmd": true}]})),
        ("ui.pointer", json!({"kind": "click", "x": 5, "y": 5})),
    ] {
        let mut p = p;
        p["agent"] = json!("a");
        p["agentKey"] = json!("not a's");
        let r = call(&mut u, method, p);
        assert!(r.as_ref().is_err_and(|e| e.contains("held by another client")), "{method}: {r:?}");
    }
    assert_eq!(drawn(&mut u), 1);
}
