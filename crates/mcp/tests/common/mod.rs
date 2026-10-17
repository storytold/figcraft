//! An in-process MCP client: feeds JSON-RPC lines to `Server::handle_line`.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use figcraft_mcp::Server;
use serde_json::{Value, json};

pub struct Client {
    pub server: Server,
    next: u64,
}

impl Client {
    /// A headless server, initialized.
    pub fn new() -> Self {
        Self::with_server(Server::headless())
    }

    /// `server`, initialized.
    pub fn with_server(server: Server) -> Self {
        let mut c = Self { server, next: 1 };
        let r = c.rpc("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}));
        assert!(r.get("result").is_some(), "initialize failed: {r}");
        assert!(c.server.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
        c
    }

    /// Send a request; returns the whole response object.
    pub fn rpc(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
        let reply = self.server.handle_line(&line).expect("a request gets a reply");
        let v: Value = serde_json::from_str(&reply).expect("reply is JSON");
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], json!(id));
        v
    }

    /// tools/call; returns the result ({content, isError}).
    pub fn call(&mut self, tool: &str, args: Value) -> Value {
        let r = self.rpc("tools/call", json!({"name": tool, "arguments": args}));
        r.get("result").cloned().unwrap_or_else(|| panic!("tools/call {tool} protocol error: {r}"))
    }

    /// tools/call that must succeed; returns the first text block parsed as JSON (or a string).
    pub fn ok(&mut self, tool: &str, args: Value) -> Value {
        let r = self.call(tool, args.clone());
        assert_eq!(r["isError"], false, "{tool} {args} failed: {}", r["content"][0]["text"]);
        let text = r["content"].as_array().unwrap().iter().find_map(|c| c.get("text").and_then(Value::as_str)).unwrap_or("");
        serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string()))
    }

    /// tools/call that must fail; returns the error text.
    pub fn err(&mut self, tool: &str, args: Value) -> String {
        let r = self.call(tool, args.clone());
        assert_eq!(r["isError"], true, "{tool} {args} should fail: {r}");
        r["content"][0]["text"].as_str().unwrap_or("").to_string()
    }

    pub fn run(&mut self, command: &str, params: Value) -> Value {
        self.ok("run_command", json!({"command": command, "params": params}))
    }

    pub fn node(&mut self, id: &Value) -> Value {
        self.ok("get_node", json!({"id": id, "depth": 3}))
    }

    pub fn create(&mut self, args: Value) -> Value {
        self.ok("create_node", args)["id"].clone()
    }

    pub fn doc(&mut self) -> Value {
        self.ok("inspect_document", json!({"depth": 6}))
    }

    /// Top-level layers of the current page.
    pub fn page_children(&mut self) -> Vec<Value> {
        let d = self.doc();
        let cur = d["currentPage"].clone();
        let page = d["pages"].as_array().unwrap().iter().find(|p| p["id"] == cur).unwrap().clone();
        page["children"].as_array().cloned().unwrap_or_default()
    }
}

pub fn approx(v: &Value, want: f64) -> bool {
    v.as_f64().is_some_and(|x| (x - want).abs() < 0.51)
}

pub fn png_dims(b64: &str) -> (u32, u32) {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
    figcraft_mcp::png_size(&bytes).unwrap()
}
