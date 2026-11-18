//! Fuzz-ish: no input line may crash the server or leave it unable to answer.

use figcraft_mcp::{Server, TOOL_NAMES};
use proptest::prelude::*;
use serde_json::{Value, json};

fn still_alive(s: &mut Server) {
    let r: Value = serde_json::from_str(&s.handle_line(r#"{"jsonrpc":"2.0","id":"alive","method":"ping"}"#).unwrap()).unwrap();
    assert_eq!(r["result"], json!({}));
}

fn arb_json() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        any::<f64>().prop_filter("finite", |f| f.is_finite()).prop_map(Value::from),
        "[a-z#0-9 .:/{}-]{0,12}".prop_map(Value::from),
    ];
    leaf.prop_recursive(3, 24, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..5).prop_map(Value::Array),
            prop::collection::btree_map("[a-zA-Z]{1,8}", inner, 0..5).prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn arbitrary_text_lines(line in "\\PC{0,200}") {
        let mut s = Server::headless();
        let _ = s.handle_line(&line);
        still_alive(&mut s);
    }

    #[test]
    fn arbitrary_json_messages(v in arb_json(), method in prop::sample::select(vec!["initialize", "tools/call", "resources/read", "prompts/get", "tools/list", "ping", "x"])) {
        let mut s = Server::headless();
        let _ = s.handle_line(&v.to_string());
        let _ = s.handle_line(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": v}).to_string());
        still_alive(&mut s);
    }

    #[test]
    fn arbitrary_tool_arguments(tool in prop::sample::select(TOOL_NAMES.to_vec()), args in arb_json()) {
        let mut s = Server::headless();
        // Some content to act on.
        let _ = s.handle_line(&json!({"jsonrpc":"2.0","id":0,"method":"tools/call","params":{"name":"create_node","arguments":{"type":"frame","width":50,"height":50}}}).to_string());
        let line = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": args}}).to_string();
        let r: Value = serde_json::from_str(&s.handle_line(&line).unwrap()).unwrap();
        prop_assert!(r.get("result").is_some() || r.get("error").is_some());
        if let Some(res) = r.get("result") {
            prop_assert!(res["content"].is_array());
        }
        still_alive(&mut s);
    }

    #[test]
    fn arbitrary_commands(cmd in prop::sample::select(figcraft_engine::all_commands().iter().map(|c| c.id).filter(|id| !id.starts_with("file.") && *id != "doc.home").collect::<Vec<_>>()), params in arb_json()) {
        let mut s = Server::headless();
        let _ = s.handle_line(&json!({"jsonrpc":"2.0","id":0,"method":"tools/call","params":{"name":"create_node","arguments":{"type":"rectangle"}}}).to_string());
        let line = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "run_command", "arguments": {"command": cmd, "params": params}}}).to_string();
        let _ = s.handle_line(&line);
        still_alive(&mut s);
    }
}

#[test]
fn huge_and_deep_inputs() {
    let mut s = Server::headless();
    let deep = format!("{}{}", "[".repeat(10_000), "]".repeat(10_000));
    let _ = s.handle_line(&deep);
    let long = format!(r#"{{"jsonrpc":"2.0","id":1,"method":"{}"}}"#, "x".repeat(1_000_000));
    let _ = s.handle_line(&long);
    let _ = s.handle_line(
        &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"inspect_document","arguments":{"depth": 1e300}}}).to_string(),
    );
    let _ = s.handle_line(
        &json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"screenshot","arguments":{"scale": 1e300, "maxSize": -5}}}).to_string(),
    );
    let _ = s.handle_line(&json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"create_node","arguments":{"type":"rectangle","width": 1e12, "height": 1e12}}}).to_string());
    let _ = s.handle_line(&json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"export","arguments":{"scale": 64}}}).to_string());
    still_alive(&mut s);
}
