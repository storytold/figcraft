//! Sessions you joined are remembered without their links showing, and can be joined again.

use std::sync::{Arc, Mutex};

use serde_json::json;

use crate::Session;
use crate::joined::{MAX_JOINED_FILES, preview_key};
use crate::live::{Event, Role};
use crate::live_tests::{Mock, Script, info};
use figcraft_collab::{Client, ClientMsg, Limits, Outgoing, Server, ServerMsg};
use figcraft_doc::Document;

const LINK: &str = "figcraft://join?host=127.0.0.1:47900&file=f&token=t";

fn session() -> (Session, Arc<Mutex<Script>>) {
    let mut s = Session::new();
    let script = Arc::new(Mutex::new(Script::default()));
    s.live.install(Box::new(Mock(script.clone())));
    (s, script)
}

/// Joined the session of a file called `name`.
fn join(s: &mut Session, script: &Arc<Mutex<Script>>, name: &str) {
    s.execute("collab.join", &json!({"link": LINK})).unwrap();
    let mut doc = Document::new(name);
    doc.repair();
    let mut server = Server::new(doc, "t", Limits::default());
    let ClientMsg::Hello(h) = Client::hello("f", "t", "Me", "") else { panic!("not a hello") };
    let (id, out) = server.join(h).unwrap();
    let w = out.into_iter().find_map(|o| if let Outgoing::To(_, w @ ServerMsg::Welcome { .. }) = o { Some(w) } else { None }).unwrap();
    script.lock().unwrap().events.extend([Event::Welcome { info: info(Role::Guest, id), peers: vec![] }, Event::Sync(w)]);
    s.live_poll();
}

#[test]
fn a_session_you_joined_is_remembered_without_its_link_showing() {
    let (mut s, script) = session();
    join(&mut s, &script, "Team file");
    assert!(s.live.unsaved, "the app saves the preferences");
    let f = s.prefs.joined_files[0].clone();
    assert_eq!((f.address.as_str(), f.file.as_str(), f.name.as_str(), f.access.as_str()), ("127.0.0.1:47900", "f", "Team file", "edit"));
    assert_eq!(f.link, LINK);
    let listed = s.execute("collab.joinedFiles", &json!({})).unwrap();
    assert_eq!(listed["files"][0]["name"], "Team file");
    assert!(!listed.to_string().contains("token"), "{listed}");
    assert!(!format!("{f:?}").contains("token"));
    // Joined again: one record, and it moves to the front.
    join(&mut s, &script, "Renamed");
    assert_eq!(s.prefs.joined_files.len(), 1);
    assert_eq!(s.prefs.joined_files[0].name, "Renamed");
}

#[test]
fn a_remembered_session_joins_with_its_link_or_comes_to_the_front() {
    let (mut s, script) = session();
    join(&mut s, &script, "Team file");
    let tab = s.active;
    s.execute("file.new", &json!({})).unwrap();
    let r = s.execute("collab.openJoined", &json!({"file": "f"})).unwrap();
    assert_eq!(r["alreadyOpen"], true);
    assert_eq!(s.active, tab);
    s.execute("collab.leave", &json!({})).unwrap();
    let joins = script.lock().unwrap().joined.len();
    let r = s.execute("collab.openJoined", &json!({"file": "f"})).unwrap();
    assert_eq!(r["status"], "connecting");
    assert_eq!(script.lock().unwrap().joined[joins], LINK);
    assert!(s.execute("collab.openJoined", &json!({"file": "other"})).is_err());
}

#[test]
fn forgetting_takes_it_off_the_list() {
    let (mut s, script) = session();
    join(&mut s, &script, "Team file");
    s.live.unsaved = false;
    s.execute("collab.forgetJoined", &json!({"file": "f"})).unwrap();
    assert!(s.prefs.joined_files.is_empty() && s.live.unsaved);
    assert!(s.execute("collab.forgetJoined", &json!({"file": "f"})).is_err());
}

#[test]
fn junk_in_the_preferences_costs_nothing_else() {
    let many: Vec<_> = (0..300).map(|i| json!({"address": "a:1", "file": format!("f{i}"), "name": "n", "link": "figcraft://x"})).collect();
    let prefs: crate::prefs::Prefs = serde_json::from_value(json!({
        "joined_files": [{"address": "", "file": "x", "link": "l"}, "junk", 7, {"address": "a:1", "file": "dup", "link": "l"}, {"address": "a:1", "file": "dup", "link": "l"}],
    }))
    .unwrap();
    assert_eq!(prefs.joined_files.len(), 1, "no place, not a record, a repeat");
    let prefs: crate::prefs::Prefs = serde_json::from_value(json!({"joined_files": many})).unwrap();
    assert_eq!(prefs.joined_files.len(), MAX_JOINED_FILES);
    let prefs: crate::prefs::Prefs = serde_json::from_value(json!({"joined_files": "junk"})).unwrap();
    assert!(prefs.joined_files.is_empty());
}

#[test]
fn a_preview_key_is_stable_and_safe_in_a_path() {
    assert_eq!(preview_key("a:1", "f"), preview_key("a:1", "f"));
    assert_ne!(preview_key("a:1", "f"), preview_key("a:2", "f"));
    assert_ne!(preview_key("a", "b/c"), preview_key("a/b", "c"));
    assert!(preview_key("../../etc", "/x").chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn a_status_check_that_cant_be_made_still_lists_the_sessions() {
    let (mut s, script) = session();
    join(&mut s, &script, "Team file");
    // This link has no way to ask: nothing is known about the server, and the list still comes.
    let listed = s.execute("collab.joinedFiles", &json!({"refresh": true})).unwrap();
    assert_eq!(listed["files"][0]["name"], "Team file");
    assert!(listed["files"][0]["reach"].is_null());
}
