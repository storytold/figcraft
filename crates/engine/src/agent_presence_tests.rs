//! An attached AI agent's presence (#1416): its commands set it, on what they edited or selected;
//! it goes a short while after its last command; the user's commands never make one.

use std::time::Duration;

use figcraft_doc::NodeId;
use serde_json::{Value, json};

use crate::Session;
use crate::agent::LINGER;

fn id(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rect(s: &mut Session, agent: Option<&str>, x: i32) -> NodeId {
    id(&s.execute_as(agent, "shape.rectangle", &json!({"x": x, "y": 0, "width": 40, "height": 40})).unwrap())
}

#[test]
fn an_agents_edit_shows_it_on_the_layers_it_changed() {
    let mut s = Session::new();
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 300, "height": 200})).unwrap());
    let r = id(&s.execute_as(Some("Claude"), "shape.rectangle", &json!({"parent": f, "x": 10, "y": 10, "width": 40, "height": 40})).unwrap());
    let p = s.agent_presence.clone().expect("the agent is at work");
    assert_eq!(p.name, "Claude (agent)");
    assert_eq!(p.doc, s.ds().unwrap().uid);
    assert_eq!(p.layers, vec![r], "the new layer, not the frame whose children changed with it");
    assert_eq!(s.agent, None, "the command ran as the agent only");
}

#[test]
fn an_agents_selection_shows_it_there_and_a_look_keeps_it() {
    let mut s = Session::new();
    let (a, b) = (rect(&mut s, None, 0), rect(&mut s, None, 100));
    s.execute_as(Some(""), "select.set", &json!({"ids": [a, b]})).unwrap();
    let p = s.agent_presence.clone().unwrap();
    assert_eq!((p.name.as_str(), p.layers.clone()), ("AI agent", vec![a, b]));
    // A command that changes nothing keeps where it works and when it was last seen moves on.
    s.execute_as(Some(""), "view.zoomFit", &json!({})).unwrap();
    let again = s.agent_presence.clone().unwrap();
    assert_eq!(again.layers, vec![a, b]);
    assert!(again.at >= p.at);
    // So does a failed one.
    assert!(s.execute_as(Some(""), "node.set", &json!({"ids": [999_999], "props": {}})).is_err());
    assert_eq!(s.agent_presence.as_ref().unwrap().layers, vec![a, b]);
}

#[test]
fn the_presence_goes_a_short_while_after_the_last_command() {
    let mut s = Session::new();
    rect(&mut s, Some("Claude"), 0);
    let at = s.agent_presence.as_ref().unwrap().at;
    assert!(s.agent_presence_at(at).is_some());
    assert!(s.agent_presence_at(at + LINGER - Duration::from_millis(1)).is_some());
    assert_eq!(s.agent_presence.as_ref().unwrap().left(at + Duration::from_secs(2)), LINGER - Duration::from_secs(2));
    assert!(s.agent_presence_at(at + LINGER).is_none(), "gone after the linger");
}

#[test]
fn the_users_commands_never_make_one() {
    let mut s = Session::new();
    let a = rect(&mut s, None, 0);
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    s.execute("node.set", &json!({"ids": [a], "props": {"x": 50}})).unwrap();
    assert!(s.agent_presence.is_none());
    // An agent's name on one file: the user's later edits elsewhere don't move it.
    rect(&mut s, Some("Claude"), 200);
    let before = s.agent_presence.clone();
    s.execute("node.set", &json!({"ids": [a], "props": {"x": 80}})).unwrap();
    assert_eq!(s.agent_presence, before);
}

#[test]
fn another_file_starts_afresh() {
    let mut s = Session::new();
    let a = rect(&mut s, Some("Claude"), 0);
    assert_eq!(s.agent_presence.as_ref().unwrap().layers, vec![a]);
    s.execute("file.new", &json!({})).unwrap();
    s.execute_as(Some("Claude"), "view.zoomFit", &json!({})).unwrap();
    let p = s.agent_presence.clone().unwrap();
    assert_eq!(p.doc, s.ds().unwrap().uid);
    assert!(p.layers.is_empty(), "the old file's layers don't follow it");
}
