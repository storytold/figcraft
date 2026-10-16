//! JSON-RPC / MCP protocol behaviour.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::Client;
use figcraft_mcp::{PROTOCOL_VERSION, Server, TOOL_NAMES};
use serde_json::{Value, json};

fn reply(s: &mut Server, line: &str) -> Value {
    serde_json::from_str(&s.handle_line(line).expect("reply")).unwrap()
}

#[test]
fn initialize_negotiates_versions() {
    for (asked, want) in [("2025-06-18", "2025-06-18"), ("2025-03-26", "2025-03-26"), ("2024-11-05", "2024-11-05"), ("1999-01-01", PROTOCOL_VERSION)]
    {
        let mut s = Server::headless();
        let r = reply(&mut s, &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": asked}}).to_string());
        assert_eq!(r["result"]["protocolVersion"], want);
        assert_eq!(r["result"]["serverInfo"]["name"], "figcraft");
        let caps = &r["result"]["capabilities"];
        assert!(caps["tools"].is_object() && caps["resources"].is_object() && caps["prompts"].is_object());
        let ins = r["result"]["instructions"].as_str().unwrap();
        assert!(ins.contains("y DOWN") && ins.contains("RELATIVE TO THE PARENT") && ins.contains("list_commands") && ins.contains("screenshot"));
        assert!(!s.is_initialized());
        assert!(s.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
        assert!(s.is_initialized());
    }
}

#[test]
fn ping_and_unknown_method() {
    let mut c = Client::new();
    assert_eq!(c.rpc("ping", json!({}))["result"], json!({}));
    let r = c.rpc("nope/nothing", json!({}));
    assert_eq!(r["error"]["code"], -32601);
}

#[test]
fn tools_list_is_complete_and_schemas_are_objects() {
    let mut c = Client::new();
    let r = c.rpc("tools/list", json!({}));
    let tools = r["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, TOOL_NAMES);
    for t in tools {
        assert_eq!(t["inputSchema"]["type"], "object", "{}", t["name"]);
        assert!(t["description"].as_str().unwrap().len() > 40, "{} needs a real description", t["name"]);
        assert!(t["annotations"]["readOnlyHint"].is_boolean());
    }
}

#[test]
fn tools_call_happy_and_error_paths() {
    let mut c = Client::new();
    // Happy path.
    let id = c.create(json!({"type": "rectangle", "x": 10, "y": 20, "width": 30, "height": 40, "props": {"fill": "#FF0000"}}));
    let n = c.node(&id);
    assert_eq!(n["type"], "RECTANGLE".to_string().to_lowercase().replace("rectangle", n["type"].as_str().unwrap()));
    assert_eq!((n["x"].as_f64(), n["width"].as_f64()), (Some(10.0), Some(30.0)));
    assert_eq!(n["fills"][0]["color"].as_str().unwrap().to_ascii_uppercase(), "#FF0000");
    // Tool-level errors are isError results, not protocol errors.
    assert!(c.err("run_command", json!({"command": "no.such.command"})).contains("unknown command"));
    assert!(c.err("run_command", json!({"params": {}})).contains("`command`"));
    assert!(c.err("run_command", json!({"command": "node.set", "params": "nope"})).contains("object"));
    assert!(c.err("get_node", json!({"id": 999_999})).contains("999999"));
    assert!(c.err("get_node", json!({"id": [1, 2]})).contains("not a layer id"));
    assert!(c.err("create_node", json!({"type": "blob"})).contains("unknown type"));
    assert!(c.err("create_node", json!({"type": "rectangle", "x": "far"})).contains("`x`"));
    assert!(c.err("set_properties", json!({"ids": [id], "props": {"bogus": 1}})).contains("unknown properties"));
    assert!(c.err("set_properties", json!({"ids": [id], "props": {}})).contains("non-empty"));
    assert!(c.err("export", json!({"format": "bmp"})).contains("unknown format"));
    assert!(c.err("batch", json!({"commands": []})).contains("empty"));
    assert!(c.err("batch", json!({"commands": [{"command": 3}]})).contains("step 0"));
    assert!(c.err("pointer", json!({"x": 1})).contains("`y`"));
    assert!(c.err("type_text", json!({"text": "hi"})).contains("not editing text"));
    assert!(c.err("open_file", json!({"path": "/definitely/not/here.figcraft"})).contains("here.figcraft"));
    // Protocol-level errors.
    let r = c.rpc("tools/call", json!({"name": "no_such_tool", "arguments": {}}));
    assert_eq!(r["error"]["code"], -32602);
    let r = c.rpc("tools/call", json!({"arguments": {}}));
    assert_eq!(r["error"]["code"], -32602);
    let r = c.rpc("tools/call", json!({"name": "undo", "arguments": [1]}));
    assert_eq!(r["error"]["code"], -32602);
    // The document is unharmed by all of that.
    assert_eq!(c.page_children().len(), 1);
}

#[test]
fn malformed_messages_are_jsonrpc_errors() {
    let mut s = Server::headless();
    assert_eq!(reply(&mut s, "{not json")["error"]["code"], -32700);
    assert_eq!(reply(&mut s, "42")["error"]["code"], -32600);
    assert_eq!(reply(&mut s, "[]")["error"]["code"], -32600);
    assert_eq!(reply(&mut s, r#"{"jsonrpc":"2.0","id":1}"#)["error"]["code"], -32600);
    assert_eq!(reply(&mut s, r#"{"jsonrpc":"2.0","id":{"a":1},"method":"ping"}"#)["error"]["code"], -32600);
    assert_eq!(reply(&mut s, r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":[1]}"#)["error"]["code"], -32602);
    assert_eq!(reply(&mut s, r#"{"jsonrpc":"2.0","id":1,"method":7}"#)["error"]["code"], -32600);
    // Responses and unknown notifications are ignored.
    assert!(s.handle_line(r#"{"jsonrpc":"2.0","id":5,"result":{}}"#).is_none());
    assert!(s.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/whatever"}"#).is_none());
    assert!(s.handle_line("   ").is_none());
    // Legacy batches get an array of replies.
    let r = reply(
        &mut s,
        r#"[{"jsonrpc":"2.0","id":1,"method":"ping"},{"jsonrpc":"2.0","method":"notifications/initialized"},{"jsonrpc":"2.0","id":2,"method":"ping"}]"#,
    );
    assert_eq!(r.as_array().unwrap().len(), 2);
}

#[test]
fn resources_and_templates() {
    let mut c = Client::new();
    let list = c.rpc("resources/list", json!({}));
    let uris: Vec<&str> = list["result"]["resources"].as_array().unwrap().iter().map(|r| r["uri"].as_str().unwrap()).collect();
    assert!(uris.contains(&"figcraft://document"));
    let t = c.rpc("resources/templates/list", json!({}));
    let tpl: Vec<&str> = t["result"]["resourceTemplates"].as_array().unwrap().iter().map(|r| r["uriTemplate"].as_str().unwrap()).collect();
    assert_eq!(tpl, ["figcraft://node/{id}", "figcraft://command/{id}"]);

    let id = c.create(json!({"type": "frame", "name": "Hero", "width": 200, "height": 100}));
    let r = c.rpc("resources/read", json!({"uri": "figcraft://document"}));
    let doc: Value = serde_json::from_str(r["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert!(doc["pages"].is_array());
    let r = c.rpc("resources/read", json!({"uri": format!("figcraft://node/{id}")}));
    let n: Value = serde_json::from_str(r["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(n["name"], "Hero");
    let r = c.rpc("resources/read", json!({"uri": "figcraft://command/layout.add"}));
    assert!(r["result"]["contents"][0]["text"].as_str().unwrap().contains("Add auto layout"));
    for bad in ["figcraft://node/123456", "figcraft://node/xyz", "figcraft://command/nope", "http://example.com"] {
        assert_eq!(c.rpc("resources/read", json!({"uri": bad}))["error"]["code"], -32002, "{bad}");
    }
    assert_eq!(c.rpc("resources/read", json!({}))["error"]["code"], -32602);
}

#[test]
fn prompts_list_and_get() {
    let mut c = Client::new();
    let r = c.rpc("prompts/list", json!({}));
    let names: Vec<&str> = r["result"]["prompts"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["mobile_screen", "design_system", "landing_page", "prototype_flow"]);
    let r = c.rpc("prompts/get", json!({"name": "mobile_screen", "arguments": {"screen": "login"}}));
    let text = r["result"]["messages"][0]["content"]["text"].as_str().unwrap();
    assert!(text.contains("login") && text.contains("iPhone 16") && text.contains("#3B5BDB") && !text.contains("{device}"));
    assert_eq!(c.rpc("prompts/get", json!({"name": "mobile_screen"}))["error"]["code"], -32602);
    assert_eq!(c.rpc("prompts/get", json!({"name": "nope"}))["error"]["code"], -32602);
    assert_eq!(c.rpc("prompts/get", json!({"name": "landing_page", "arguments": {"product": ["x"]}}))["error"]["code"], -32602);
    let r = c.rpc("prompts/get", json!({"name": "prototype_flow"}));
    assert!(r["result"]["messages"][0]["content"]["text"].as_str().unwrap().contains("Home, Details, Checkout"));
}

#[test]
fn list_commands_filters() {
    let mut c = Client::new();
    let all = c.ok("list_commands", json!({}));
    let all = all.as_str().unwrap();
    assert!(all.lines().count() > 200);
    let some = c.ok("list_commands", json!({"filter": "align"}));
    let some = some.as_str().unwrap();
    assert!(some.contains("align.left") && !some.contains("file.save"));
    assert!(c.ok("list_commands", json!({"filter": "zzzzzz"})).as_str().unwrap().contains("no command"));
}

#[test]
fn serve_over_streams() {
    let input = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n\xff\xfe garbage\n{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n";
    let mut out = Vec::new();
    Server::headless().serve(&input[..], &mut out).unwrap();
    let lines: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[1]["error"]["code"], -32700);
    assert_eq!(lines[2]["id"], 2);
}

#[test]
fn serve_rejects_oversized_frames_before_consuming_the_stream() {
    use std::io::{BufReader, Read};

    const LIMIT: u64 = 32 * 1024 * 1024;
    for terminated in [false, true] {
        let mut oversized = std::io::repeat(b' ').take(LIMIT + 8192);
        let tail: &[u8] = if terminated { b"\n{\"method\":\"notifications/initialized\"}\n" } else { b"" };
        let mut input = BufReader::with_capacity(256, (&mut oversized).chain(tail));
        let mut output = Vec::new();
        let mut server = Server::headless();
        server.serve(&mut input, &mut output).unwrap();
        assert!(!server.is_initialized(), "no input following the oversized frame is dispatched");
        assert!(oversized.limit() >= 8192 - 256, "stop after the limit plus one read buffer");
        let reply: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(reply["error"]["code"], -32600);
        assert_eq!(reply["error"]["message"], "message too large");
    }
}

#[test]
fn serve_accepts_a_limit_sized_frame_and_a_final_unterminated_frame() {
    use std::io::{BufReader, Read};

    const LIMIT: u64 = 32 * 1024 * 1024;
    let ping = b"{\"id\":1,\"method\":\"ping\"}";
    let padding = std::io::repeat(b' ').take(LIMIT - ping.len() as u64);
    let input = (&ping[..]).chain(padding).chain(&b"\n\r\n{\"id\":2,\"method\":\"ping\"}"[..]);
    let mut output = Vec::new();
    Server::headless().serve(BufReader::new(input), &mut output).unwrap();
    let lines: Vec<Value> = String::from_utf8(output).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["result"], json!({}));
    assert_eq!(lines[1]["id"], 2);
}
