//! `type_text` for an agent working apart (the PR 2281 H2 defect): the text editor of an agent
//! apart is kept in its own slot, so typing through `ui.type` (which reads the window's editor)
//! failed with "not editing text". `type_text` types through `text.edit.insert`, a command run
//! under the agent's name like `text.edit.begin`.

use std::{cell::RefCell, rc::Rc};

use figcraft_mcp::{Backend, Headless, Server, name_agent};
use serde_json::{Value, json};

/// One `Headless` that the MCP server (agent `a`) and the test (agent `b`) both reach.
struct Shared(Rc<RefCell<Headless>>);

impl Backend for Shared {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.0.borrow_mut().call(method, params)
    }
    fn has_ui(&self) -> bool {
        self.0.borrow().has_ui()
    }
    fn describe(&self) -> String {
        self.0.borrow().describe()
    }
}

fn rpc(s: &mut Server, method: &str, params: Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
    serde_json::from_str(&s.handle_line(&line).unwrap()).unwrap()
}

fn tool(s: &mut Server, name: &str, args: Value) -> Value {
    let r = rpc(s, "tools/call", json!({"name": name, "arguments": args}));
    r["result"].clone()
}

fn ok(s: &mut Server, name: &str, args: Value) -> Value {
    let r = tool(s, name, args.clone());
    assert_eq!(r["isError"], false, "{name} {args}: {}", r["content"][0]["text"]);
    serde_json::from_str(r["content"][0]["text"].as_str().unwrap_or("")).unwrap_or(Value::Null)
}

#[test]
fn type_text_reaches_the_editor_of_an_agent_apart() {
    let shared = Rc::new(RefCell::new(Headless::new()));
    let mut s = Server::new(Box::new(Shared(shared.clone()))).with_agent("a");
    let r = rpc(&mut s, "initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}));
    assert!(r.get("result").is_some(), "{r}");
    assert!(s.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());

    // Agent `b` is there too, joined by the test; agent `a` joins through the MCP tool.
    let mut p = json!({"agent": "b"});
    name_agent("agent.join", &mut p, "b", "key-b");
    shared.borrow_mut().call("agent.join", p).unwrap();
    ok(&mut s, "agent_join", json!({}));
    // Guard: both are apart, else the editor stays in the window and this test proves nothing.
    let listed = ok(&mut s, "agent_list", json!({}));
    let agents = listed["agents"].as_array().unwrap();
    for name in ["a", "b"] {
        let found = agents.iter().find(|x| x["agent"] == name);
        assert_eq!(found.map(|x| x["apart"].clone()), Some(json!(true)), "{name} must be apart: {listed}");
    }

    let node = ok(&mut s, "create_node", json!({"type": "text", "x": 0, "y": 0, "text": "Hi"}));
    let id = node["id"].clone();
    let r = tool(&mut s, "type_text", json!({"id": id, "text": "Hello"}));
    assert_eq!(r["isError"], false, "type_text by an agent apart failed: {}", r["content"][0]["text"]);
    let n = ok(&mut s, "get_node", json!({"id": id, "depth": 0}));
    assert_eq!(n["characters"], "Hello", "{n}");
}
