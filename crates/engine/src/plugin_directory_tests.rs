//! Installing from and publishing to a plugin directory, against a scripted directory (the real
//! server, over the network, is tested in `figcraft-server`): nothing is fetched without
//! `confirm: true`, a download that doesn't match its SHA-256 or its entry is never installed,
//! installed plugins run in the sandbox, update and are removed with their folder, and publishing
//! sends a plugin folder as a checked bundle.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use figcraft_collab::message::{ClientMsg, PluginEntry, PluginReply, PluginReq};
use figcraft_plugin::bundle::{Asset, Bundle, sha256_hex};
use serde_json::{Value, json};

use crate::live::{Event, HostOptions, LiveLink, Presence, SessionInfo};
use crate::{EngineError, Session};

/// A scripted directory: its bundles by entry, the requests it got, and damage to apply.
#[derive(Default)]
struct Dir {
    plugins: Vec<(PluginEntry, Vec<u8>)>,
    requests: Vec<(String, PluginReq)>,
    /// Flip a byte of every bundle sent.
    tamper: bool,
    /// Send this id's bundle whatever was asked for.
    swap: Option<String>,
}

struct Link(Arc<Mutex<Dir>>);

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
    fn plugins(&mut self, server: &str, req: PluginReq) -> Result<PluginReply, String> {
        let mut d = self.0.lock().unwrap();
        d.requests.push((server.to_string(), req.clone()));
        Ok(match req {
            PluginReq::List { .. } => PluginReply::List(d.plugins.iter().map(|(e, _)| e.clone()).collect()),
            PluginReq::Fetch { id, version } => {
                let want = d.swap.clone().unwrap_or(id);
                match d.plugins.iter().rev().find(|(e, _)| e.id == want && version.as_ref().is_none_or(|v| *v == e.version)) {
                    Some((entry, bytes)) => {
                        let mut bundle = bytes.clone();
                        if d.tamper {
                            let last = bundle.len() - 1;
                            bundle[last] ^= 1;
                        }
                        PluginReply::Bundle { entry: entry.clone(), bundle }
                    }
                    None => PluginReply::Failed { reason: "no such plugin".into() },
                }
            }
            PluginReq::Publish { bundle } => {
                let b = Bundle::decode(&bundle, usize::MAX).map_err(|e| e.to_string())?;
                let m = b.check()?;
                let e = entry(&m.id, &m.version, &bundle);
                d.plugins.push((e.clone(), bundle));
                PluginReply::Published(e)
            }
            PluginReq::Unpublish { id, .. } => {
                d.plugins.retain(|(e, _)| e.id != id);
                PluginReply::Removed(vec!["all".into()])
            }
        })
    }
}

/// A module that does nothing and succeeds.
fn wasm() -> Vec<u8> {
    wat::parse_str(r#"(module (memory (export "memory") 1) (func (export "figcraft_run") (param i32) (result i32) i32.const 0))"#).unwrap()
}

fn manifest(id: &str, version: &str) -> String {
    json!({"id": id, "name": "Tiny", "version": version, "author": "Ada", "main": "tiny.wasm", "permissions": {"access": "read"}}).to_string()
}

fn bundle(id: &str, version: &str) -> Vec<u8> {
    Bundle { manifest: manifest(id, version), wasm: wasm(), assets: vec![Asset { name: "LICENSE".into(), data: b"MIT".to_vec() }] }.encode()
}

fn entry(id: &str, version: &str, bytes: &[u8]) -> PluginEntry {
    PluginEntry {
        id: id.into(),
        name: "Tiny".into(),
        version: version.into(),
        author: "Ada".into(),
        access: "read".into(),
        bytes: bytes.len() as u64,
        sha256: sha256_hex(bytes),
        ..PluginEntry::default()
    }
}

fn offer(d: &Arc<Mutex<Dir>>, id: &str, version: &str) {
    let b = bundle(id, version);
    d.lock().unwrap().plugins.push((entry(id, version, &b), b));
}

fn session() -> (Session, Arc<Mutex<Dir>>) {
    let mut s = Session::new();
    let d = Arc::new(Mutex::new(Dir::default()));
    s.live.install(Box::new(Link(d.clone())));
    (s, d)
}

fn fetches(d: &Arc<Mutex<Dir>>) -> usize {
    d.lock().unwrap().requests.iter().filter(|(_, r)| matches!(r, PluginReq::Fetch { .. })).count()
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("figcraft-plugdir-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const ID: &str = "org.example.tiny";

#[test]
fn listing_remembers_the_server_without_its_key() {
    let (mut s, d) = session();
    offer(&d, ID, "1.0.0");
    let r = s.execute("plugin.directory.list", &json!({"server": "ws://127.0.0.1:9/?key=secret"})).unwrap();
    assert_eq!(r["server"], "ws://127.0.0.1:9/");
    assert_eq!(r["plugins"][0]["id"], ID);
    assert_eq!(r["plugins"][0]["installed"], Value::Null);
    assert_eq!(s.prefs.plugin_directory.as_deref(), Some("ws://127.0.0.1:9/"));
    assert!(!serde_json::to_string(&s.prefs).unwrap().contains("secret"), "the key is never kept");
    // The server browsed last is the default.
    s.execute("plugin.directory.list", &json!({})).unwrap();
    assert_eq!(d.lock().unwrap().requests.last().unwrap().0, "ws://127.0.0.1:9/");
    // Without a link (the browser, a bare session) it's a clear error.
    let e = Session::new().execute("plugin.directory.list", &json!({"server": "ws://127.0.0.1:9/"})).unwrap_err().to_string();
    assert!(e.contains("desktop app"), "{e}");
}

#[test]
fn installing_needs_confirm_and_a_matching_download() {
    let (mut s, d) = session();
    offer(&d, ID, "1.0.0");
    s.execute("plugin.directory.list", &json!({"server": "ws://127.0.0.1:9/"})).unwrap();
    for confirm in [json!(null), json!(false), json!("true"), json!(1)] {
        let e = s.execute("plugin.directory.install", &json!({"id": ID, "confirm": confirm})).unwrap_err();
        assert!(matches!(e, EngineError::BadParams { .. }) && e.to_string().contains("confirm"), "{e}");
    }
    assert_eq!(fetches(&d), 0, "nothing is downloaded before the user confirms");
    // A damaged download, or one that isn't what was shown, is never installed.
    d.lock().unwrap().tamper = true;
    let e = s.execute("plugin.directory.install", &json!({"id": ID, "confirm": true})).unwrap_err().to_string();
    assert!(e.contains("SHA-256"), "{e}");
    d.lock().unwrap().tamper = false;
    let e = s.execute("plugin.directory.install", &json!({"id": ID, "confirm": true, "sha256": "00ff"})).unwrap_err().to_string();
    assert!(e.contains("shown"), "{e}");
    offer(&d, "org.example.other", "1.0.0");
    d.lock().unwrap().swap = Some("org.example.other".into());
    let e = s.execute("plugin.directory.install", &json!({"id": ID, "confirm": true})).unwrap_err().to_string();
    assert!(e.contains("another plugin"), "{e}");
    d.lock().unwrap().swap = None;
    // A bundle whose manifest disagrees with its entry.
    let b = bundle("org.example.liar", "9.9.9");
    d.lock().unwrap().plugins.push((entry(ID, "2.0.0", &b), b));
    let e = s.execute("plugin.directory.install", &json!({"id": ID, "version": "2.0.0", "confirm": true})).unwrap_err().to_string();
    assert!(e.contains("doesn't match"), "{e}");
    assert!(s.prefs.plugins.is_empty(), "nothing got installed");
    // The real thing: installed for the session (no install folder), and it runs.
    let r = s.execute("plugin.directory.install", &json!({"id": ID, "version": "1.0.0", "confirm": true})).unwrap();
    assert_eq!((r["version"].as_str(), r["path"].as_str(), r["replaced"].as_bool()), (Some("1.0.0"), None, Some(false)));
    assert_eq!(s.prefs.plugins[0].source.as_deref(), Some("ws://127.0.0.1:9/"));
    s.execute("plugin.run", &json!({"id": ID})).unwrap();
    let listed = s.execute("plugin.directory.list", &json!({})).unwrap();
    let mine = listed["plugins"].as_array().unwrap().iter().find(|p| p["version"] == "1.0.0").unwrap().clone();
    assert_eq!((mine["installed"].as_str(), mine["update"].as_bool()), (Some("1.0.0"), Some(false)));
}

#[test]
fn installed_plugins_live_in_their_folder_update_and_go() {
    let (mut s, d) = session();
    let root = temp("install");
    s.plugins.install_dir = Some(root.clone());
    offer(&d, ID, "1.0.0");
    let r = s.execute("plugin.directory.install", &json!({"id": ID, "server": "ws://127.0.0.1:9/", "confirm": true})).unwrap();
    let manifest = PathBuf::from(r["path"].as_str().unwrap());
    let folder = manifest.parent().unwrap().to_path_buf();
    assert_eq!(folder.parent().unwrap(), root.as_path());
    assert!(!folder.file_name().unwrap().to_string_lossy().contains("example"), "the folder is named by a hash, not the id");
    assert_eq!(std::fs::read(folder.join("module.wasm")).unwrap(), wasm());
    assert_eq!(std::fs::read(folder.join("assets").join("LICENSE")).unwrap(), b"MIT");
    assert!(std::fs::read_to_string(&manifest).unwrap().contains("module.wasm"));
    s.execute("plugin.run", &json!({"id": ID})).unwrap();
    // A newer version is an update; the same folder holds it.
    offer(&d, ID, "1.1.0");
    let listed = s.execute("plugin.directory.list", &json!({"server": "ws://127.0.0.1:9/"})).unwrap();
    assert!(listed["plugins"].as_array().unwrap().iter().any(|p| p["version"] == "1.1.0" && p["update"] == true));
    let r = s.execute("plugin.directory.install", &json!({"id": ID, "version": "1.1.0", "confirm": true})).unwrap();
    assert_eq!((r["replaced"].as_bool(), r["path"].as_str()), (Some(true), manifest.to_str()));
    assert_eq!(s.prefs.plugins.len(), 1);
    assert_eq!(s.prefs.plugins[0].manifest.version, "1.1.0");
    s.execute("plugin.run", &json!({"id": ID})).unwrap();
    // Removing deletes its folder (and only its folder).
    s.execute("plugin.remove", &json!({"id": ID})).unwrap();
    assert!(!folder.exists() && root.exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn publishing_sends_a_checked_folder() {
    let (mut s, d) = session();
    let dir = temp("publish");
    std::fs::create_dir_all(dir.join("build")).unwrap();
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::write(dir.join("build").join("tiny.wasm"), wasm()).unwrap();
    let text = json!({"id": ID, "name": "Tiny", "version": "1.0.0", "main": "build/tiny.wasm"}).to_string();
    std::fs::write(dir.join("figcraft-plugin.json"), &text).unwrap();
    std::fs::write(dir.join("assets").join("icon.png"), [1, 2, 3]).unwrap();
    let e = s.execute("plugin.directory.publish", &json!({"path": dir.to_string_lossy()})).unwrap_err().to_string();
    assert!(e.contains("server"), "{e}");
    let r = s.execute("plugin.directory.publish", &json!({"path": dir.to_string_lossy(), "server": "ws://127.0.0.1:9/?key=k"})).unwrap();
    assert_eq!((r["published"]["version"].as_str(), r["server"].as_str()), (Some("1.0.0"), Some("ws://127.0.0.1:9/")));
    let sent = d.lock().unwrap().plugins.last().unwrap().1.clone();
    let b = Bundle::decode(&sent, usize::MAX).unwrap();
    assert_eq!((b.manifest.as_str(), b.wasm.clone()), (text.as_str(), wasm()));
    assert_eq!(b.assets, vec![Asset { name: "icon.png".into(), data: vec![1, 2, 3] }]);
    // Refused before anything is sent: a hidden asset, a version a directory can't compare.
    let count = d.lock().unwrap().requests.len();
    std::fs::write(dir.join("assets").join(".secret"), b"x").unwrap();
    assert!(
        s.execute("plugin.directory.publish", &json!({"path": dir.to_string_lossy(), "server": "ws://h:1/"}))
            .unwrap_err()
            .to_string()
            .contains("asset")
    );
    std::fs::remove_file(dir.join("assets").join(".secret")).unwrap();
    std::fs::write(dir.join("figcraft-plugin.json"), text.replace("1.0.0", "next")).unwrap();
    assert!(
        s.execute("plugin.directory.publish", &json!({"path": dir.to_string_lossy(), "server": "ws://h:1/"}))
            .unwrap_err()
            .to_string()
            .contains("version")
    );
    assert_eq!(d.lock().unwrap().requests.len(), count);
    // Unpublishing names it.
    let r = s.execute("plugin.directory.unpublish", &json!({"id": ID, "server": "ws://127.0.0.1:9/?key=k"})).unwrap();
    assert_eq!(r["id"], ID);
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn publishing_refuses_a_linked_asset_instead_of_bundling_what_it_points_at() {
    let (mut s, d) = session();
    let dir = temp("publish-link");
    std::fs::create_dir_all(dir.join("build")).unwrap();
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::write(dir.join("build").join("tiny.wasm"), wasm()).unwrap();
    std::fs::write(dir.join("figcraft-plugin.json"), json!({"id": ID, "name": "Tiny", "version": "1.0.0", "main": "build/tiny.wasm"}).to_string())
        .unwrap();
    // A private file outside the plugin folder, reached through a link in its assets.
    let secret = temp("publish-link-secret").join("token.txt");
    std::fs::write(&secret, b"per-user-secret").unwrap();
    std::os::unix::fs::symlink(&secret, dir.join("assets").join("notes.txt")).unwrap();
    let e = s.execute("plugin.directory.publish", &json!({"path": dir.to_string_lossy(), "server": "ws://h:1/"})).unwrap_err().to_string();
    assert!(e.contains("link"), "{e}");
    assert!(d.lock().unwrap().plugins.is_empty(), "nothing was sent");
    // The whole assets folder as a link is refused the same way.
    std::fs::remove_file(dir.join("assets").join("notes.txt")).unwrap();
    std::fs::remove_dir(dir.join("assets")).unwrap();
    std::os::unix::fs::symlink(secret.parent().unwrap(), dir.join("assets")).unwrap();
    let e = s.execute("plugin.directory.publish", &json!({"path": dir.to_string_lossy(), "server": "ws://h:1/"})).unwrap_err().to_string();
    assert!(e.contains("link"), "{e}");
}

#[test]
fn a_bundle_asking_for_more_than_the_directory_showed_is_not_installed() {
    let (mut s, d) = session();
    // Shown as read-only with one command; the bundle's manifest asks to write, any command.
    let text =
        json!({"id": ID, "name": "Tiny", "version": "1.0.0", "author": "Ada", "main": "tiny.wasm", "permissions": {"access": "write"}}).to_string();
    let b = Bundle { manifest: text, wasm: wasm(), assets: vec![] }.encode();
    let mut e = entry(ID, "1.0.0", &b);
    e.commands = vec!["node.get".into()];
    d.lock().unwrap().plugins.push((e, b));
    s.execute("plugin.directory.list", &json!({"server": "ws://127.0.0.1:9/"})).unwrap();
    let err = s.execute("plugin.directory.install", &json!({"id": ID, "confirm": true})).unwrap_err().to_string();
    assert!(err.contains("write"), "{err}");
    assert!(s.prefs.plugins.is_empty(), "nothing was installed");
    // Read-only as shown, but more commands than shown.
    let text = json!({"id": ID, "name": "Tiny", "version": "1.0.1", "author": "Ada", "main": "tiny.wasm", "permissions": {"access": "read", "commands": ["node.get", "node.set"]}}).to_string();
    let b = Bundle { manifest: text, wasm: wasm(), assets: vec![] }.encode();
    let mut e = entry(ID, "1.0.1", &b);
    e.commands = vec!["node.get".into()];
    d.lock().unwrap().plugins.clear();
    d.lock().unwrap().plugins.push((e, b));
    s.execute("plugin.directory.list", &json!({"server": "ws://127.0.0.1:9/"})).unwrap();
    let err = s.execute("plugin.directory.install", &json!({"id": ID, "confirm": true})).unwrap_err().to_string();
    assert!(err.contains("commands"), "{err}");
}
