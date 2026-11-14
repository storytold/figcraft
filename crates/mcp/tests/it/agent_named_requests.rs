//! Every request the MCP server makes for its agent carries the agent's name and key, whatever
//! builds it: a tool, a resource, or a render the backend makes by requests of its own.

use std::cell::RefCell;
use std::rc::Rc;

use figcraft_mcp::{Backend, Server};
use serde_json::{Value, json};

type Calls = Rc<RefCell<Vec<(String, Value)>>>;

/// A backend that answers each method with something the tools can read, and keeps what it was
/// asked. `ui` is whether it claims a window (a window capture is then a request of its own).
struct Recording {
    calls: Calls,
    ui: bool,
}

impl Backend for Recording {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.calls.borrow_mut().push((method.into(), params));
        Ok(match method {
            "document.inspect" => json!({"currentPage": 1, "pages": [{"id": 1, "children": [{"id": 7}]}], "selection": []}),
            "ui.render" | "ui.screenshot" => json!({"data": "AAAA", "width": 1, "height": 1}),
            _ => json!({}),
        })
    }
    fn has_ui(&self) -> bool {
        self.ui
    }
    fn describe(&self) -> String {
        "recording".into()
    }
}

fn rpc(s: &mut Server, method: &str, params: Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
    serde_json::from_str(&s.handle_line(&line).unwrap()).unwrap()
}

/// A server for the agent `a` over a recording backend, initialized.
fn server(ui: bool) -> (Server, Calls) {
    let calls = Calls::default();
    let mut s = Server::new(Box::new(Recording { calls: calls.clone(), ui })).with_agent("a");
    let r = rpc(&mut s, "initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}));
    assert!(r.get("result").is_some(), "{r}");
    assert!(s.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
    (s, calls)
}

/// Every request made while the server runs `tools` and reads `resources`, in order. A tool may
/// fail on the canned answers: only what it asked is looked at.
fn requests(ui: bool, tools: &[(&str, Value)], resources: &[&str]) -> Vec<(String, Value)> {
    let (mut s, calls) = server(ui);
    for (name, args) in tools {
        rpc(&mut s, "tools/call", json!({"name": name, "arguments": args}));
    }
    for uri in resources {
        rpc(&mut s, "resources/read", json!({"uri": uri}));
    }
    calls.take()
}

/// What the sweep of every tool and resource asks of the backend, with and without a window: the
/// requests of each of the two servers it starts, one list per server (each has its own key).
pub(crate) fn sweep() -> Vec<Vec<(String, Value)>> {
    let tools = [
        ("run_command", json!({"command": "select.all", "params": {}})),
        ("inspect_document", json!({})),
        ("get_node", json!({"id": 7})),
        ("pointer", json!({"kind": "hover", "x": 0, "y": 0})),
        ("key", json!({"key": "A"})),
        ("type_text", json!({"text": "x"})),
        ("screenshot", json!({})),
        ("screenshot", json!({"ids": [7]})),
        ("export", json!({"ids": [7], "format": "png"})),
        ("undo", json!({})),
    ];
    let resources = ["figcraft://document", "figcraft://commands", "figcraft://comments", "figcraft://node/7", "figcraft://command/layout.add"];
    vec![requests(false, &tools, &resources), requests(true, &[("screenshot", json!({"window": true})), ("screenshot", json!({"window": 3}))], &[])]
}

fn named(calls: &[(String, Value)], method: &str) -> Vec<bool> {
    calls.iter().filter(|(m, _)| m == method).map(|(_, p)| p["agent"] == "a" && p["agentKey"].as_str().is_some_and(|k| !k.is_empty())).collect()
}

/// A render of given layers is a `ui.render` request of its own: named like the others.
#[test]
fn a_render_the_backend_makes_by_requests_names_the_agent() {
    let calls = requests(false, &[("screenshot", json!({"ids": [7]}))], &[]);
    let renders = named(&calls, "ui.render");
    assert!(!renders.is_empty(), "the screenshot made no ui.render request: {calls:?}");
    assert!(renders.iter().all(|n| *n), "an unnamed ui.render: {calls:?}");
}

/// The capture of the window, and of one window by id, are named too.
#[test]
fn a_window_capture_names_the_agent() {
    for window in [json!(true), json!(3)] {
        let calls = requests(true, &[("screenshot", json!({"window": window}))], &[]);
        let captures = named(&calls, "ui.screenshot");
        assert!(!captures.is_empty(), "no ui.screenshot request for {window}: {calls:?}");
        assert!(captures.iter().all(|n| *n), "an unnamed ui.screenshot for {window}: {calls:?}");
    }
}

/// The layers of the page, asked to render the page without ids, are asked as the agent too.
#[test]
fn the_page_layers_of_a_render_are_asked_as_the_agent() {
    let calls = requests(false, &[("screenshot", json!({}))], &[]);
    let inspects = named(&calls, "document.inspect");
    assert!(!inspects.is_empty(), "no document.inspect request: {calls:?}");
    assert!(inspects.iter().all(|n| *n), "an unnamed document.inspect: {calls:?}");
}
