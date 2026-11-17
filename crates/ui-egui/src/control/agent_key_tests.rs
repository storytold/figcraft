//! A named agent holds its name with a key (`agentKey`) that only its client knows: no other
//! client of the control channel acts under that name, or makes it leave.

use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::control::handle;
use crate::layers_drag_tests::Ui;

fn call(u: &mut Ui, method: &str, params: Value) -> Result<Value, String> {
    let ctx = u.ctx.clone();
    handle(&mut u.app, &ctx, method, &params)
}

/// A rectangle drawn under `agent`'s name, by a client that gives `key`.
fn rectangle(agent: &str, key: Option<&str>) -> Value {
    let mut p = json!({"agent": agent, "command": "shape.rectangle", "params": {"x": 0, "y": 0, "width": 10, "height": 10}});
    if let Some(key) = key {
        p["agentKey"] = json!(key);
    }
    p
}

/// The agents `agent.list` says are there.
fn there(u: &mut Ui) -> Vec<String> {
    let listed = call(u, "agent.list", json!({})).unwrap();
    listed["agents"].as_array().unwrap().iter().filter_map(|a| a["agent"].as_str().map(str::to_owned)).collect()
}

/// `a` joined with its key. A client that gives another key, or none, is not `a`: its command
/// under that name is refused.
#[test]
fn only_its_own_client_acts_under_a_held_name() {
    let mut u = Ui::new(Session::new());
    call(&mut u, "agent.join", json!({"agent": "a", "agentKey": "the key of a"})).unwrap();

    let other = call(&mut u, "engine.execute", rectangle("a", Some("another key")));
    assert!(other.is_err(), "a client with another key drew under a's name: {other:?}");
    let none = call(&mut u, "engine.execute", rectangle("a", None));
    assert!(none.is_err(), "a client with no key drew under a's name: {none:?}");

    let own = call(&mut u, "engine.execute", rectangle("a", Some("the key of a")));
    assert!(own.is_ok(), "a itself still works: {own:?}");
}

/// `b` cannot make `a` leave: `a` would lose its pages and its selections. With its key `a`
/// leaves, and its name is free again.
#[test]
fn only_its_own_client_makes_a_held_name_leave() {
    let mut u = Ui::new(Session::new());
    call(&mut u, "agent.join", json!({"agent": "a", "agentKey": "the key of a"})).unwrap();
    call(&mut u, "agent.join", json!({"agent": "b", "agentKey": "the key of b"})).unwrap();

    let by_b = call(&mut u, "agent.leave", json!({"agent": "a", "agentKey": "the key of b"}));
    assert!(by_b.is_err(), "b made a leave: {by_b:?}");
    let by_none = call(&mut u, "agent.leave", json!({"agent": "a"}));
    assert!(by_none.is_err(), "a client with no key made a leave: {by_none:?}");
    assert_eq!(there(&mut u), ["a", "b"], "a is still there");

    call(&mut u, "agent.leave", json!({"agent": "a", "agentKey": "the key of a"})).unwrap();
    assert_eq!(there(&mut u), ["b"]);
    call(&mut u, "agent.join", json!({"agent": "a", "agentKey": "the key of a new client"})).unwrap();
}

/// `agent.join` is how an agent takes its name: without a key the name would be anyone's.
#[test]
fn joining_takes_a_key() {
    let mut u = Ui::new(Session::new());
    let keyless = call(&mut u, "agent.join", json!({"agent": "a"}));
    assert!(keyless.is_err(), "a name was joined without a key: {keyless:?}");
    assert!(there(&mut u).is_empty(), "and nobody came");
}
