//! The keyboard, pointer and typing methods (`ui.key`, `ui.pointer`, `ui.type`) are made under the
//! agent's name like `engine.execute`: with another agent at work, its Cmd+Z is refused as its
//! `edit.undo` is, and the user's never is.

use figcraft_mcp::{Backend, Headless, name_agent};
use serde_json::{Value, json};

/// `method` as the MCP server sends it for the agent `name` (key `key-<name>`).
fn as_agent(h: &mut Headless, method: &str, mut p: Value, name: &str) -> Result<Value, String> {
    name_agent(method, &mut p, name, &format!("key-{name}"));
    h.call(method, p)
}

/// A session where agents `a` and `b` are there and `b` drew a rectangle: the history holds an
/// edit of `b`'s, which an undo of `a` would take away.
fn two_agents() -> Headless {
    let mut h = Headless::new();
    for name in ["a", "b"] {
        as_agent(&mut h, "agent.join", json!({"agent": name}), name).unwrap();
    }
    let rect = json!({"command": "shape.rectangle", "params": {"x": 0, "y": 0, "width": 10, "height": 10}});
    as_agent(&mut h, "engine.execute", rect, "b").unwrap();
    h
}

fn drawn(h: &mut Headless) -> usize {
    let doc = h.call("document.inspect", json!({})).unwrap();
    doc["pages"][0]["children"].as_array().map_or(0, Vec::len)
}

/// Every method of the UI that acts carries the agent's name and key to the backend.
#[test]
fn the_ui_methods_carry_the_agents_name_and_key() {
    for method in ["ui.key", "ui.type", "ui.input", "ui.pointer", "ui.menu.invoke"] {
        let mut p = json!({});
        name_agent(method, &mut p, "a", "k");
        assert_eq!((p["agent"].as_str(), p["agentKey"].as_str()), (Some("a"), Some("k")), "{method}: {p}");
    }
    let mut p = json!({});
    name_agent("ui.inspect", &mut p, "a", "k");
    assert_eq!(p, json!({}), "a method that only reads is left as it is");
}

/// Cmd+Z by key, from `a` while `b` is there: refused, `b`'s rectangle stays. This is the path
/// `ui.key` -> `Headless::key` -> `execute("edit.undo")`, which ran without the agent's name.
#[test]
fn an_agents_undo_key_is_refused_while_another_agent_is_there() {
    let mut h = two_agents();
    assert_eq!(drawn(&mut h), 1, "b's rectangle is in the history");
    let e = as_agent(&mut h, "ui.key", json!({"key": "Z", "cmd": true}), "a").unwrap_err();
    assert!(e.contains("history is shared"), "{e}");
    assert_eq!(drawn(&mut h), 1, "b's rectangle is still there");
    let e = as_agent(&mut h, "ui.key", json!({"key": "Z", "cmd": true, "shift": true}), "a").unwrap_err();
    assert!(e.contains("history is shared"), "redo: {e}");
}

/// The user's Cmd+Z (no agent) is never refused, with agents at work.
#[test]
fn the_users_undo_key_still_works_while_agents_are_there() {
    let mut h = two_agents();
    h.call("ui.key", json!({"key": "Z", "cmd": true})).unwrap();
    assert_eq!(drawn(&mut h), 0, "the user's undo took b's rectangle away, as it may");
}

/// A key under a name another client holds is refused before it does anything.
#[test]
fn a_ui_key_under_a_held_name_needs_its_key() {
    let mut h = two_agents();
    let e = h.call("ui.key", json!({"key": "Z", "cmd": true, "agent": "a", "agentKey": "not a's"})).unwrap_err();
    assert!(e.contains("held by another client"), "{e}");
    assert_eq!(drawn(&mut h), 1);
}
