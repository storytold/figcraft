//! The tools that make building and checking a layout cheap for an agent: create_tree, outline,
//! lint_layout, the copy map of edit.duplicate, autosave, and the client name an attached app sees.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::{Arc, Mutex};

use common::Client;
use figcraft_mcp::{Backend, Headless, Server};
use serde_json::{Value, json};

fn card() -> Value {
    json!({"type": "frame", "name": "Card", "width": 320, "height": 40,
    "props": {"fill": "#FFFFFF", "layout": "vertical", "gap": 12, "padding": 16, "sizingV": "hug"},
    "children": [
        {"type": "text", "name": "Title", "text": "Ada Lovelace", "props": {"fontSize": 20}},
        {"type": "frame", "name": "Button", "height": 40, "props": {"fill": "#3B5BDB", "layout": "horizontal", "sizingH": "fill"},
            "children": [{"type": "text", "name": "Label", "text": "Follow", "props": {"fill": "#FFFFFF"}}]}
    ]})
}

#[test]
fn create_tree_outline_and_lint_round_trip() {
    let mut c = Client::new();
    let r = c.ok("create_tree", json!({"tree": card(), "x": 100, "y": 80}));
    let ids = r["ids"].as_object().unwrap();
    assert_eq!(ids.len(), 4, "{r}");
    // Colours given as "#hex" in nested props reach the engine normalised, as in create_node.
    let button = c.node(&ids["Card/Button"]);
    assert_eq!(button["fills"][0]["color"].as_str().unwrap().to_ascii_uppercase(), "#3B5BDB");
    assert_eq!(c.doc()["history"]["undo"][0], "Create layers", "one undo step");

    let outline = c.ok("outline", json!({"id": r["id"]}));
    let text = outline.as_str().unwrap();
    assert_eq!(text.lines().count(), 4, "{text}");
    assert!(text.lines().next().unwrap().contains("“Card”"), "{text}");

    let lint = c.ok("lint_layout", json!({"id": r["id"]}));
    assert_eq!(lint["issues"], json!([]), "a clean card: {lint}");
    // A badge past the right edge of a clipping button: the lint names it.
    c.ok("set_properties", json!({"ids": [ids["Card/Button"]], "props": {"clip": true, "layout": null}}));
    let badge = c.create(json!({"type": "rectangle", "name": "Badge", "parent": ids["Card/Button"], "x": 270, "y": 4, "width": 60, "height": 20}));
    let lint = c.ok("lint_layout", json!({}));
    assert!(lint["issues"].as_array().unwrap().iter().any(|i| i["rule"] == "clipped" && i["id"] == badge), "{lint}");

    assert!(c.err("create_tree", json!({"tree": {"type": "frame", "children": [{"type": "blob"}]}})).contains("tree.children[0]"));
    assert!(c.err("create_tree", json!({"tree": "card"})).contains("`tree`"));
    assert!(c.err("outline", json!({"id": [1]})).contains("not a layer id"));
}

#[test]
fn duplicate_maps_every_copy() {
    let mut c = Client::new();
    let r = c.ok("create_tree", json!({"tree": card()}));
    let card_id = r["id"].clone();
    let d = c.run("edit.duplicate", json!({"ids": [card_id]}));
    let label = r["ids"]["Card/Button/Label"].as_u64().unwrap().to_string();
    let copy = d["map"][&label].clone();
    assert!(copy.is_number() && copy != r["ids"]["Card/Button/Label"], "{d}");
    assert_eq!(c.node(&copy)["name"], "Label");
}

#[test]
fn autosave_writes_the_file_after_every_change() {
    let dir = std::env::temp_dir().join(format!("figcraft-autosave-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("agent.figcraft");
    let mut c = Client::with_server(Server::headless().with_autosave(true));
    // No file yet: the first change says how to get one, once.
    let first = c.call("create_node", json!({"type": "rectangle"}));
    let notes: Vec<&str> = first["content"].as_array().unwrap().iter().filter_map(|b| b["text"].as_str()).collect();
    assert!(notes.iter().any(|n| n.contains("autosave") && n.contains("save_file")), "{first}");
    let second = c.call("create_node", json!({"type": "rectangle"}));
    assert_eq!(second["content"].as_array().unwrap().len(), 1, "the note is sent once: {second}");

    c.ok("save_file", json!({"path": path}));
    let saved = std::fs::metadata(&path).unwrap().len();
    c.ok("create_tree", json!({"tree": card()}));
    assert!(std::fs::metadata(&path).unwrap().len() > saved, "the tree reached the file");
    assert_eq!(c.doc()["dirty"], false);
    // Reading never saves (and never fails because of autosave).
    c.ok("outline", json!({}));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A headless backend that records what the server sends it.
struct Recording {
    inner: Headless,
    hellos: Arc<Mutex<Vec<(String, String)>>>,
}

impl Backend for Recording {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.inner.call(method, params)
    }
    fn has_ui(&self) -> bool {
        false
    }
    fn describe(&self) -> String {
        "recording".into()
    }
    fn hello(&mut self, client: &str, version: &str) {
        self.hellos.lock().unwrap().push((client.into(), version.into()));
    }
}

#[test]
fn initialize_tells_the_backend_which_client_connected() {
    let hellos = Arc::new(Mutex::new(Vec::new()));
    let server = Server::new(Box::new(Recording { inner: Headless::new(), hellos: hellos.clone() }));
    let _ = Client::with_server(server);
    assert_eq!(*hellos.lock().unwrap(), vec![("test".to_string(), "0".to_string())]);
}
