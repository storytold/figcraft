//! Plugins from bundle files, catalogs and GitHub releases (`plugin.install`, `plugin.update`,
//! `plugin.catalog.*`), kept in craft-plugin's store: nothing is written before `confirm: true`,
//! downloads go through the session's link and are checked, the store lists them again at the next
//! start, plugins written for another app or of another kind are refused, and runs of a
//! `craft_run` module are one undo step like any plugin's.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use craft_plugin::bundle::{Bundle, sha256_hex};
use figcraft_collab::message::ClientMsg;
use serde_json::{Value, json};

use crate::components::tests::ex;
use crate::live::{Event, HostOptions, LiveLink, Presence, SessionInfo};
use crate::plugins_tests::{b64, layer_count, lit, undo_len};
use crate::{EngineError, Session};

const ID: &str = "io.github.example.boxes";
const API: &str = "https://api.github.com/repos/example/boxes/releases/latest";
const ASSET: &str = "https://github.com/example/boxes/releases/download/v1.0.0/boxes.craftplugin";
const SUMS: &str = "https://github.com/example/boxes/releases/download/v1.0.0/SHA256SUMS";
const ASSET2: &str = "https://github.com/example/boxes/releases/download/v1.1.0/boxes.craftplugin";
const SUMS2: &str = "https://github.com/example/boxes/releases/download/v1.1.0/SHA256SUMS";
const CATALOG: &str = "https://raw.githubusercontent.com/example/catalog/main/index.json";

/// A link that serves fixed HTTPS bodies and counts what was asked for.
#[derive(Default)]
struct Web {
    files: HashMap<String, Vec<u8>>,
    asked: Vec<String>,
}

struct Link(Arc<Mutex<Web>>);

impl LiveLink for Link {
    fn host(&mut self, _: u64, _: HostOptions) -> Result<SessionInfo, String> {
        Err("no".into())
    }
    fn join(&mut self, _: u64, _: &str, _: &str) -> Result<(), String> {
        Err("no".into())
    }
    fn leave(&mut self, _: u64) {}
    fn presence(&mut self, _: u64, _: &Presence) {}
    fn send(&mut self, _: u64, _: ClientMsg) {}
    fn poll(&mut self) -> Vec<(u64, Event)> {
        Vec::new()
    }
    fn fetch(&mut self, url: &str, max: usize) -> Result<Option<Vec<u8>>, String> {
        let mut w = self.0.lock().unwrap();
        w.asked.push(url.to_string());
        match w.files.get(url) {
            Some(b) if b.len() > max => Err(format!("more than {max} bytes")),
            Some(b) => Ok(Some(b.clone())),
            None => Ok(None),
        }
    }
}

/// A folder removed when the test ends.
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("figcraft-plugin-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A module under the shared ABI's names (`craft`, `craft_run`) that runs `cmd` with `params`
/// once and fails with the refusal when it is refused.
fn module(cmd: &str, params: &Value) -> Vec<u8> {
    let p = params.to_string();
    let wat = format!(
        r#"(module
          (import "craft" "call" (func $call (param i32 i32 i32 i32) (result i32)))
          (import "craft" "reply_read" (func $reply_read (param i32 i32) (result i32)))
          (import "craft" "set_output" (func $set_output (param i32 i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "{}") (data (i32.const {}) "{}")
          (func (export "craft_run") (param i32) (result i32)
            (if (call $call (i32.const 0) (i32.const {}) (i32.const {}) (i32.const {})) (then (call $set_output (i32.const 4096) (call $reply_read (i32.const 4096) (i32.const 4096))) (return (i32.const 1))))
            (i32.const 0)))"#,
        lit(cmd),
        cmd.len(),
        lit(&p),
        cmd.len(),
        cmd.len(),
        p.len()
    );
    wat::parse_str(&wat).unwrap()
}

fn rectangle() -> Vec<u8> {
    module("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10}))
}

/// A bundle of plugin `id` at `version`, written for `apps`, asking for `access`.
fn bundle_of(id: &str, version: &str, access: &str, apps: Value, wasm: Vec<u8>) -> Vec<u8> {
    let manifest = json!({"id": id, "name": "Boxes", "version": version, "main": "boxes.wasm", "kind": "command", "apps": apps,
        "repo": "https://github.com/example/boxes", "permissions": {"access": access, "commands": ["shape.*"]}});
    Bundle { manifest: manifest.to_string(), wasm, assets: Vec::new() }.encode()
}

fn boxes(version: &str) -> Vec<u8> {
    bundle_of(ID, version, "write", json!({"figcraft": {"api": 6}}), rectangle())
}

fn release(tag: &str, asset: &str, sums: &str) -> Vec<u8> {
    json!({"tag_name": tag, "assets": [
        {"name": "boxes.craftplugin", "size": 0, "browser_download_url": asset},
        {"name": "SHA256SUMS", "size": 100, "browser_download_url": sums}]})
    .to_string()
    .into_bytes()
}

/// A session whose link serves release v1.0.0 of example/boxes, and its web.
fn online() -> (Session, Arc<Mutex<Web>>) {
    let web = Arc::new(Mutex::new(Web::default()));
    let bytes = boxes("1.0.0");
    {
        let mut w = web.lock().unwrap();
        w.files.insert(API.into(), release("v1.0.0", ASSET, SUMS));
        w.files.insert(SUMS.into(), format!("{}  boxes.craftplugin\n", sha256_hex(&bytes)).into_bytes());
        w.files.insert(ASSET.into(), bytes);
    }
    let mut s = Session::new();
    s.live.install(Box::new(Link(web.clone())));
    (s, web)
}

fn err(s: &mut Session, cmd: &str, p: Value) -> String {
    match s.execute(cmd, &p) {
        Ok(v) => panic!("{cmd} succeeded: {v}"),
        Err(e) => {
            assert!(!matches!(e, EngineError::Internal { .. }), "{e}");
            e.to_string()
        }
    }
}

fn installed(s: &Session) -> Vec<String> {
    s.prefs.plugins.iter().map(|p| p.manifest.id.clone()).collect()
}

#[test]
fn a_bundle_file_is_shown_then_installed_into_the_store_and_listed_again_at_start() {
    let dir = TempDir::new("file");
    let mut s = Session::new();
    s.plugins.store_dir = Some(dir.0.clone());
    let bytes = boxes("1.0.0");
    let shown = ex(&mut s, "plugin.install", json!({"data": b64(&bytes), "name": "boxes.craftplugin"}));
    assert_eq!(shown["installed"], json!(false));
    assert_eq!(shown["sha256"], json!(sha256_hex(&bytes)));
    assert_eq!(shown["permissions"]["access"], json!("write"));
    assert_eq!(shown["kind"], json!("command"));
    assert!(installed(&s).is_empty() && !dir.0.exists(), "nothing is written before confirm");
    let out = ex(&mut s, "plugin.install", json!({"data": b64(&bytes), "confirm": true, "sha256": sha256_hex(&bytes)}));
    assert_eq!((out["installed"].clone(), out["store"].clone()), (json!(true), json!(true)));
    assert_eq!(installed(&s), [ID]);
    // A craft_run module runs a real command as one undo step named after the plugin.
    let (layers, undo) = (layer_count(&s), undo_len(&s));
    ex(&mut s, "plugin.run", json!({"id": ID}));
    assert_eq!((layer_count(&s), undo_len(&s)), (layers + 1, undo + 1));
    assert_eq!(s.ds().unwrap().history.undo.last().unwrap().label, "Boxes");
    // Not saved with the preferences: the store lists it at the next start.
    let prefs = serde_json::to_value(&s.prefs).unwrap();
    assert_eq!(prefs["plugins"], json!([]));
    let mut next = Session::new();
    next.plugins.store_dir = Some(dir.0.clone());
    assert_eq!(crate::plugin_store::load_store(&mut next), 1);
    assert_eq!(installed(&next), [ID]);
    assert_eq!(ex(&mut next, "plugin.list", json!({}))["plugins"][0]["store"], json!(true));
    ex(&mut next, "plugin.run", json!({"id": ID}));
    // Removing it removes its folder.
    ex(&mut next, "plugin.remove", json!({"id": ID}));
    let mut again = Session::new();
    again.plugins.store_dir = Some(dir.0.clone());
    assert_eq!(crate::plugin_store::load_store(&mut again), 0);
}

#[test]
fn a_shown_sha256_must_match_what_is_installed() {
    let mut s = Session::new();
    let e = err(&mut s, "plugin.install", json!({"data": b64(&boxes("1.0.0")), "confirm": true, "sha256": "0".repeat(64)}));
    assert!(e.contains("isn't the version that was shown"), "{e}");
    assert!(installed(&s).is_empty());
}

#[test]
fn plugins_for_another_app_or_of_another_kind_are_refused() {
    let mut s = Session::new();
    let other = bundle_of(ID, "1.0.0", "write", json!({"designcraft": {"api": 6}}), rectangle());
    let e = err(&mut s, "plugin.install", json!({"data": b64(&other)}));
    assert!(e.contains("written for designcraft"), "{e}");
    let manifest = json!({"id": ID, "name": "Boxes", "version": "1.0.0", "main": "b.wasm", "kind": "filter", "apps": {"figcraft": {"api": 1}}});
    let e = err(&mut s, "plugin.import", json!({"manifest": manifest, "wasm": b64(&rectangle())}));
    assert!(e.contains("`filter` plug-in"), "{e}");
    // A legacy manifest (no `apps`) still imports and runs.
    let legacy = json!({"id": ID, "name": "Boxes", "version": "1.0.0", "main": "b.wasm", "permissions": {"access": "write"}});
    ex(&mut s, "plugin.import", json!({"manifest": legacy, "wasm": b64(&rectangle())}));
    ex(&mut s, "plugin.run", json!({"id": ID}));
}

#[test]
fn a_repository_release_is_shown_then_installed_without_downloading_it_again() {
    let dir = TempDir::new("repo");
    let (mut s, web) = online();
    s.plugins.store_dir = Some(dir.0.clone());
    let shown = ex(&mut s, "plugin.install", json!({"repo": "example/boxes"}));
    assert_eq!(shown["installed"], json!(false));
    assert_eq!(shown["inCatalog"], json!(false), "the confirmation says it is in no catalog");
    assert_eq!(shown["from"], json!("github.com/example/boxes@v1.0.0"));
    assert_eq!(web.lock().unwrap().asked, [API, SUMS, ASSET]);
    assert!(!dir.0.exists());
    let sha = shown["sha256"].clone();
    let out = ex(&mut s, "plugin.install", json!({"repo": "example/boxes", "confirm": true, "sha256": sha}));
    assert_eq!(out["installed"], json!(true));
    assert_eq!(web.lock().unwrap().asked.len(), 3, "the bundle shown is the one installed");
    assert_eq!(installed(&s), [ID]);
    let stored = craft_plugin::store::Store::new(dir.0.clone()).load(ID).unwrap().unwrap();
    assert_eq!(
        stored.record.source,
        craft_plugin::store::Source::Github { repo: "example/boxes".into(), tag: "v1.0.0".into(), asset: "boxes.craftplugin".into() }
    );
}

#[test]
fn a_repository_release_with_a_wrong_checksum_is_refused() {
    let (mut s, web) = online();
    web.lock().unwrap().files.insert(SUMS.into(), format!("{}  boxes.craftplugin\n", "ab".repeat(32)).into_bytes());
    let e = err(&mut s, "plugin.install", json!({"repo": "example/boxes", "confirm": true}));
    assert!(e.contains("SHA-256"), "{e}");
    assert!(installed(&s).is_empty());
    let e = err(&mut s, "plugin.install", json!({"repo": "not a repo"}));
    assert!(!e.is_empty());
}

#[test]
fn without_a_link_that_downloads_github_is_unavailable() {
    let mut s = Session::new();
    let e = err(&mut s, "plugin.install", json!({"repo": "example/boxes"}));
    assert!(e.contains(crate::plugin_store::NO_HTTP), "{e}");
}

fn catalog(bytes: &[u8], version: &str, url: &str) -> Vec<u8> {
    json!({"schema": 1, "packages": [{"id": ID, "name": "Boxes", "repo": "https://github.com/example/boxes",
        "versions": [{"version": version, "apps": {"figcraft": {"api": 6}}, "url": url, "sha256": sha256_hex(bytes), "size": bytes.len()}]}]})
    .to_string()
    .into_bytes()
}

#[test]
fn catalogs_are_added_listed_and_installed_from() {
    let dir = TempDir::new("catalog");
    let (mut s, web) = online();
    s.plugins.store_dir = Some(dir.0.clone());
    let bytes = boxes("1.0.0");
    web.lock().unwrap().files.insert(CATALOG.into(), catalog(&bytes, "1.0.0", ASSET));
    let e = err(&mut s, "plugin.catalog.add", json!({"url": "https://example.com/index.json"}));
    assert!(!e.is_empty());
    let added = ex(&mut s, "plugin.catalog.add", json!({"url": CATALOG}));
    assert_eq!((added["added"].clone(), added["packages"].clone()), (json!(true), json!(1)));
    assert_eq!(s.prefs.plugin_catalogs, [CATALOG]);
    let list = ex(&mut s, "plugin.catalog.list", json!({}));
    assert_eq!(list["plugins"][0]["id"], json!(ID));
    assert_eq!(list["plugins"][0]["installed"], Value::Null);
    // A repository a catalog lists is said to be in one.
    assert_eq!(ex(&mut s, "plugin.install", json!({"repo": "example/boxes"}))["inCatalog"], json!(true));
    let shown = ex(&mut s, "plugin.install", json!({"catalog": CATALOG, "id": ID}));
    assert_eq!(shown["from"], json!(format!("{ID} in {CATALOG}")));
    ex(&mut s, "plugin.install", json!({"catalog": CATALOG, "id": ID, "confirm": true, "sha256": shown["sha256"]}));
    assert_eq!(installed(&s), [ID]);
    assert_eq!(ex(&mut s, "plugin.catalog.list", json!({}))["plugins"][0]["installed"], json!("1.0.0"));
    // A catalog whose pinned SHA-256 doesn't match the download installs nothing.
    let other = boxes("1.0.0");
    let mut wrong = catalog(&other, "1.0.0", ASSET);
    wrong = String::from_utf8(wrong).unwrap().replace(&sha256_hex(&other), &"0".repeat(64)).into_bytes();
    web.lock().unwrap().files.insert(CATALOG.into(), wrong);
    let e = err(&mut s, "plugin.install", json!({"catalog": CATALOG, "id": ID, "confirm": true}));
    assert!(e.contains("SHA-256"), "{e}");
    assert_eq!(ex(&mut s, "plugin.catalog.remove", json!({"url": CATALOG}))["removed"], json!(true));
    assert!(s.prefs.plugin_catalogs.is_empty());
}

#[test]
fn updates_are_found_and_installed_from_the_repository() {
    let dir = TempDir::new("update");
    let (mut s, web) = online();
    s.plugins.store_dir = Some(dir.0.clone());
    let sha = ex(&mut s, "plugin.install", json!({"repo": "example/boxes"}))["sha256"].clone();
    ex(&mut s, "plugin.install", json!({"repo": "example/boxes", "confirm": true, "sha256": sha}));
    assert_eq!(ex(&mut s, "plugin.update", json!({}))["updates"], json!([]));
    let next = boxes("1.1.0");
    {
        let mut w = web.lock().unwrap();
        w.files.insert(API.into(), release("v1.1.0", ASSET2, SUMS2));
        w.files.insert(SUMS2.into(), format!("{}  boxes.craftplugin\n", sha256_hex(&next)).into_bytes());
        w.files.insert(ASSET2.into(), next.clone());
    }
    let updates = ex(&mut s, "plugin.update", json!({}));
    assert_eq!(updates["updates"][0]["id"], json!(ID));
    assert_eq!(updates["updates"][0]["sha256"], json!(sha256_hex(&next)));
    let shown = ex(&mut s, "plugin.update", json!({"id": ID}));
    assert_eq!((shown["version"].clone(), shown["installedVersion"].clone()), (json!("1.1.0"), json!("1.0.0")));
    assert_eq!(shown["permissionsChanged"], json!(false));
    assert_eq!(s.prefs.plugins[0].manifest.version, "1.0.0", "nothing installed before confirm");
    ex(&mut s, "plugin.update", json!({"id": ID, "confirm": true, "sha256": sha256_hex(&next)}));
    assert_eq!(s.prefs.plugins[0].manifest.version, "1.1.0");
    assert_eq!(craft_plugin::store::Store::new(dir.0.clone()).load(ID).unwrap().unwrap().record.version, "1.1.0");
}

#[test]
fn a_read_plugin_from_the_store_that_edits_is_refused_and_undone() {
    let mut s = Session::new();
    let reader = bundle_of(ID, "1.0.0", "read", json!({"figcraft": {"api": 6}}), rectangle());
    let sha = sha256_hex(&reader);
    ex(&mut s, "plugin.install", json!({"data": b64(&reader), "confirm": true, "sha256": sha}));
    let (layers, undo) = (layer_count(&s), undo_len(&s));
    let e = err(&mut s, "plugin.run", json!({"id": ID}));
    assert!(e.contains("may only read"), "{e}");
    assert_eq!((layer_count(&s), undo_len(&s)), (layers, undo));
    // A refused command (files) is refused the same way.
    let saver = bundle_of("io.github.example.saver", "1.0.0", "write", json!({"figcraft": {"api": 6}}), module("file.save", &json!({})));
    let sha = sha256_hex(&saver);
    ex(&mut s, "plugin.install", json!({"data": b64(&saver), "confirm": true, "sha256": sha}));
    let e = err(&mut s, "plugin.run", json!({"id": "io.github.example.saver"}));
    assert!(e.contains("isn't available to plugins"), "{e}");
    assert_eq!((layer_count(&s), undo_len(&s)), (layers, undo));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_development_folder_with_craft_plugin_json_imports() {
    let dir = TempDir::new("dev");
    std::fs::create_dir_all(&dir.0).unwrap();
    let manifest = json!({"id": ID, "name": "Boxes", "version": "1.0.0", "main": "boxes.wasm", "apps": {"figcraft": {"api": 6}},
        "permissions": {"access": "write"}});
    std::fs::write(dir.0.join("craft-plugin.json"), manifest.to_string()).unwrap();
    std::fs::write(dir.0.join("boxes.wasm"), rectangle()).unwrap();
    let out = ex(&mut Session::new(), "plugin.import", json!({"path": dir.0.to_string_lossy()}));
    assert_eq!(out["id"], json!(ID));
    assert!(out["path"].as_str().unwrap().ends_with("craft-plugin.json"));
}

#[test]
fn the_shared_policy_refuses_what_figcraft_always_refused() {
    let policy = &crate::plugins::POLICY;
    for refused in [
        "widget.insert",
        "file.save",
        "doc.open",
        "version.save",
        "app.quit",
        "pref.set",
        "help.about",
        "plugin.install",
        "plugins.manage",
        "libraries.open",
        "library.add",
        "library.addAll",
        "library.check",
        "library.publishToServer",
        "edit.undo",
        "edit.redo",
        "edit.repeat",
        "command.batch",
        "data.fill",
        "collab.host",
        "window.new",
        "view.theme",
    ] {
        assert!(policy.refuses(refused), "{refused}");
    }
    for allowed in ["doc.list", "version.list", "library.list", "shape.rectangle", "view.get", "edit.duplicate"] {
        assert!(!policy.refuses(allowed), "{allowed}");
    }
}
