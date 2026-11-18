//! Compact trees are built by the backend, including every size-budget retry.

use crate::common::Client;
use figcraft_mcp::{Backend, Headless, call_tool};
use serde_json::{Value, json};

fn tree(mut create: impl FnMut(Value) -> Value) -> Value {
    let root = create(json!({"type": "frame", "name": "Product list", "width": 1200, "height": 2400}));
    for i in 0..24 {
        let card = create(json!({"type": "frame", "parent": root, "name": format!("Product card {i}"), "width": 280, "height": 160,
            "props": {"fill": "#F0F0F0", "cornerRadius": 12, "pluginData": {"source": "synthetic".repeat(40)}}}));
        for j in 0..6 {
            create(json!({"type": "text", "parent": card, "name": format!("Description {j}"), "text": "Synthetic product description. ".repeat(8),
                "x": 12, "y": j * 20, "width": 240}));
        }
    }
    root
}

fn compact_keys(node: &Value, outline: bool) {
    let allowed: &[&str] = if outline {
        &["id", "name", "type", "characters", "mainComponentName", "visible", "children", "childCount", "truncated"]
    } else {
        &["id", "name", "children", "childCount", "truncated"]
    };
    for key in node.as_object().unwrap().keys() {
        assert!(allowed.contains(&key.as_str()), "unexpected {key}: {node}");
    }
    for child in node["children"].as_array().into_iter().flatten() {
        compact_keys(child, outline);
    }
}

#[test]
fn tool_schemas_describe_compact_inspection_options() {
    let mut c = Client::new();
    let tools = c.rpc("tools/list", json!({}));
    for name in ["inspect_document", "get_node"] {
        let tool = tools["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == name).unwrap();
        let props = &tool["inputSchema"]["properties"];
        assert_eq!(props["fields"]["type"], "array");
        assert_eq!(props["fields"]["items"]["type"], "string");
        assert!(props["fields"]["items"]["enum"].as_array().unwrap().contains(&json!("absoluteBounds")));
        assert_eq!(props["outline"]["type"], "boolean");
        assert!(props["fields"]["description"].as_str().unwrap().contains("case-sensitive"));
        assert!(props["outline"]["description"].as_str().unwrap().contains("120"));
    }
}

#[test]
fn a_realistic_nested_tree_is_much_smaller_with_fields_or_outline() {
    let mut c = Client::new();
    let root = tree(|params| c.create(params));
    for (tool, mut args) in [("inspect_document", json!({"depth": 6})), ("get_node", json!({"id": root, "depth": 6}))] {
        let full = c.ok(tool, args.clone());
        let full_bytes = full.to_string().len();
        args["outline"] = json!(true);
        let outline = c.ok(tool, args.clone());
        assert!(outline.to_string().len() * 2 < full_bytes, "{tool}: outline {} vs full {full_bytes}", outline.to_string().len());
        args.as_object_mut().unwrap().remove("outline");
        args["fields"] = json!([]);
        let ids = c.ok(tool, args);
        assert!(ids.to_string().len() * 5 < full_bytes, "{tool}: ids {} vs full {full_bytes}", ids.to_string().len());
        if tool == "inspect_document" {
            assert_eq!(outline["history"], full["history"]);
            assert_eq!(ids["selection"], full["selection"]);
            compact_keys(&outline["pages"][0], true);
        } else {
            compact_keys(&outline, true);
        }
    }
}

struct Recording {
    inner: Headless,
    calls: Vec<(String, Value)>,
}

impl Backend for Recording {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.calls.push((method.into(), params.clone()));
        self.inner.call(method, params)
    }
    fn has_ui(&self) -> bool {
        false
    }
    fn describe(&self) -> String {
        "recording headless".into()
    }
}

#[test]
fn fields_and_outline_survive_every_depth_retry() {
    let mut backend = Recording { inner: Headless::new(), calls: Vec::new() };
    let root = tree(|params| {
        let result = call_tool(&mut backend, "create_node", &params);
        assert!(!result.is_error, "{}", result.first_text());
        serde_json::from_str::<Value>(result.first_text()).unwrap()["id"].clone()
    });
    for options in [json!({"fields": ["name"], "outline": false}), json!({"outline": true})] {
        for (tool, method) in [("inspect_document", "document.inspect"), ("get_node", "node.get")] {
            backend.calls.clear();
            let mut args = options.clone();
            if tool == "get_node" {
                args["id"] = root.clone();
            }
            args["depth"] = json!(6);
            args["maxBytes"] = json!(6000);
            let result = call_tool(&mut backend, tool, &args);
            assert!(!result.is_error, "{}", result.first_text());
            let value: Value = serde_json::from_str(result.first_text()).unwrap();
            assert!(value["truncated"].is_object(), "{tool}: {value}");
            assert!(backend.calls.len() > 1, "must exercise retries: {tool}");
            for (called, params) in &backend.calls {
                assert_eq!(called, method);
                for key in ["fields", "outline"] {
                    assert_eq!(params.get(key), options.get(key), "{tool}: {params}");
                }
            }
            let outline = options["outline"] == true;
            if tool == "inspect_document" {
                compact_keys(&value["pages"][0], outline);
            } else {
                compact_keys(&value, outline);
            }
        }
    }
}

#[test]
fn compact_children_keep_window_markers_and_page_in_order() {
    let mut c = Client::new();
    let root = tree(|params| c.create(params));
    let full = c.ok("get_node", json!({"id": root, "depth": 1, "fields": []}));
    let expected = full["children"].as_array().unwrap();
    let wire = serde_json::to_string(&serde_json::to_string_pretty(&full).unwrap()).unwrap().len();
    let limit = wire + 1024 - 1;
    assert!(limit > 2048, "room for both the envelope and the marker");
    let mut seen = Vec::new();
    let mut offset = Some(0);
    let mut pages = 0;
    while let Some(at) = offset {
        let page = c.ok("get_node", json!({"id": root, "depth": 1, "fields": [], "offset": at, "maxBytes": limit}));
        assert!(page["truncated"]["children"].is_object(), "{page}");
        let wire = serde_json::to_string(&serde_json::to_string_pretty(&page).unwrap()).unwrap().len();
        assert!(wire + 1024 <= limit, "page at {at} exceeds maxBytes: {wire} + envelope > {limit}");
        assert_eq!(page["truncated"]["children"]["from"], at);
        seen.extend(page["children"].as_array().unwrap().iter().cloned());
        offset = page["truncated"]["children"]["nextOffset"].as_u64();
        pages += 1;
        assert!(pages <= expected.len());
    }
    assert!(pages > 1, "the fixture must require a child window");
    assert_eq!(&seen, expected);
}

/// `text` with Unix line ends: a Windows checkout with `core.autocrlf` has the docs with CRLF.
fn lf(text: &str) -> String {
    text.replace("\r\n", "\n")
}

fn json_example(section: &str) -> &str {
    section.split("```json\n").nth(1).unwrap().split("```").next().unwrap()
}

#[test]
fn compact_examples_run_verbatim_from_the_docs() {
    let mut c = Client::new();
    for _ in 0..2 {
        c.create(json!({"type": "rectangle"}));
    }
    let docs = lf(include_str!("../../../../docs/mcp.md"));
    let section = docs.split("### Compact layer trees\n").nth(1).unwrap();
    for line in json_example(section).lines() {
        let reply: Value = serde_json::from_str(&c.server.handle_line(line).unwrap()).unwrap();
        assert_eq!(reply["result"]["isError"], false, "{reply}");
    }
    let mut h = Headless::new();
    for _ in 0..2 {
        h.call("engine.execute", json!({"command": "node.create", "params": {"type": "rectangle"}})).unwrap();
    }
    let docs = lf(include_str!("../../../../docs/control-protocol.md"));
    let section = docs.split("### `node.get ").nth(1).unwrap();
    for line in json_example(section).lines() {
        let request: Value = serde_json::from_str(line).unwrap();
        h.call(request["method"].as_str().unwrap(), request["params"].clone()).unwrap();
    }
}

#[test]
fn invalid_compact_options_fail_in_tools_and_headless_control() {
    let mut c = Client::new();
    let id = c.create(json!({"type": "rectangle"}));
    let mut h = Headless::new();
    let created = h.call("engine.execute", json!({"command": "node.create", "params": {"type": "rectangle"}})).unwrap();
    assert_eq!(created["id"], id);
    for mut args in [
        json!({"fields": [1]}),
        json!({"fields": null}),
        json!({"fields": ["Name"]}),
        json!({"outline": "yes"}),
        json!({"outline": true, "fields": []}),
    ] {
        args["id"] = id.clone();
        for tool in ["inspect_document", "get_node"] {
            assert!(!c.err(tool, args.clone()).is_empty());
        }
        for method in ["document.inspect", "node.get"] {
            assert!(h.call(method, args.clone()).is_err(), "{method}: {args}");
        }
    }
}
