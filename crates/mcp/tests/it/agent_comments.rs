//! An agent attached over MCP comments under its own name (#1414): the client's name from
//! `initialize` (`clientInfo.title`, else `.name`), marked as an agent's, or "AI agent".

use figcraft_mcp::Server;
use serde_json::{Value, json};

use crate::common::Client;

fn rpc(s: &mut Server, method: &str, params: Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
    serde_json::from_str(&s.handle_line(&line).unwrap()).unwrap()
}

fn run(s: &mut Server, command: &str, params: Value) -> Value {
    let r = rpc(s, "tools/call", json!({"name": "run_command", "arguments": {"command": command, "params": params}}));
    assert_eq!(r["result"]["isError"], false, "{r}");
    serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

/// The first message's (author, agent marker) after the server comments once.
fn signed(s: &mut Server) -> (String, Value) {
    run(s, "comment.add", json!({"x": 10, "y": 10, "text": "Made it bigger"}));
    let t = run(s, "comment.list", json!({"filter": "all"}))["comments"][0].clone();
    (t["author"].as_str().unwrap().to_string(), t["agent"].clone())
}

#[test]
fn an_agent_comments_under_its_clients_name() {
    let mut s = Server::headless();
    rpc(
        &mut s,
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "claude-code", "title": "Claude", "version": "1"}}),
    );
    assert_eq!(signed(&mut s), ("Claude (agent)".into(), json!(true)), "the title wins over the name");
    let mut c = Client::new();
    c.run("comment.add", json!({"x": 0, "y": 0, "text": "Hi"}));
    let t = c.run("comment.list", json!({}))["comments"][0].clone();
    assert_eq!((t["author"].clone(), t["agent"].clone()), (json!("test (agent)"), json!(true)), "the name without a title");
    assert_eq!(c.run("comment.people", json!({}))["you"], "test (agent)", "the agent is itself on every call");
}

#[test]
fn an_agent_without_a_name_is_still_an_agent() {
    let mut s = Server::headless();
    assert_eq!(signed(&mut s), ("AI agent".into(), json!(true)), "no initialize");
    let mut s = Server::headless();
    rpc(&mut s, "initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "  ", "version": "1"}}));
    assert_eq!(signed(&mut s), ("AI agent".into(), json!(true)), "a blank name");
}
