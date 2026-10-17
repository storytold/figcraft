//! `--automation-read-root` / `--automation-write-root` over MCP (#715): a tool call reads only
//! inside the read and write roots and writes only inside the write roots.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::disallowed_methods)]

use std::path::PathBuf;

use figcraft_engine::file_access::Roots;
use figcraft_mcp::Server;
use serde_json::{Value, json};

/// `<tmp>/figcraft-mcp-roots-<name>-<pid>` with `in/` (read root), `out/` (write root),
/// `outside/` and `out2/` (named like the write root, outside it).
struct Dirs(PathBuf);

impl Dirs {
    fn new(name: &str) -> Self {
        let d = std::env::temp_dir().join(format!("figcraft-mcp-roots-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for sub in ["in", "out", "outside", "out2"] {
            std::fs::create_dir_all(d.join(sub)).unwrap();
        }
        Self(d)
    }

    fn p(&self, rel: &str) -> String {
        self.0.join(rel).to_string_lossy().into_owned()
    }

    fn server(&self) -> Server {
        Server::headless().with_roots(Some(Roots::new(&[self.p("in")], &[self.p("out")]).unwrap()))
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn call(s: &mut Server, tool: &str, args: Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": args}}).to_string();
    let r: Value = serde_json::from_str(&s.handle_line(&line).unwrap()).unwrap();
    r["result"].clone()
}

fn text(r: &Value) -> String {
    r["content"][0]["text"].as_str().unwrap_or_default().to_string()
}

#[test]
fn writes_go_only_into_the_write_root() {
    let d = Dirs::new("write");
    let mut s = d.server();
    assert_eq!(call(&mut s, "create_node", json!({"type": "frame", "width": 40, "height": 20}))["isError"], false);
    assert_eq!(call(&mut s, "run_command", json!({"command": "export.add"}))["isError"], false);
    for (tool, args) in [
        ("save_file", json!({"path": d.p("outside/a.figcraft")})),
        ("save_file", json!({"path": d.p("out2/a.figcraft")})),
        ("save_file", json!({"path": d.p("out/../outside/a.figcraft")})),
        ("save_file", json!({"path": d.p("in/a.figcraft")})),
        ("export", json!({"format": "png", "path": d.p("outside/a.png")})),
        ("run_command", json!({"command": "file.export", "params": {"format": "svg", "path": d.p("outside/a.svg")}})),
        ("run_command", json!({"command": "file.exportLayers", "params": {"dir": d.p("outside")}})),
        ("batch", json!({"commands": [{"command": "file.saveAs", "params": {"path": d.p("outside/b.figcraft")}}]})),
    ] {
        let r = call(&mut s, tool, args.clone());
        assert_eq!(r["isError"], true, "{tool} {args}: {r}");
        assert!(text(&r).contains("automation path rejected"), "{tool}: {r}");
    }
    for sub in ["outside", "out2", "in"] {
        assert_eq!(std::fs::read_dir(d.0.join(sub)).unwrap().count(), 0, "nothing written in {sub}");
    }
    assert_eq!(call(&mut s, "save_file", json!({"path": d.p("out/a.figcraft")}))["isError"], false);
    assert_eq!(call(&mut s, "export", json!({"format": "png", "path": d.p("out/a.png")}))["isError"], false);
    assert!(d.0.join("out/a.figcraft").is_file() && d.0.join("out/a.png").is_file());
    // Without a path the bytes come back, written nowhere.
    let r = call(&mut s, "export", json!({"format": "png"}));
    assert_eq!(r["content"][0]["type"], "image", "{r}");
}

#[test]
fn reads_go_only_into_the_read_and_write_roots() {
    let d = Dirs::new("read");
    // Files made without roots: one in each folder.
    let mut free = Server::headless();
    assert_eq!(call(&mut free, "create_node", json!({"type": "frame", "width": 40, "height": 20, "name": "Made"}))["isError"], false);
    for f in ["in/a.figcraft", "out/b.figcraft", "outside/c.figcraft"] {
        assert_eq!(call(&mut free, "export", json!({"format": "svg", "path": d.p(&f.replace(".figcraft", ".svg"))}))["isError"], false);
        assert_eq!(call(&mut free, "run_command", json!({"command": "file.saveAs", "params": {"path": d.p(f)}}))["isError"], false);
    }
    let mut s = d.server();
    for (tool, args) in [
        ("open_file", json!({"path": d.p("outside/c.figcraft")})),
        ("open_file", json!({"path": d.p("in/../outside/c.figcraft")})),
        ("open_file", json!({"path": d.p("in/a.figcraft"), "variables": d.p("outside/c.svg")})),
        ("run_command", json!({"command": "file.import", "params": {"path": d.p("outside/c.svg")}})),
        ("run_command", json!({"command": "file.placeImage", "params": {"path": d.p("outside/c.svg")}})),
        ("run_command", json!({"command": "library.add", "params": {"path": d.p("outside/c.figcraft")}})),
    ] {
        let r = call(&mut s, tool, args.clone());
        assert_eq!(r["isError"], true, "{tool} {args}: {r}");
        assert!(text(&r).contains("automation path rejected"), "{tool}: {r}");
    }
    assert_eq!(call(&mut s, "open_file", json!({"path": d.p("in/a.figcraft")}))["isError"], false);
    assert_eq!(call(&mut s, "open_file", json!({"path": d.p("out/b.figcraft")}))["isError"], false, "a write root may be read");
    assert_eq!(call(&mut s, "run_command", json!({"command": "file.import", "params": {"path": d.p("in/a.svg")}}))["isError"], false);
}

#[test]
fn initialize_names_the_roots_and_no_roots_changes_nothing() {
    let d = Dirs::new("init");
    let mut s = d.server();
    let r: Value =
        serde_json::from_str(&s.handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#).unwrap())
            .unwrap();
    let ins = r["result"]["instructions"].as_str().unwrap();
    assert!(ins.contains("only these folders may be read") && ins.contains("figcraft-mcp-roots-init"), "{ins}");
    let mut free = Server::headless();
    let r: Value = serde_json::from_str(
        &free.handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#).unwrap(),
    )
    .unwrap();
    assert!(!r["result"]["instructions"].as_str().unwrap().contains("only these folders"));
    assert_eq!(call(&mut free, "create_node", json!({"type": "frame", "width": 4, "height": 4}))["isError"], false);
    assert_eq!(call(&mut free, "export", json!({"format": "png", "path": d.p("outside/free.png")}))["isError"], false);
}
