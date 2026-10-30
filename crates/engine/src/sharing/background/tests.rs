//! Commands run without waiting on a server, against a scripted link that holds each background
//! request until the test answers it: nothing is waited for, scripts still wait, answers resume the
//! command (twice for a command with two requests), a cancelled command ignores its answer, errors
//! are its outcome, a re-run acts on the file it started on, and a link that can't send in the
//! background is waited on as before.

use std::sync::{Arc, Mutex};

use figcraft_collab::message::{ClientMsg, LibraryReply, LibraryReq, PluginEntry, PluginReply, PluginReq};
use figcraft_plugin::bundle::{Bundle, sha256_hex};
use serde_json::json;

use crate::library_server::{archive, read_archive};
use crate::live::{Event, HostOptions, LiveLink, NetReply, NetRequest, Presence, SessionInfo};
use crate::{EngineError, Session};

const ID: &str = "org.example.tiny";
const SERVER: &str = "ws://127.0.0.1:9/";
const LINK: &str = "ws://127.0.0.1:9/?file=f&token=t";

/// What the scripted link saw: background requests out, answers to hand back, waited-for calls.
#[derive(Default)]
struct Wire {
    background: bool,
    out: Vec<(u64, NetRequest)>,
    answers: Vec<Event>,
    waited: usize,
    /// The team library the scripted server keeps (an archive), if any.
    library: Option<Vec<u8>>,
}

struct Link(Arc<Mutex<Wire>>);

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
        std::mem::take(&mut self.0.lock().unwrap().answers).into_iter().map(|e| (0, e)).collect()
    }
    fn plugins(&mut self, _: &str, req: PluginReq) -> Result<PluginReply, String> {
        self.0.lock().unwrap().waited += 1;
        Ok(directory(req))
    }
    fn library(&mut self, _: &str, req: LibraryReq) -> Result<LibraryReply, String> {
        let mut w = self.0.lock().unwrap();
        w.waited += 1;
        Ok(library(&mut w, req))
    }
    fn background(&mut self, job: u64, req: NetRequest) -> bool {
        let mut w = self.0.lock().unwrap();
        if w.background {
            w.out.push((job, req));
        }
        w.background
    }
}

fn wasm() -> Vec<u8> {
    wat::parse_str(r#"(module (memory (export "memory") 1) (func (export "figcraft_run") (param i32) (result i32) i32.const 0))"#).unwrap()
}

fn bundle() -> Vec<u8> {
    let manifest = json!({"id": ID, "name": "Tiny tool", "version": "1.0.0", "main": "t.wasm", "permissions": {"access": "write", "commands": ["shape.rectangle"]}});
    Bundle { manifest: manifest.to_string(), wasm: wasm(), assets: Vec::new() }.encode()
}

fn directory(req: PluginReq) -> PluginReply {
    let bytes = bundle();
    let entry =
        PluginEntry { id: ID.into(), name: "Tiny tool".into(), version: "1.0.0".into(), sha256: sha256_hex(&bytes), ..PluginEntry::default() };
    match req {
        PluginReq::List { .. } => PluginReply::List(vec![entry]),
        PluginReq::Fetch { .. } => PluginReply::Bundle { entry, bundle: bytes },
        _ => PluginReply::Failed { reason: "no".into() },
    }
}

fn library(w: &mut Wire, req: LibraryReq) -> LibraryReply {
    match (req, &w.library) {
        (LibraryReq::Fetch { .. }, Some(a)) => LibraryReply::Library { version: 1, archive: a.clone() },
        (LibraryReq::Fetch { .. }, None) => LibraryReply::Empty,
        (LibraryReq::Publish { archive }, _) => {
            w.library = Some(archive);
            LibraryReply::Published { version: 1 }
        }
        (LibraryReq::Usage, _) => LibraryReply::Usage(Vec::new()),
        (LibraryReq::ReportDetached(_) | LibraryReq::ReportInserted(_), _) => LibraryReply::Noted,
        (LibraryReq::Detached, _) => LibraryReply::Detached(Vec::new()),
        (LibraryReq::Actions, _) => LibraryReply::Actions(Vec::new()),
    }
}

fn session(background: bool) -> (Session, Arc<Mutex<Wire>>) {
    let wire = Arc::new(Mutex::new(Wire { background, ..Wire::default() }));
    let mut s = Session::new();
    s.live.install(Box::new(Link(wire.clone())));
    (s, wire)
}

/// Answer the requests out (as the scripted server would) and let the session take the answers;
/// how many there were.
fn answer(s: &mut Session, wire: &Arc<Mutex<Wire>>) -> usize {
    let out = std::mem::take(&mut wire.lock().unwrap().out);
    for (job, req) in &out {
        let mut w = wire.lock().unwrap();
        let reply = match req.clone() {
            NetRequest::Plugins { req, .. } => NetReply::Plugins(Ok(directory(req))),
            NetRequest::Library { req, .. } => NetReply::Library(Ok(library(&mut w, req))),
            NetRequest::Upload(_) => NetReply::Upload(Err("no".into())),
            NetRequest::Hosting { .. } => NetReply::Hosting(Err("no".into())),
            NetRequest::Share { .. } => NetReply::Share(Err("no".into())),
        };
        w.answers.push(Event::Net { job: *job, reply: Box::new(reply) });
    }
    s.live_poll();
    out.len()
}

/// A library file's published library as the archive a server keeps.
fn published() -> Vec<u8> {
    let mut s = Session::new();
    let f = s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 10, "height": 10, "name": "Chip"})).unwrap();
    s.execute("component.create", &json!({"ids": [f["id"]]})).unwrap();
    s.execute("library.publish", &json!({})).unwrap();
    archive(s.doc().unwrap().published.as_ref().unwrap()).unwrap()
}

#[test]
fn a_continue_batch_waits_instead_of_skipping_background_work() {
    for nested in [false, true] {
        let (mut s, wire) = session(true);
        let before = s.doc().unwrap().clone();
        let request = json!({"command": "plugin.directory.list", "params": {"server": SERVER}});
        let step = if nested { json!({"command": "command.batch", "params": {"onError": "continue", "commands": [request]}}) } else { request };
        let params = json!({"onError": "continue", "commands": [
            {"command": "page.rename", "params": {"name": "Before request"}},
            step,
            {"command": "page.rename", "params": {"name": "After request"}}
        ]});
        assert_eq!(s.start_background("command.batch", params).unwrap(), None);
        assert!(s.waiting_for("command.batch"));
        assert_eq!(s.doc().unwrap(), &before, "waiting rolls back the whole batch");
        assert_eq!(s.undo_depth(), (0, 0));
        assert_eq!(answer(&mut s, &wire), 1);
        let (command, result) = s.take_background_done().unwrap();
        assert_eq!(command, "command.batch");
        let result = result.unwrap();
        assert!(result.as_array().unwrap().iter().all(|r| r["ok"] == true));
        let directory = if nested { &result[1]["result"][0]["result"] } else { &result[1]["result"] };
        assert_eq!(directory["plugins"][0]["id"], ID);
        assert_eq!(s.doc().unwrap().node(s.page().unwrap()).unwrap().name, "After request");
        assert_eq!(s.undo_depth(), (1, 0));
        assert!(!s.waiting_for("command.batch"));
    }
}

#[test]
fn browsing_and_installing_wait_for_nothing_and_scripts_still_wait() {
    let (mut s, wire) = session(true);
    assert_eq!(s.start_background("plugin.directory.list", json!({"server": SERVER})).unwrap(), None);
    assert!(s.waiting_for("plugin.directory.list") && s.plugins.directory.is_none());
    assert_eq!(wire.lock().unwrap().waited, 0, "nothing was waited for");
    // A script (the MCP server, the control channel) still gets its answer at once.
    let r = s.execute("plugin.directory.list", &json!({"server": SERVER})).unwrap();
    assert_eq!(r["plugins"][0]["id"], ID);
    assert_eq!(wire.lock().unwrap().waited, 1);
    assert!(s.take_background_done().is_none(), "no answer yet");
    assert_eq!(answer(&mut s, &wire), 1);
    let (cmd, r) = s.take_background_done().unwrap();
    assert_eq!((cmd.as_str(), r.unwrap()["plugins"][0]["id"].as_str()), ("plugin.directory.list", Some(ID)));
    assert!(!s.waiting_for("plugin.directory.list") && s.take_background_done().is_none());

    let install = json!({"id": ID, "confirm": true, "server": SERVER});
    assert_eq!(s.start_background("plugin.directory.install", install).unwrap(), None);
    assert!(s.prefs.plugins.is_empty() && !s.plugins.unsaved);
    answer(&mut s, &wire);
    let (_, r) = s.take_background_done().unwrap();
    assert_eq!(r.unwrap()["installed"], ID);
    assert_eq!(s.prefs.plugins.len(), 1);
    assert!(s.plugins.unsaved, "the app is told to save the plugin list at once");
    assert_eq!(wire.lock().unwrap().waited, 1, "the background path never waited");
}

#[test]
fn cancelled_commands_drop_their_answer_and_errors_are_the_outcome() {
    let (mut s, wire) = session(true);
    s.start_background("plugin.directory.list", json!({"server": SERVER})).unwrap();
    s.cancel_background("plugin.directory.list");
    assert!(!s.waiting_for("plugin.directory.list"));
    answer(&mut s, &wire);
    assert!(s.take_background_done().is_none() && s.plugins.directory.is_none(), "the late answer is ignored");

    // Starting it again replaces the one waiting: only the newer answer counts.
    s.start_background("plugin.directory.list", json!({"server": SERVER})).unwrap();
    s.start_background("plugin.directory.list", json!({"server": SERVER})).unwrap();
    assert_eq!(answer(&mut s, &wire), 2);
    assert!(s.take_background_done().is_some() && s.take_background_done().is_none());

    s.start_background("plugin.directory.list", json!({"server": SERVER})).unwrap();
    let (job, _) = wire.lock().unwrap().out.pop().unwrap();
    wire.lock().unwrap().answers.push(Event::Net { job, reply: Box::new(NetReply::Plugins(Err("the server can't be reached".into()))) });
    s.live_poll();
    let (_, r) = s.take_background_done().unwrap();
    assert!(r.unwrap_err().to_string().contains("can't be reached"));
    // Bad parameters fail at once, without a request.
    assert!(s.start_background("plugin.directory.install", json!({"id": ID})).is_err());
    assert!(wire.lock().unwrap().out.is_empty());
}

#[test]
fn a_command_with_two_requests_resumes_twice_and_keeps_what_it_sent() {
    let (mut s, wire) = session(true);
    let f = s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 10, "height": 10, "name": "Chip"})).unwrap();
    s.execute("component.create", &json!({"ids": [f["id"]]})).unwrap();
    // The server's current library first, then the publish.
    assert_eq!(s.start_background("library.publishToServer", json!({"server": LINK})).unwrap(), None);
    assert_eq!(answer(&mut s, &wire), 1);
    assert!(s.take_background_done().is_none() && s.waiting_for("library.publishToServer"));
    assert!(matches!(&wire.lock().unwrap().out[..], [(_, NetRequest::Library { req: LibraryReq::Publish { .. }, .. })]));
    assert_eq!(answer(&mut s, &wire), 1);
    let (_, r) = s.take_background_done().unwrap();
    assert_eq!(r.unwrap()["version"], 1);
    let kept = read_archive(wire.lock().unwrap().library.as_ref().unwrap()).unwrap();
    let cached = s.libraries.cached(LINK).unwrap().unwrap();
    assert_eq!(cached.manifest, kept.manifest, "the library kept here is the one the server got");
    assert_eq!(wire.lock().unwrap().waited, 0);
}

#[test]
fn a_rerun_acts_on_the_file_it_started_on() {
    let (mut s, wire) = session(true);
    wire.lock().unwrap().library = Some(published());
    let started = s.ds().unwrap().uid;
    assert_eq!(s.start_background("library.add", json!({"path": LINK})).unwrap(), None);
    s.execute("file.new", &json!({})).unwrap();
    let other = s.ds().unwrap().uid;
    answer(&mut s, &wire);
    let (_, r) = s.take_background_done().unwrap();
    assert_eq!(r.unwrap()["name"], "Untitled");
    assert_eq!(s.ds().unwrap().uid, other, "the file shown stays shown");
    let enabled = |s: &Session, uid: u64| s.docs.iter().find(|d| d.uid == uid).unwrap().doc.libraries.enabled.len();
    assert_eq!((enabled(&s, started), enabled(&s, other)), (1, 0), "the library went to the file it was added in");

    // A file closed meanwhile leaves nothing to add the library to.
    s.libraries = crate::library::Cache::default();
    s.execute("file.new", &json!({})).unwrap();
    s.start_background("library.add", json!({"path": LINK})).unwrap();
    s.execute("file.close", &json!({})).unwrap();
    let shown = s.ds().unwrap().uid;
    answer(&mut s, &wire);
    let (_, r) = s.take_background_done().unwrap();
    assert!(matches!(r, Err(EngineError::NoDocument)), "{r:?}");
    assert_eq!(s.ds().unwrap().uid, shown);
}

#[test]
fn a_link_without_background_requests_is_waited_on() {
    let (mut s, wire) = session(false);
    let r = s.start_background("plugin.directory.list", json!({"server": SERVER})).unwrap().unwrap();
    assert_eq!(r["plugins"][0]["id"], ID);
    assert!(!s.waiting_for("plugin.directory.list"));
    assert_eq!(wire.lock().unwrap().waited, 1);
    // Without any link, the usual error.
    let mut bare = Session::new();
    assert!(bare.start_background("plugin.directory.list", json!({"server": SERVER})).unwrap_err().to_string().contains("desktop app"));
}

/// Confined to automation roots, commands reach no server (#1095): browsing a plugin directory,
/// hosting, joining and publishing are refused before the link is asked. Unconfined, the same
/// request reaches the server as before.
#[test]
fn commands_confined_to_automation_roots_reach_no_server() {
    let dir = std::env::temp_dir().join(format!("figcraft-roots-network-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let roots = Arc::new(crate::file_access::Roots::new(&[], &[dir.to_string_lossy().into_owned()]).unwrap());
    let (mut s, wire) = session(false);
    let refused = |r: Result<serde_json::Value, EngineError>| {
        let e = r.unwrap_err().to_string();
        assert!(e.contains(crate::file_access::NETWORK_REFUSED), "{e}");
    };
    crate::file_access::confined(Some(&roots), || {
        refused(s.execute("plugin.directory.list", &json!({"server": SERVER})));
        refused(s.execute("collab.host", &json!({})));
        refused(s.execute("collab.join", &json!({"link": LINK})));
        refused(s.execute("collab.publish", &json!({"server": SERVER})));
    });
    assert_eq!(wire.lock().unwrap().waited, 0, "the server was never asked");
    assert!(s.live.iter().next().is_none(), "no session was started");
    let _ = std::fs::remove_dir_all(&dir);

    let r = s.execute("plugin.directory.list", &json!({"server": SERVER})).unwrap();
    assert_eq!(r["plugins"][0]["id"], ID);
    assert_eq!(wire.lock().unwrap().waited, 1);
    let e = s.execute("collab.host", &json!({})).unwrap_err().to_string();
    assert!(!e.contains(crate::file_access::NETWORK_REFUSED), "unconfined, the link is asked: {e}");
}

/// A team library fetched before stays cached, but confined to automation roots (#1095) checking
/// it for updates is refused, and `library.updates` and `library.list` say so instead of
/// answering from the cache as if the server had nothing newer.
#[test]
fn a_confined_update_check_reports_the_refusal_not_the_cached_library() {
    let (mut s, wire) = session(false);
    wire.lock().unwrap().library = Some(published());
    s.execute("library.add", &json!({"path": LINK})).unwrap();
    assert!(matches!(s.libraries.cached(LINK), Some(Ok(_))));
    let waited = wire.lock().unwrap().waited;
    let dir = std::env::temp_dir().join(format!("figcraft-roots-updates-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let roots = Arc::new(crate::file_access::Roots::new(&[], &[dir.to_string_lossy().into_owned()]).unwrap());
    crate::file_access::confined(Some(&roots), || {
        let u = s.execute("library.updates", &json!({})).unwrap();
        assert_eq!(u["count"], 0);
        let e = u["errors"][0]["error"].as_str().unwrap();
        assert!(e.contains(crate::file_access::NETWORK_REFUSED), "{u}");
        let l = s.execute("library.list", &json!({})).unwrap();
        let e = l["libraries"][0]["error"].as_str().unwrap();
        assert!(e.contains(crate::file_access::NETWORK_REFUSED), "{l}");
    });
    assert_eq!(wire.lock().unwrap().waited, waited, "the server was never asked");
    assert!(matches!(s.libraries.cached(LINK), Some(Ok(_))), "the cached library is kept");
    let _ = std::fs::remove_dir_all(&dir);

    // Unconfined, the same check reaches the server and finds nothing new.
    let u = s.execute("library.updates", &json!({})).unwrap();
    assert_eq!((u["count"].clone(), u["errors"].as_array().map(Vec::len)), (json!(0), Some(0)), "{u}");
    assert_eq!(wire.lock().unwrap().waited, waited + 1);
}
