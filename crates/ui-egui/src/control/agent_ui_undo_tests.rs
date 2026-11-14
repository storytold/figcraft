//! What an undo through `ui.key` and `ui.menu.invoke` still allows: an agent alone may undo its
//! own work, and the user's undo is never refused with agents at work. The refusals of another
//! agent's undo, by every door, are in `agent_doors_tests`.

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
