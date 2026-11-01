//! Plugins: runs are one undo step and roll back when they fail, permissions and the host's own
//! limits are enforced, fuel and memory are capped, and junk never gets in. Test plugins are
//! assembled from WAT text: each makes a fixed list of command calls.

use base64::Engine as _;
use serde_json::{Value, json};

use crate::components::tests::{ex, idv, node};
use crate::{EngineError, Session};
use figcraft_doc::NodeId;

/// What a test plugin does when a call is refused: fail the run (status 1), or carry on and report
/// the refusal as its output.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum OnRefusal {
    Fail,
    Report,
}

/// `s` as a WAT string literal.
pub(crate) fn lit(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_graphic() && b != b'"' && b != b'\\' || b == b' ' { (b as char).to_string() } else { format!("\\{b:02x}") })
        .collect()
}

/// A module that reads the selection, makes `calls` in order and outputs the selection it read
/// (or the first refusal). `tail` runs after the calls (an endless loop, a trap…).
pub(crate) fn script(calls: &[(&str, Value)], on_refusal: OnRefusal, tail: &str) -> Vec<u8> {
    let mut data = String::new();
    let mut body = String::from("(local.set $sel (call $reply_read (i32.const 60000) (call $selection)))\n");
    let mut at = 0usize;
    for (cmd, params) in calls {
        let p = params.to_string();
        data += &format!("(data (i32.const {at}) \"{}\") (data (i32.const {}) \"{}\")\n", lit(cmd), at + cmd.len(), lit(&p));
        let status = if on_refusal == OnRefusal::Fail { 1 } else { 0 };
        body += &format!(
            "(if (call $call (i32.const {at}) (i32.const {}) (i32.const {}) (i32.const {}))
               (then (call $set_output (i32.const 40000) (call $reply_read (i32.const 40000) (i32.const 20000))) (return (i32.const {status}))))\n",
            cmd.len(),
            at + cmd.len(),
            p.len()
        );
        at += cmd.len() + p.len();
    }
    assert!(at < 40000);
    let wat = format!(
        r#"(module
          (import "figcraft" "call" (func $call (param i32 i32 i32 i32) (result i32)))
          (import "figcraft" "selection" (func $selection (result i32)))
          (import "figcraft" "reply_read" (func $reply_read (param i32 i32) (result i32)))
          (import "figcraft" "set_output" (func $set_output (param i32 i32)))
          (import "figcraft" "notify" (func $notify (param i32 i32)))
          (import "figcraft" "log" (func $log (param i32 i32)))
          (memory (export "memory") 2)
          {data}
          (func (export "figcraft_run") (param $n i32) (result i32) (local $sel i32)
            {body}
            {tail}
            (call $set_output (i32.const 60000) (local.get $sel))
            (i32.const 0)))"#
    );
    wat::parse_str(&wat).unwrap_or_else(|e| panic!("{e}\n{wat}"))
}

pub(crate) fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub(crate) fn manifest(id: &str, access: &str, allow: &[&str]) -> Value {
    json!({"id": id, "name": format!("Plugin {id}"), "version": "1.0.0", "main": "plugin.wasm",
           "permissions": {"access": access, "commands": allow}})
}

/// Install a plugin from a manifest and module bytes (no files).
pub(crate) fn install(s: &mut Session, manifest: Value, wasm: &[u8]) {
    ex(s, "plugin.import", json!({"manifest": manifest, "wasm": b64(wasm)}));
}

pub(crate) fn rect(s: &mut Session, x: f64) -> NodeId {
    idv(&ex(s, "shape.rectangle", json!({"x": x, "y": 0, "width": 10, "height": 10})))
}

pub(crate) fn layer_count(s: &Session) -> usize {
    let ds = s.ds().unwrap();
    ds.doc.node(ds.page).unwrap().children.len()
}

pub(crate) fn undo_len(s: &Session) -> usize {
    s.ds().unwrap().history.undo.len()
}

fn run_err(s: &mut Session, id: &str) -> String {
    match s.execute("plugin.run", &json!({"id": id, "fuel": 2_000_000})) {
        Ok(v) => panic!("ran: {v}"),
        Err(e) => {
            assert!(!matches!(e, EngineError::Internal { .. }), "{e}");
            e.to_string()
        }
    }
}

#[test]
fn a_run_reads_the_selection_and_its_edits_are_one_undo_step() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0);
    ex(&mut s, "select.set", json!({"ids": [a]}));
    let calls = [
        ("object.rename", json!({"name": "Tidy"})),
        ("shape.rectangle", json!({"x": 50, "y": 0, "width": 20, "height": 20})),
        ("node.get", json!({"id": a})),
    ];
    install(&mut s, manifest("org.test.tidy", "write", &[]), &script(&calls, OnRefusal::Fail, ""));
    let (layers, undo) = (layer_count(&s), undo_len(&s));
    let out = ex(&mut s, "plugin.run", json!({"id": "org.test.tidy"}));
    assert_eq!(out["output"], json!([a.0]), "the plugin read the selection");
    assert_eq!(out["calls"], json!(6));
    assert_eq!(node(&s, a).name, "Tidy");
    assert_eq!(layer_count(&s), layers + 1);
    assert_eq!(undo_len(&s), undo + 1, "one undo step for the whole run");
    assert_eq!(s.ds().unwrap().history.undo.last().unwrap().label, "Plugin org.test.tidy");
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(node(&s, a).name, "Rectangle");
    assert_eq!(layer_count(&s), layers);
    // Run last plugin runs it again.
    ex(&mut s, "plugins.runLast", json!({}));
    assert_eq!(layer_count(&s), layers + 1);
    assert_eq!(ex(&mut s, "plugin.list", json!({}))["last"], json!({"id": "org.test.tidy", "command": "run"}));
}

#[test]
fn read_only_plugins_and_allowlists_are_enforced() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0);
    // Read access: queries run, edits are refused and the run fails, leaving nothing.
    install(
        &mut s,
        manifest("org.test.reader", "read", &[]),
        &script(&[("node.get", json!({"id": a})), ("shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5}))], OnRefusal::Fail, ""),
    );
    let (layers, undo) = (layer_count(&s), undo_len(&s));
    let e = run_err(&mut s, "org.test.reader");
    assert!(e.contains("may only read"), "{e}");
    assert_eq!((layer_count(&s), undo_len(&s)), (layers, undo));
    // Write access limited to some commands.
    install(
        &mut s,
        manifest("org.test.narrow", "write", &["node.*", "select.set"]),
        &script(&[("node.get", json!({"id": a})), ("object.rename", json!({"name": "x"}))], OnRefusal::Fail, ""),
    );
    let e = run_err(&mut s, "org.test.narrow");
    assert!(e.contains("asked permission for"), "{e}");
    assert_eq!(node(&s, a).name, "Rectangle");
}

#[test]
fn files_dialogs_history_and_plugins_are_off_limits() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0);
    ex(&mut s, "select.set", json!({"ids": [a]}));
    let dir = std::env::temp_dir().join(format!("figcraft-plugin-deny-{}", std::process::id()));
    let target = dir.join("stolen.figcraft").to_string_lossy().into_owned();
    let refused = [
        ("file.saveAs", json!({"path": target})),
        ("file.open", json!({"path": "C:/Windows/win.ini"})),
        ("image.place", json!({"path": "secret.png"})),
        ("text.loadFont", json!({"path": "/etc/hosts"})),
        ("library.add", json!({"path": "lib.figcraft"})),
        ("plugin.run", json!({"id": "org.test.deny"})),
        ("plugin.remove", json!({"id": "org.test.deny"})),
        ("edit.undo", json!({})),
        ("command.batch", json!({"commands": [{"command": "file.saveAs", "params": {"path": target}}]})),
        ("data.fill", json!({"id": a, "rows": [{}], "dir": dir.to_string_lossy()})),
        ("variable.export", json!({"format": "css", "path": target})),
        ("codeConnect.export", json!({"path": target})),
        ("codeConnect.import", json!({"path": "/etc/passwd"})),
        ("component.playground", json!({"id": a, "path": target})),
        ("variable.import", json!({"variables": "/etc/passwd"})),
        ("collab.recoverFile", json!({"path": "copy.figcraft", "file": "x", "session": 1, "client": 1, "seq": 0})),
        ("window.new", json!({"path": "C:/Windows/win.ini"})),
        ("doc.switch", json!({"index": 0})),
        ("pref.keepTool", json!({})),
        // Without a name it would open inline renaming in the layers panel: a UI request.
        ("object.rename", json!({})),
    ];
    for (cmd, params) in refused {
        install(&mut s, manifest("org.test.deny", "write", &[]), &script(&[(cmd, params.clone())], OnRefusal::Report, ""));
        let (doc, undo) = (s.ds().unwrap().doc.clone(), undo_len(&s));
        let out = ex(&mut s, "plugin.run", json!({"id": "org.test.deny"}));
        let msg = out["output"].as_str().unwrap_or_default().to_string();
        assert!(msg.contains("plugins") || msg.contains("dialog"), "{cmd}: {out}");
        assert!(std::sync::Arc::ptr_eq(&doc, &s.ds().unwrap().doc), "{cmd} left the document as it was");
        assert_eq!(undo_len(&s), undo, "{cmd}");
        assert!(s.ui_requests.is_empty(), "{cmd}: {:?}", s.ui_requests);
    }
    assert!(!dir.exists(), "nothing was written");
    assert!(ex(&mut s, "plugin.list", json!({}))["plugins"].as_array().unwrap().len() == 1, "the plugin didn't remove itself");
    // The read-only exceptions are open.
    install(&mut s, manifest("org.test.reads", "read", &[]), &script(&[("doc.list", json!({})), ("version.list", json!({}))], OnRefusal::Fail, ""));
    ex(&mut s, "plugin.run", json!({"id": "org.test.reads"}));
}

#[test]
fn traps_endless_loops_and_memory_hogs_roll_back() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0);
    ex(&mut s, "select.set", json!({"ids": [a]}));
    let edit = [("shape.rectangle", json!({"x": 90, "y": 0, "width": 5, "height": 5})), ("object.rename", json!({"name": "Half done"}))];
    let bulk_output = "(call $set_output (i32.const 0) (i32.const 65536))".repeat(32);
    let cases = [
        ("org.test.trap", "unreachable", "crashed"),
        ("org.test.loop", "(loop $l (br $l))", "instruction budget"),
        ("org.test.bulk", bulk_output.as_str(), "instruction budget"),
        ("org.test.grow", "(if (i32.eq (memory.grow (i32.const 2000)) (i32.const -1)) (then unreachable))", "crashed"),
    ];
    for (id, tail, want) in cases {
        install(&mut s, manifest(id, "write", &[]), &script(&edit, OnRefusal::Fail, tail));
        let (layers, undo, sel) = (layer_count(&s), undo_len(&s), s.selection());
        let started = web_time::Instant::now();
        let e = run_err(&mut s, id);
        assert!(e.contains(want), "{id}: {e}");
        assert!(started.elapsed().as_secs() < 20, "{id} was stopped promptly");
        assert_eq!((layer_count(&s), undo_len(&s), s.selection()), (layers, undo, sel), "{id} rolled back");
        assert_eq!(node(&s, a).name, "Rectangle");
    }
    // A module that asks for more memory than the cap up front never starts.
    let hog =
        wat::parse_str(r#"(module (memory (export "memory") 3000) (func (export "figcraft_run") (param i32) (result i32) (i32.const 0)))"#).unwrap();
    install(&mut s, manifest("org.test.hog", "write", &[]), &hog);
    assert!(run_err(&mut s, "org.test.hog").contains("memory limit"));
}

#[test]
fn exceeding_a_log_limit_rolls_back_the_plugins_edits() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0);
    ex(&mut s, "select.set", json!({"ids": [a]}));
    let edits = [("object.rename", json!({"name": "Half done"})), ("shape.rectangle", json!({"x": 90, "y": 0, "width": 5, "height": 5}))];
    let wasm = script(&edits, OnRefusal::Fail, "(call $log (i32.const 80000) (i32.const 8193))");
    let id = "org.test.log-limit";
    install(&mut s, manifest(id, "write", &[]), &wasm);
    let ds = s.ds().unwrap();
    let (doc, page, selection, undo) = (ds.doc.clone(), ds.page, s.selection(), undo_len(&s));
    let error = run_err(&mut s, id);
    assert!(error.contains("log line") && error.contains("8192"), "{error}");
    assert_eq!(s.doc().unwrap(), doc.as_ref());
    assert_eq!(s.ds().unwrap().page, page);
    assert_eq!(s.selection(), selection);
    assert_eq!(undo_len(&s), undo);
    assert_eq!(node(&s, a).name, "Rectangle");
}

#[test]
fn parameters_defaults_and_notifications_reach_the_plugin() {
    let mut s = Session::new();
    let echo = wat::parse_str(
        r#"(module
          (import "figcraft" "reply_read" (func $reply_read (param i32 i32) (result i32)))
          (import "figcraft" "set_output" (func $set_output (param i32 i32)))
          (import "figcraft" "notify" (func $notify (param i32 i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "Done!")
          (func (export "figcraft_run") (param $n i32) (result i32)
            (call $set_output (i32.const 100) (call $reply_read (i32.const 100) (local.get $n)))
            (call $notify (i32.const 0) (i32.const 5))
            (i32.const 0)))"#,
    )
    .unwrap();
    let mut m = manifest("org.test.echo", "read", &[]);
    m["commands"] = json!([
        {"id": "first", "label": "First"},
        {"id": "grid", "label": "Swatch grid", "params": [{"name": "columns", "type": "number", "default": 4, "min": 1, "max": 8}, {"name": "label", "type": "text"}]}
    ]);
    install(&mut s, m, &echo);
    let out = ex(&mut s, "plugin.run", json!({"id": "org.test.echo", "command": "grid", "params": {"columns": 99}}));
    assert_eq!(out["output"], json!({"api": 1, "command": "grid", "params": {"columns": 8.0, "label": ""}, "selection": []}));
    assert_eq!(s.message.as_deref(), Some("Done!"));
    assert_eq!(undo_len(&s), 0, "a run that edits nothing adds no undo step");
    assert_eq!(ex(&mut s, "plugin.run", json!({"id": "org.test.echo"}))["output"]["command"], "first", "the first command by default");
    for bad in [
        json!({"id": "org.test.echo", "command": "nope"}),
        json!({"id": "org.test.echo", "command": "grid", "params": {"columns": "many"}}),
        json!({"id": "missing"}),
        json!({}),
    ] {
        assert!(matches!(s.execute("plugin.run", &bad), Err(EngineError::BadParams { .. })), "{bad}");
    }
    assert!(s.execute("plugins.runLast", &json!({})).is_ok());
    ex(&mut s, "plugin.remove", json!({"id": "org.test.echo"}));
    assert!(s.execute("plugins.runLast", &json!({})).is_err(), "the removed plugin is forgotten");
}

#[test]
fn junk_manifests_and_modules_are_refused() {
    let mut s = Session::new();
    let ok = script(&[], OnRefusal::Fail, "");
    let junk = [
        json!({}),
        json!({"manifest": {}}),
        json!({"manifest": manifest("org.test.x", "write", &[]), "wasm": "!!!not base64"}),
        json!({"manifest": manifest("org.test.x", "write", &[]), "wasm": b64(b"\0asm\x01\0\0\0garbage")}),
        json!({"manifest": manifest("org.test.x", "write", &[]), "wasm": b64(b"")}),
        json!({"manifest": manifest("org.test.x", "everything", &[]), "wasm": b64(&ok)}),
        json!({"manifest": manifest("../escape", "write", &[]), "wasm": b64(&ok)}),
        json!({"manifest": "{ not json", "wasm": b64(&ok)}),
        json!({"manifest": 7, "wasm": b64(&ok)}),
        json!({"manifest": manifest("org.test.x", "write", &[])}),
        json!({"path": "Z:/definitely/not/here/figcraft-plugin.json"}),
        json!({"path": 5}),
        json!({"manifest": manifest("org.test.x", "write", &[]), "wasm": "A".repeat(23_000_000)}),
    ];
    for j in &junk {
        match s.execute("plugin.import", j) {
            Ok(v) => panic!("imported {v}"),
            Err(EngineError::Internal { msg, .. }) => panic!("panicked: {msg}"),
            Err(_) => {}
        }
    }
    assert_eq!(ex(&mut s, "plugin.list", json!({}))["plugins"], json!([]));
    // Every plugin command with junk parameters (the engine-wide sweep runs them too).
    install(&mut s, manifest("org.test.ok", "write", &[]), &ok);
    for cmd in ["plugin.list", "plugin.import", "plugin.remove", "plugin.run", "plugins.runLast", "plugins.manage"] {
        for j in [
            json!(null),
            json!([1]),
            json!({"id": 5, "command": [], "params": "x", "fuel": "NaN"}),
            json!({"id": "org.test.ok", "params": [1], "fuel": -1}),
        ] {
            if let Err(EngineError::Internal { msg, .. }) = s.execute(cmd, &j) {
                panic!("{cmd} panicked on {j}: {msg}");
            }
        }
    }
}

/// A temporary folder removed when dropped (also when a test fails).
struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn development_plugins_load_from_a_manifest_and_reload_their_module() {
    let dir = TempDir(std::env::temp_dir().join(format!("figcraft-plugin-dev-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let write_module = |calls: &[(&str, Value)]| std::fs::write(dir.0.join("plugin.wasm"), script(calls, OnRefusal::Fail, "")).unwrap();
    std::fs::write(dir.0.join("figcraft-plugin.json"), manifest("org.test.dev", "write", &[]).to_string()).unwrap();
    write_module(&[("shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5}))]);
    let mut s = Session::new();
    // The folder or the manifest file itself.
    let out = ex(&mut s, "plugin.import", json!({"path": dir.0.to_string_lossy()}));
    assert_eq!(out["id"], "org.test.dev");
    assert_eq!(out["replaced"], false);
    let manifest_path = dir.0.join("figcraft-plugin.json").to_string_lossy().into_owned();
    assert_eq!(ex(&mut s, "plugin.import", json!({"path": manifest_path}))["replaced"], true);
    ex(&mut s, "plugin.run", json!({"id": "org.test.dev"}));
    assert_eq!(layer_count(&s), 1);
    // A rebuilt module runs without importing again.
    write_module(&[
        ("shape.ellipse", json!({"x": 0, "y": 0, "width": 5, "height": 5})),
        ("shape.ellipse", json!({"x": 9, "y": 0, "width": 5, "height": 5})),
    ]);
    ex(&mut s, "plugin.run", json!({"id": "org.test.dev"}));
    assert_eq!(layer_count(&s), 3);
    // Only plugins with a manifest file are saved with the preferences.
    install(&mut s, manifest("org.test.memory", "read", &[]), &script(&[], OnRefusal::Fail, ""));
    let saved = serde_json::to_value(&s.prefs).unwrap();
    let ids: Vec<&str> = saved["plugins"].as_array().unwrap().iter().filter_map(|p| p["manifest"]["id"].as_str()).collect();
    assert_eq!(ids, vec!["org.test.dev"]);
    let back: crate::Prefs = serde_json::from_value(saved).unwrap();
    assert_eq!(back.plugins.len(), 1);
    // A missing module fails the run cleanly.
    std::fs::remove_file(dir.0.join("plugin.wasm")).unwrap();
    assert!(run_err(&mut s, "org.test.dev").contains("plugin.wasm"));
    ex(&mut s, "plugin.remove", json!({"id": "org.test.dev"}));
    assert_eq!(ex(&mut s, "plugin.list", json!({}))["plugins"].as_array().unwrap().len(), 1);
}

#[test]
fn a_full_undo_history_keeps_its_steps_and_tampered_manifests_are_refused() {
    let mut s = Session::new();
    s.ds_mut().unwrap().history.limit = 3;
    let first = rect(&mut s, 0.0);
    for x in [20.0, 40.0] {
        rect(&mut s, x);
    }
    assert_eq!(undo_len(&s), 3);
    let edits: Vec<(&str, Value)> = (0..5).map(|i| ("shape.ellipse", json!({"x": i * 10, "y": 50, "width": 5, "height": 5}))).collect();
    install(&mut s, manifest("org.test.many", "write", &[]), &script(&edits, OnRefusal::Fail, ""));
    let oldest = s.ds().unwrap().history.undo[1].doc.clone();
    ex(&mut s, "plugin.run", json!({"id": "org.test.many"}));
    let undo = &s.ds().unwrap().history.undo;
    assert_eq!(undo.len(), 3, "the run is one step; only the oldest step made room for it");
    assert!(std::sync::Arc::ptr_eq(&undo[0].doc, &oldest));
    assert!(s.ds().unwrap().doc.contains(first));
    // A failing run leaves the history alone.
    install(&mut s, manifest("org.test.fail", "write", &[]), &script(&edits, OnRefusal::Fail, "unreachable"));
    let before: Vec<_> = s.ds().unwrap().history.undo.iter().map(|e| e.doc.clone()).collect();
    run_err(&mut s, "org.test.fail");
    let after = &s.ds().unwrap().history.undo;
    assert!(before.len() == after.len() && before.iter().zip(after).all(|(a, b)| std::sync::Arc::ptr_eq(a, &b.doc)));
    // A manifest edited in the preferences file is checked again before it runs.
    s.prefs.plugins[0].manifest.main = "../../elsewhere.wasm".into();
    assert!(matches!(s.execute("plugin.run", &json!({"id": "org.test.many"})), Err(EngineError::BadParams { .. })));
}

/// A module that offers `files` (name, mime, data) with `save_file` in order, outputs the last
/// refusal and returns `exit`.
fn saver(files: &[(&str, &str, &[u8])], exit: i32) -> Vec<u8> {
    let hex = |b: &[u8]| b.iter().map(|b| format!("\\{b:02x}")).collect::<String>();
    let mut data = String::new();
    let mut body = String::new();
    let mut at = 0usize;
    for (name, mime, bytes) in files {
        let (n, m) = (at, at + name.len());
        let d = m + mime.len();
        data += &format!(
            "(data (i32.const {n}) \"{}\") (data (i32.const {m}) \"{}\") (data (i32.const {d}) \"{}\")\n",
            hex(name.as_bytes()),
            hex(mime.as_bytes()),
            hex(bytes)
        );
        body += &format!(
            "(local.set $st (call $save_file (i32.const {n}) (i32.const {}) (i32.const {m}) (i32.const {}) (i32.const {d}) (i32.const {})))
             (if (local.get $st) (then (call $set_output (i32.const 30000) (call $reply_read (i32.const 30000) (i32.const 500)))))\n",
            name.len(),
            mime.len(),
            bytes.len()
        );
        at = d + bytes.len();
    }
    let wat = format!(
        r#"(module
          (import "figcraft" "save_file" (func $save_file (param i32 i32 i32 i32 i32 i32) (result i32)))
          (import "figcraft" "reply_read" (func $reply_read (param i32 i32) (result i32)))
          (import "figcraft" "set_output" (func $set_output (param i32 i32)))
          (memory (export "memory") 1)
          {data}
          (func (export "figcraft_run") (param $n i32) (result i32) (local $st i32)
            {body}
            (i32.const {exit})))"#
    );
    wat::parse_str(&wat).unwrap_or_else(|e| panic!("{e}\n{wat}"))
}

fn save_manifest(id: &str, save_file: bool) -> Value {
    json!({"id": id, "name": "Saver", "version": "1.0.0", "main": "plugin.wasm", "api": 4, "permissions": {"access": "read", "saveFile": save_file}})
}

fn saves(s: &Session) -> Vec<&crate::UiRequest> {
    s.ui_requests.iter().filter(|r| r.kind == "dialog.pluginSave").collect()
}

#[test]
fn a_plugin_without_the_permission_cannot_save() {
    let mut s = Session::new();
    install(&mut s, save_manifest("org.test.nosave", false), &saver(&[("a.txt", "text/plain", b"hi")], 0));
    let out = ex(&mut s, "plugin.run", json!({"id": "org.test.nosave"}));
    assert!(out["output"].as_str().unwrap_or_default().contains("permission"), "{out}");
    assert!(saves(&s).is_empty(), "no dialog without the permission");
}

#[test]
fn a_plugin_with_the_permission_requests_one_save_dialog_with_its_bytes() {
    let mut s = Session::new();
    install(&mut s, save_manifest("org.test.save", true), &saver(&[("out.jsx", "TEXT/Javascript", b"\x00hi\xff")], 0));
    ex(&mut s, "plugin.run", json!({"id": "org.test.save"}));
    let reqs = saves(&s);
    let [r] = reqs.as_slice() else { panic!("one request, got {}", reqs.len()) };
    assert_eq!(r.data["plugin"], "org.test.save");
    assert_eq!(r.data["name"], "out.jsx");
    assert_eq!(r.data["mime"], "text/javascript");
    assert_eq!(r.data["data"], b64(b"\x00hi\xff"));
    // Nothing in the request is a path.
    assert!(r.data.get("path").is_none());
}

#[test]
fn a_second_file_in_one_run_and_bad_names_are_refused() {
    let mut s = Session::new();
    install(&mut s, save_manifest("org.test.two", true), &saver(&[("a.txt", "text/plain", b"1"), ("b.txt", "text/plain", b"2")], 0));
    let out = ex(&mut s, "plugin.run", json!({"id": "org.test.two"}));
    assert_eq!(saves(&s).len(), 1, "only the first file");
    assert!(out["output"].as_str().unwrap_or_default().contains("one file"), "{out}");
    s.ui_requests.clear();
    install(&mut s, save_manifest("org.test.bad", true), &saver(&[("../etc/passwd", "text/plain", b"1")], 0));
    let out = ex(&mut s, "plugin.run", json!({"id": "org.test.bad"}));
    assert!(saves(&s).is_empty());
    assert!(out["output"].as_str().unwrap_or_default().contains("file name"), "{out}");
}

#[test]
fn an_odd_mime_type_becomes_generic_binary() {
    let mut s = Session::new();
    install(&mut s, save_manifest("org.test.mime", true), &saver(&[("a.bin", "not a type", b"1")], 0));
    ex(&mut s, "plugin.run", json!({"id": "org.test.mime"}));
    assert_eq!(saves(&s)[0].data["mime"], "application/octet-stream");
}

#[test]
fn a_run_that_fails_afterwards_leaves_no_dialog_behind() {
    let mut s = Session::new();
    install(&mut s, save_manifest("org.test.late", true), &saver(&[("a.txt", "text/plain", b"1")], 1));
    run_err(&mut s, "org.test.late");
    assert!(saves(&s).is_empty(), "a failed run opens no dialog");
}

#[test]
fn file_commands_stay_closed_to_plugins() {
    let mut s = Session::new();
    let calls = [("file.saveAs", json!({"path": "/tmp/x.figcraft"}))];
    install(&mut s, manifest("org.test.files", "read", &[]), &script(&calls, OnRefusal::Report, ""));
    let out = ex(&mut s, "plugin.run", json!({"id": "org.test.files"}));
    assert!(out["output"].as_str().unwrap_or_default().contains("isn't available to plugins"), "{out}");
}

#[test]
fn save_names_are_plain_file_names() {
    use crate::plugins::sanitize_save_name as clean;
    let long = "a".repeat(129);
    let max = "a".repeat(128);
    // 128 two-byte characters are within the limit even though they take 256 bytes.
    let wide = "é".repeat(128);
    for bad in ["", "   ", ".", "..", "../x", "a/b", "a\\b", "C:x", "a..b", "a\nb", "a\u{0}b", "a\u{7f}b", "a*b", long.as_str()] {
        assert_eq!(clean(bad), None, "{bad:?}");
    }
    assert_eq!(clean(" out.jsx ").as_deref(), Some("out.jsx"));
    assert_eq!(clean(&max).as_deref(), Some(max.as_str()));
    assert_eq!(clean(&wide).as_deref(), Some(wide.as_str()));
    assert_eq!(clean("Entwurf – Büro.jsx").as_deref(), Some("Entwurf – Büro.jsx"));
}

#[test]
fn reinstalling_under_an_existing_id_drops_the_old_settings() {
    let mut s = Session::new();
    let m = manifest("org.test.reuse", "read", &[]);
    install(&mut s, m.clone(), &script(&[], OnRefusal::Fail, ""));
    s.prefs.plugin_settings.entry("org.test.reuse".into()).or_default().insert("key".into(), json!("v"));
    install(&mut s, m, &script(&[], OnRefusal::Fail, ""));
    assert!(!s.prefs.plugin_settings.contains_key("org.test.reuse"), "{:?}", s.prefs.plugin_settings);
}
