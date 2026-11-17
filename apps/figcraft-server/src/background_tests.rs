//! The app's way of talking to a server without waiting (`Session::start_background`), end to end
//! over loopback: browsing and installing from the plugin directory, publishing a team library,
//! adding it and asking for its usage each return at once and finish when the answers arrive; a
//! server that never answers doesn't hold the caller up.

use std::net::TcpListener;
use std::time::{Duration, Instant};

use craft_plugin::bundle::Bundle;
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::store::Store;
use crate::test_util::{TempDir, config, publish, pump, session};

const ID: &str = "org.example.tiny";

fn bundle() -> Vec<u8> {
    let wasm =
        wat::parse_str(r#"(module (memory (export "memory") 1) (func (export "figcraft_run") (param i32) (result i32) i32.const 0))"#).unwrap();
    let manifest = json!({"id": ID, "name": "Tiny tool", "version": "1.0.0", "main": "tiny.wasm", "permissions": {"access": "read"}});
    Bundle { manifest: manifest.to_string(), wasm, assets: Vec::new() }.encode()
}

/// Start `cmd` without waiting (it must not finish at once: its answer comes over the network),
/// then pump the session until it finished; its result.
fn background(s: &mut Session, cmd: &str, params: Value) -> Value {
    let started = Instant::now();
    assert_eq!(s.start_background(cmd, params).unwrap(), None, "{cmd} waits for its answer");
    assert!(started.elapsed() < Duration::from_secs(1), "{cmd} returned at once");
    pump(&mut [&mut *s], cmd, |s| !s[0].waiting_for(cmd));
    let (done, r) = s.take_background_done().unwrap();
    assert_eq!(done, cmd);
    r.unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn id(v: &Value) -> Value {
    v["id"].clone()
}

#[test]
fn browsing_and_installing_finish_in_the_background() {
    let dir = TempDir::new("background-plugins");
    Store::open(dir.path(), config(dir.path()).store()).unwrap().directory().publish(&bundle()).unwrap();
    let mut server = crate::start(&config(dir.path())).unwrap();
    let plain = format!("ws://{}/", server.addr());
    let mut s = session("Bo");
    let listed = background(&mut s, "plugin.directory.list", json!({"server": plain}));
    assert_eq!(listed["plugins"][0]["id"], ID);
    assert_eq!(s.plugins.directory.as_ref().map(|l| l.entries.len()), Some(1), "Browse shows the listing");
    let sha = listed["plugins"][0]["sha256"].clone();
    let r = background(&mut s, "plugin.directory.install", json!({"id": ID, "confirm": true, "sha256": sha, "server": plain}));
    assert_eq!(r["installed"], ID);
    assert_eq!(s.prefs.plugins.len(), 1);
    assert!(s.plugins.unsaved, "the app saves the plugin list at once");
    // Scripts still wait for their answer.
    assert_eq!(s.execute("plugin.directory.list", &json!({})).unwrap()["plugins"][0]["installed"], "1.0.0");
    server.stop();
}

#[test]
fn team_library_requests_finish_in_the_background() {
    let dir = TempDir::new("background-library");
    let mut server = crate::start(&config(dir.path())).unwrap();
    let addr = server.addr();
    let mut a = session("Ada");
    let f = a.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 40, "name": "Button"})).unwrap();
    a.execute("component.create", &json!({"ids": [id(&f)]})).unwrap();
    let p = publish(&mut a, dir.path(), addr);
    // Fetching the server's current library, then publishing: two answers.
    assert_eq!(background(&mut a, "library.publishToServer", json!({}))["version"], 1);
    let mut b = session("Bo");
    let added = background(&mut b, "library.add", json!({"path": p.view}));
    assert_eq!(added["components"].as_array().map(Vec::len), Some(1));
    assert_eq!(b.doc().unwrap().libraries.enabled.len(), 1);
    let usage = background(&mut a, "library.usage", json!({}));
    assert!(usage["files"].is_array(), "{usage}");
    server.stop();
}

#[test]
fn a_server_that_never_answers_holds_nothing_up() {
    // It never accepts: connections wait in its backlog, and no answer ever comes.
    let hole = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = format!("ws://{}/", hole.local_addr().unwrap());
    let mut s = session("Cy");
    let started = Instant::now();
    assert_eq!(s.start_background("plugin.directory.list", json!({"server": server})).unwrap(), None);
    assert!(started.elapsed() < Duration::from_secs(1), "the caller didn't wait");
    assert!(s.waiting_for("plugin.directory.list"));
    // Closing the window stops waiting; the answer, when the request gives up, is dropped.
    s.cancel_background("plugin.directory.list");
    assert!(!s.waiting_for("plugin.directory.list") && s.take_background_done().is_none());
    drop(hole);
}
