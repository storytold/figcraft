//! Opening a file that names a team library on some server doesn't contact that server (#1097):
//! only team libraries you added or checked are asked for updates by themselves.

use std::sync::{Arc, Mutex};

use figcraft_collab::message::{ClientMsg, LibraryReply, LibraryReq};
use figcraft_doc::{Document, LibraryLink};
use serde_json::json;

use crate::live::{Event, HostOptions, LiveLink, NetRequest, Presence, SessionInfo};
use crate::{Prefs, Session};

const LINK: &str = "ws://tracker.example:9/?file=f&token=per-file-secret";

/// Every request the app would send anywhere.
#[derive(Default)]
struct Wire {
    sent: Vec<String>,
}

struct Link(Arc<Mutex<Wire>>);

impl Link {
    fn note(&self, what: &str) {
        self.0.lock().unwrap().sent.push(what.into());
    }
}

impl LiveLink for Link {
    fn host(&mut self, _: u64, _: HostOptions) -> Result<SessionInfo, String> {
        self.note("host");
        Err("no".into())
    }
    fn join(&mut self, _: u64, _: &str, _: &str) -> Result<(), String> {
        self.note("join");
        Err("no".into())
    }
    fn leave(&mut self, _: u64) {}
    fn presence(&mut self, _: u64, _: &Presence) {}
    fn send(&mut self, _: u64, _: ClientMsg) {
        self.note("send");
    }
    fn poll(&mut self) -> Vec<(u64, Event)> {
        Vec::new()
    }
    fn library(&mut self, link: &str, _: LibraryReq) -> Result<LibraryReply, String> {
        self.note(link);
        Ok(LibraryReply::Empty)
    }
    fn library_background(&mut self, jobs: Vec<(String, LibraryReq)>) {
        for (link, _) in jobs {
            self.note(&link);
        }
    }
    fn background(&mut self, _: u64, _: NetRequest) -> bool {
        self.note("background");
        false
    }
}

/// A session with `prefs` that opened a file (from disk) whose libraries include [`LINK`].
fn opened(prefs: Prefs, tag: &str) -> (Session, Arc<Mutex<Wire>>, std::path::PathBuf) {
    let mut d = Document::new("Someone else's file");
    d.libraries_mut().enabled.push(LibraryLink { id: "lib".into(), name: "Kit".into(), path: LINK.into(), version: 3 });
    let path = std::env::temp_dir().join(format!("figcraft-beacon-{}-{tag}.figcraft", std::process::id()));
    std::fs::write(&path, figcraft_format::save_bytes(&d, None).unwrap()).unwrap();
    let mut s = Session::new();
    s.prefs = prefs;
    let wire = Arc::new(Mutex::new(Wire::default()));
    s.live.install(Box::new(Link(wire.clone())));
    s.execute("file.open", &json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!(s.doc().unwrap().libraries.enabled.len(), 1);
    (s, wire, path)
}

fn sent(w: &Arc<Mutex<Wire>>) -> Vec<String> {
    w.lock().unwrap().sent.clone()
}

#[test]
fn opening_a_file_naming_an_unknown_team_library_contacts_nothing() {
    let (mut s, wire, path) = opened(Prefs::default(), "unknown");
    // What the Assets panel does now and then while the file is shown.
    for _ in 0..3 {
        s.refresh_server_libraries();
    }
    assert!(sent(&wire).is_empty(), "no connection attempt: {:?}", sent(&wire));
    assert!(!s.library_allowed(LINK));
    // You check it: now it is asked, and from then on by itself.
    assert_eq!(s.execute("library.check", &json!({})).unwrap()["checking"], 1);
    assert_eq!(sent(&wire), vec![LINK.to_string()]);
    assert!(s.library_allowed(LINK) && s.live.unsaved, "kept with the preferences");
    s.refresh_server_libraries();
    assert_eq!(sent(&wire).len(), 2);
    // Kept as a hash: the preferences hold no token.
    assert!(!serde_json::to_string(&s.prefs).unwrap().contains("per-file-secret"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_team_library_you_checked_before_is_checked_when_a_file_names_it() {
    let mut mine = Session::new();
    mine.allow_library(LINK);
    let (mut s, wire, path) = opened(mine.prefs.clone(), "checked");
    s.refresh_server_libraries();
    assert_eq!(sent(&wire), vec![LINK.to_string()]);
    let _ = std::fs::remove_file(path);
}

#[test]
fn checking_one_team_library_by_name() {
    let (mut s, wire, path) = opened(Prefs::default(), "named");
    assert!(s.execute("library.check", &json!({"library": "Nope"})).is_err());
    assert_eq!(s.execute("library.check", &json!({"library": "Kit"})).unwrap()["checking"], 1);
    assert_eq!(sent(&wire), vec![LINK.to_string()]);
    let _ = std::fs::remove_file(path);
}
