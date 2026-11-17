//! The plugin directory: publishing (checked in full, key only), listing and searching, fetching
//! (hash-checked, within a budget), unpublishing and its caps in the store; then end to end over
//! loopback: the app's commands publish a plugin folder, another session browses without the key,
//! installs with confirmation, runs, updates and removes it; the raw protocol refuses wrong keys and
//! stray messages; and the server's command line lists and moderates.

use std::path::Path;

use craft_plugin::bundle::{Asset, Bundle, sha256_hex};
use figcraft_collab::message::{ClientMsg, OpenPlugins, PROTOCOL, PluginReply, PluginReq, ServerMsg};
use serde_json::json;

use crate::cli::{self, Cli, Command};
use crate::directory::{DirectoryConfig, INDEX, PLUGINS};
use crate::store::{Store, StoreConfig};
use crate::test_util::{Raw, TempDir, config, key, session};

pub(crate) fn wasm() -> Vec<u8> {
    wat::parse_str(r#"(module (memory (export "memory") 1) (func (export "figcraft_run") (param i32) (result i32) i32.const 0))"#).unwrap()
}

pub(crate) fn manifest(id: &str, version: &str) -> String {
    json!({"id": id, "name": "Tiny tool", "version": version, "author": "Ada Example", "description": "Does very little.", "main": "tiny.wasm",
           "permissions": {"access": "write", "commands": ["shape.rectangle"]}})
    .to_string()
}

fn bundle(id: &str, version: &str) -> Vec<u8> {
    Bundle { manifest: manifest(id, version), wasm: wasm(), assets: vec![Asset { name: "icon.png".into(), data: vec![7; 10] }] }.encode()
}

fn cfg(dir: &Path, f: impl Fn(&mut DirectoryConfig)) -> StoreConfig {
    let mut c = config(dir).store();
    f(&mut c.directory);
    c
}

const ID: &str = "org.example.tiny";

#[test]
fn publish_list_fetch_unpublish_and_restart() {
    let dir = TempDir::new("plugins-store");
    let mut store = Store::open(dir.path(), cfg(dir.path(), |_| {})).unwrap();
    let d = store.directory();
    let first = d.publish(&bundle(ID, "1.0.0")).unwrap();
    assert_eq!((first.name.as_str(), first.author.as_str(), first.access.as_str()), ("Tiny tool", "Ada Example", "write"));
    assert_eq!(first.commands, ["shape.rectangle"]);
    assert_eq!(first.sha256, sha256_hex(&bundle(ID, "1.0.0")));
    assert!(d.publish(&bundle(ID, "1.0.0")).unwrap_err().contains("newer"));
    assert!(d.publish(&bundle(ID, "0.9")).unwrap_err().contains("newer"));
    d.publish(&bundle(ID, "1.10.0")).unwrap();
    d.publish(&bundle("org.example.other", "2.0.0")).unwrap();
    let all = d.list("");
    assert_eq!(all.len(), 2);
    let tiny = all.iter().find(|e| e.id == ID).unwrap();
    assert_eq!((tiny.version.as_str(), tiny.versions.clone()), ("1.10.0", vec!["1.10.0".to_string(), "1.0.0".to_string()]));
    assert_eq!(d.list("ADA EXAMPLE").len(), 2, "search covers the author, in any case");
    assert!(d.list("nothing like it").is_empty());
    let (e, bytes) = d.fetch(ID, None).unwrap();
    assert_eq!((e.version.as_str(), sha256_hex(&bytes)), ("1.10.0", e.sha256.clone()));
    assert_eq!(d.fetch(ID, Some("1.0")).unwrap().0.version, "1.0.0");
    assert!(d.fetch(ID, Some("3.0.0")).is_err() && d.fetch("org.nope", None).is_err());
    // Unpublishing one version deletes its bundle.
    let file = dir.path().join(PLUGINS).join(format!("{}.bin", first.sha256));
    assert!(file.exists());
    assert_eq!(d.unpublish(ID, Some("1.0.0")).unwrap(), ["1.0.0"]);
    assert!(!file.exists());
    assert!(d.unpublish(ID, Some("1.0.0")).is_err());
    drop(store);
    // Kept across a restart; a bundle damaged on disk is never sent, a missing one is skipped.
    let mut store = Store::open(dir.path(), cfg(dir.path(), |_| {})).unwrap();
    assert_eq!(store.directory().entries().len(), 2);
    let latest = store.directory().fetch(ID, None).unwrap().0;
    let path = dir.path().join(PLUGINS).join(format!("{}.bin", latest.sha256));
    let mut damaged = std::fs::read(&path).unwrap();
    let last = damaged.len() - 1;
    damaged[last] ^= 0xFF;
    std::fs::write(&path, &damaged).unwrap();
    assert!(store.directory().fetch(ID, None).unwrap_err().contains("damaged"));
    std::fs::write(&path, b"short").unwrap();
    drop(store);
    let store = Store::open(dir.path(), cfg(dir.path(), |_| {})).unwrap();
    assert!(store.problems().iter().any(|p| p.contains(ID) && p.contains("damaged")), "{:?}", store.problems());
    drop(store);
    // A damaged index lists nothing but deletes nothing.
    std::fs::write(dir.path().join(PLUGINS).join(INDEX), b"{not json").unwrap();
    let store = Store::open(dir.path(), cfg(dir.path(), |_| {})).unwrap();
    assert!(store.problems().iter().any(|p| p.contains(INDEX)));
    let mut store = store;
    assert!(store.directory().publish(&bundle(ID, "9.0.0")).unwrap_err().contains("damaged"), "nothing is written over a damaged index");
    assert_eq!(std::fs::read(dir.path().join(PLUGINS).join(INDEX)).unwrap(), b"{not json");
    assert!(dir.path().join(PLUGINS).join(format!("{}.bin", latest.sha256)).exists());
}

#[test]
fn bad_bundles_caps_and_keys() {
    let dir = TempDir::new("plugins-bad");
    let mut store = Store::open(dir.path(), cfg(dir.path(), |d| d.max_plugin_bytes = 64 << 10)).unwrap();
    let d = store.directory();
    let with = |f: &dyn Fn(&mut Bundle)| {
        let mut b = Bundle::decode(&bundle(ID, "1.0.0"), usize::MAX).unwrap();
        f(&mut b);
        b.encode()
    };
    let wasi = wat::parse_str(r#"(module (import "wasi_snapshot_preview1" "proc_exit" (func (param i32))) (memory (export "memory") 1) (func (export "figcraft_run") (param i32) (result i32) i32.const 0))"#).unwrap();
    for (bytes, why) in [
        (b"PK\x03\x04 a zip".to_vec(), "isn't"),
        (vec![0u8; 70 << 10], "keeps plugins up to"),
        (with(&|b| b.manifest = "{}".into()), "manifest"),
        (with(&|b| b.manifest = manifest(ID, "1.0.0").replace("\"tiny.wasm\"", "\"tiny.wasm\", \"api\": 99")), "API"),
        (with(&|b| b.manifest = manifest(ID, "latest")), "version"),
        (with(&|b| b.wasm = b"not wasm".to_vec()), "WebAssembly"),
        (with(&|b| b.wasm = wasi.clone()), "imports"),
        (with(&|b| b.assets[0].name = "../../server.key".into()), "asset"),
        (with(&|b| b.assets[0].name = "aux.png".into()), "asset"),
        (with(&|b| b.assets[0].data = vec![0; 70 << 10]), "keeps plugins up to"),
    ] {
        let e = d.publish(&bytes).unwrap_err();
        assert!(e.contains(why), "{why}: {e}");
    }
    assert!(d.entries().is_empty());
    // Only with the key.
    assert!(matches!(d.handle(false, PluginReq::Publish { bundle: bundle(ID, "1.0.0") }), PluginReply::Failed { reason } if reason.contains("key")));
    assert!(matches!(d.handle(false, PluginReq::Unpublish { id: ID.into(), version: None }), PluginReply::Failed { .. }));
    assert!(matches!(d.handle(true, PluginReq::Publish { bundle: bundle(ID, "1.0.0") }), PluginReply::Published(_)));
    assert!(matches!(d.handle(false, PluginReq::List { query: "x".repeat(100_000) }), PluginReply::List(_)));
    // Fetches share a budget: a flood is turned away, not queued.
    let busy = (0..800).filter_map(|_| d.fetch(ID, None).err()).filter(|e| e.contains("busy")).count();
    assert!(busy > 0, "a flood of fetches hits the budget");
    drop(store);
    // Caps: versions kept, bytes kept, off.
    let dir = TempDir::new("plugins-caps");
    let size = bundle(ID, "1.0.0").len() as u64;
    let mut store = Store::open(dir.path(), cfg(dir.path(), |d| d.max_plugins = 2)).unwrap();
    store.directory().publish(&bundle(ID, "1.0.0")).unwrap();
    store.directory().publish(&bundle(ID, "1.0.1")).unwrap();
    assert!(store.directory().publish(&bundle(ID, "1.0.2")).unwrap_err().contains("at most 2"));
    drop(store);
    let dir = TempDir::new("plugins-full");
    let mut store = Store::open(dir.path(), cfg(dir.path(), |d| d.max_total_bytes = size + 10)).unwrap();
    store.directory().publish(&bundle(ID, "1.0.0")).unwrap();
    assert!(store.directory().publish(&bundle(ID, "1.0.1")).unwrap_err().contains("full"));
    drop(store);
    let dir = TempDir::new("plugins-off");
    let mut store = Store::open(dir.path(), cfg(dir.path(), |d| d.max_plugins = 0)).unwrap();
    assert!(
        matches!(store.directory().handle(true, PluginReq::List { query: String::new() }), PluginReply::Failed { reason } if reason.contains("off"))
    );
    assert!(!dir.path().join(PLUGINS).exists());
}

fn ex(s: &mut figcraft_engine::Session, cmd: &str, p: serde_json::Value) -> serde_json::Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

/// A plugin folder: manifest, module in a subfolder, an asset.
fn folder(root: &Path, version: &str) -> std::path::PathBuf {
    let dir = root.join(format!("tiny-{version}"));
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::write(dir.join("figcraft-plugin.json"), manifest(ID, version)).unwrap();
    std::fs::write(dir.join("tiny.wasm"), wasm()).unwrap();
    std::fs::write(dir.join("assets").join("icon.png"), [1, 2, 3]).unwrap();
    dir
}

#[test]
fn publish_browse_install_update_and_remove_over_the_network() {
    let dir = TempDir::new("plugins-net");
    let mut server = crate::start(&config(dir.path())).unwrap();
    let addr = server.addr();
    let with_key = format!("ws://{addr}/?key={}", key(dir.path()));
    let plain = format!("ws://{addr}/");
    // The publisher: the server's key.
    let mut publisher = session("Ada");
    let e = publisher.execute("plugin.directory.publish", &json!({"path": folder(dir.path(), "1.0.0"), "server": plain})).unwrap_err().to_string();
    assert!(e.contains("key"), "{e}");
    let r = ex(&mut publisher, "plugin.directory.publish", json!({"path": folder(dir.path(), "1.0.0"), "server": with_key}));
    assert_eq!(r["published"]["version"], "1.0.0");
    let wrong = format!("ws://{addr}/?key=0123456789abcdef0123");
    assert!(
        publisher
            .execute("plugin.directory.publish", &json!({"path": folder(dir.path(), "1.0.1"), "server": wrong}))
            .unwrap_err()
            .to_string()
            .contains("wrong server key")
    );
    // A user: browses without the key, installs only with confirmation, runs it.
    let mut user = session("Bo");
    let listed = ex(&mut user, "plugin.directory.list", json!({"server": plain}));
    let p = &listed["plugins"][0];
    assert_eq!((p["id"].as_str(), p["access"].as_str(), p["installed"].as_str()), (Some(ID), Some("write"), None));
    assert!(user.execute("plugin.directory.install", &json!({"id": ID})).is_err());
    let sha = p["sha256"].as_str().unwrap().to_string();
    let r = ex(&mut user, "plugin.directory.install", json!({"id": ID, "confirm": true, "sha256": sha}));
    assert_eq!((r["version"].as_str(), r["source"].as_str()), (Some("1.0.0"), Some(plain.as_str())));
    ex(&mut user, "plugin.run", json!({"id": ID}));
    // An update appears and installs over it; browsing users can't unpublish.
    ex(&mut publisher, "plugin.directory.publish", json!({"path": folder(dir.path(), "1.0.1"), "server": with_key}));
    let listed = ex(&mut user, "plugin.directory.list", json!({}));
    assert_eq!((listed["plugins"][0]["version"].as_str(), listed["plugins"][0]["update"].as_bool()), (Some("1.0.1"), Some(true)));
    let r = ex(&mut user, "plugin.directory.install", json!({"id": ID, "confirm": true}));
    assert_eq!((r["version"].as_str(), r["replaced"].as_bool()), (Some("1.0.1"), Some(true)));
    assert!(user.execute("plugin.directory.unpublish", &json!({"id": ID, "server": plain})).unwrap_err().to_string().contains("key"));
    ex(&mut user, "plugin.remove", json!({"id": ID}));
    let r = ex(&mut publisher, "plugin.directory.unpublish", json!({"id": ID, "server": with_key}));
    assert_eq!(r["removed"], json!(["1.0.0", "1.0.1"]));
    assert!(ex(&mut user, "plugin.directory.list", json!({}))["plugins"].as_array().unwrap().is_empty());
    server.stop();
}

#[test]
fn the_raw_protocol_refuses_wrong_keys_and_stray_messages() {
    let dir = TempDir::new("plugins-raw");
    let mut server = crate::start(&config(dir.path())).unwrap();
    let addr = server.addr();
    let open = |key: &str| ClientMsg::OpenPlugins(OpenPlugins { protocol: PROTOCOL, key: key.into(), client_version: String::new() });
    let answer = |raw: &mut Raw| match raw.next() {
        ServerMsg::Plugins(r) => *r,
        other => panic!("not a directory answer: {other:?}"),
    };
    let mut raw = Raw::connect(addr);
    raw.send(&open("not the key"));
    assert!(matches!(raw.next(), ServerMsg::Refused { reason } if reason.contains("wrong server key")));
    let mut raw = Raw::connect(addr);
    raw.send(&ClientMsg::OpenPlugins(OpenPlugins { protocol: PROTOCOL + 1, key: String::new(), client_version: String::new() }));
    assert!(matches!(raw.next(), ServerMsg::Refused { reason } if reason.contains("protocol")));
    let mut raw = Raw::connect(addr);
    raw.send(&open(""));
    assert_eq!(answer(&mut raw), PluginReply::Ready { publisher: false });
    raw.send(&ClientMsg::Plugins(PluginReq::Publish { bundle: bundle(ID, "1.0.0") }));
    assert!(matches!(answer(&mut raw), PluginReply::Failed { reason } if reason.contains("key")));
    raw.send(&ClientMsg::Plugins(PluginReq::Fetch { id: "../../server.key".into(), version: Some("..".into()) }));
    assert!(matches!(answer(&mut raw), PluginReply::Failed { .. }));
    raw.send(&ClientMsg::Ping);
    assert_eq!(raw.next(), ServerMsg::Pong);
    raw.send(&ClientMsg::Admin(figcraft_collab::message::Admin::Files));
    assert!(matches!(raw.next(), ServerMsg::Refused { reason } if reason.contains("plugin directory")));
    let mut raw = Raw::connect(addr);
    raw.send(&open(&key(dir.path())));
    assert_eq!(answer(&mut raw), PluginReply::Ready { publisher: true });
    raw.send(&ClientMsg::Plugins(PluginReq::Publish { bundle: bundle(ID, "1.0.0") }));
    assert!(matches!(answer(&mut raw), PluginReply::Published(e) if e.id == ID));
    server.stop();
}

#[test]
fn the_command_line_lists_and_moderates() {
    let dir = TempDir::new("plugins-cli");
    let run = |command: Command| cli::run(&Cli { config: config(dir.path()), command });
    assert!(run(Command::Plugins).unwrap().contains("No plugins yet"));
    Store::open(dir.path(), config(dir.path()).store()).unwrap().directory().publish(&bundle(ID, "1.0.0")).unwrap();
    let table = run(Command::Plugins).unwrap();
    assert!(table.contains(ID) && table.contains("Ada Example") && table.contains("write"), "{table}");
    // Through the running server too.
    let mut server = crate::start(&config(dir.path())).unwrap();
    assert!(run(Command::Plugins).unwrap().contains(ID));
    assert!(run(Command::RemovePlugin { id: ID.into(), version: Some("2.0.0".into()) }).is_err());
    assert!(run(Command::RemovePlugin { id: ID.into(), version: None }).unwrap().contains("Unpublished"));
    server.stop();
    assert!(run(Command::Plugins).unwrap().contains("No plugins yet"));
    let parsed = cli::parse(&["plugins".to_string(), "remove".into(), ID.into(), "--version".into(), "1.0.0".into()]).unwrap();
    assert_eq!(parsed.command, Command::RemovePlugin { id: ID.into(), version: Some("1.0.0".into()) });
    assert_eq!(cli::parse(&["--version".to_string()]).unwrap().command, Command::Version);
}
