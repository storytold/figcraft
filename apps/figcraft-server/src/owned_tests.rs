//! Your online files against a real server: a file shared online is recorded with its owner
//! token; after a restart of the app the server says what became of it (`Admin::Owned`, one
//! request for all the files), it opens again as its owner in step with everyone who joined by
//! code, its sharing stops and starts from the record, and removing it takes it off the server's
//! disk. The request itself: only a file's own token learns about it, junk never hurts, and an
//! owner's question opens nothing (a file isn't kept alive by being asked about).

use figcraft_collab::message::{Admin, AdminReply, ClientMsg, MAX_OWNED, OpenHosting, PROTOCOL, ServerMsg};
use figcraft_engine::Prefs;
use serde_json::{Value, json};

use crate::config::Config;
use crate::test_util::{Raw, TempDir, config, live, pump, same, session};

fn open(dir: &std::path::Path) -> Config {
    Config { open_publish: true, ..config(dir) }
}

fn ex(s: &mut figcraft_engine::Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

/// An open hosting connection that names no file.
fn hosting(addr: std::net::SocketAddr) -> Raw {
    let mut raw = Raw::connect(addr);
    raw.send(&ClientMsg::OpenHosting(OpenHosting { protocol: PROTOCOL, client_version: "test".into(), file: String::new(), owner: String::new() }));
    assert!(matches!(raw.next(), ServerMsg::Hosting(_)));
    raw
}

fn owned(raw: &mut Raw, files: Vec<(String, String)>) -> AdminReply {
    raw.send(&ClientMsg::Admin(Admin::Owned { files }));
    match raw.next() {
        ServerMsg::Admin(r) => *r,
        other => panic!("answered {other:?}"),
    }
}

/// Ada shares her open file online through the server at `url`: the app with the file live.
fn share(url: &str) -> figcraft_engine::Session {
    let mut a = session("Ada");
    ex(&mut a, "pref.publishServer", json!({"url": url}));
    ex(&mut a, "shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 30}));
    ex(&mut a, "collab.share", json!({}));
    pump(&mut [&mut a], "the share", |s| live(s[0]) && s[0].live().share.as_ref().is_some_and(|sh| sh.links.is_some()));
    a
}

/// The app quits (its sessions end with it); the next run has only the preferences.
fn restart(a: figcraft_engine::Session, name: &str) -> figcraft_engine::Session {
    let prefs: Prefs = serde_json::from_value(serde_json::to_value(&a.prefs).unwrap()).unwrap();
    drop(a);
    let mut next = session(name);
    next.prefs = prefs;
    next
}

#[test]
fn a_shared_file_is_its_owners_again_after_the_app_restarts() {
    let dir = TempDir::new("owned-restart");
    let mut server = crate::start(&open(dir.path())).unwrap();
    let url = format!("ws://{}/", server.addr());
    let a = share(&url);
    let code = a.live_summary()["code"].as_str().unwrap().to_string();
    let file = a.prefs.online_files[0].file.clone();
    let owner = a.prefs.online_files[0].owner.clone();
    assert_eq!(a.live().published.as_ref().and_then(|p| p.owner.clone()), Some(owner.clone()));
    // The app quits. The next run knows the file from its preferences alone.
    let mut a = restart(a, "Ada");
    let list = ex(&mut a, "collab.files", json!({}));
    assert_eq!((&list["files"][0]["file"], &list["files"][0]["state"], &list["files"][0]["code"]), (&json!(file), &json!("unknown"), &json!(code)));
    // The server says it is there, and still shared.
    let list = ex(&mut a, "collab.files", json!({"refresh": true}));
    assert_eq!((&list["files"][0]["state"], &list["files"][0]["shared"], &list["problems"]), (&json!("online"), &json!(true), &json!([])));
    assert!(list["files"][0]["expires"].as_u64().is_some_and(|e| e > crate::store::now()));
    assert!(!list.to_string().contains(&owner));
    // Open it: the owner is in the file, synced, in a tab of its own.
    let opened = ex(&mut a, "collab.openFile", json!({"file": file}));
    assert!(!opened.to_string().contains(&owner));
    pump(&mut [&mut a], "the owner back in", |s| live(s[0]));
    assert_eq!(a.docs.len(), 2);
    assert!(!a.live_summary().to_string().contains(&owner), "the token stays in the record");
    // Someone joins with the code given out before the restart, and sees the owner's edits.
    let mut b = session("Bo");
    ex(&mut b, "pref.publishServer", json!({"url": url}));
    ex(&mut b, "collab.join", json!({"code": code}));
    pump(&mut [&mut a, &mut b], "Bo in", |s| live(s[1]) && s[0].live().peers.len() == 1);
    ex(&mut a, "shape.ellipse", json!({"x": 100, "y": 100, "width": 20, "height": 20}));
    pump(&mut [&mut a, &mut b], "the edit at Bo's", |s| same(s[0].doc().unwrap(), s[1].doc().unwrap()));
    // Stop sharing from the list (no tab needed): Bo is out, the owner stays, the code is dead.
    assert_eq!(ex(&mut a, "collab.manageFile", json!({"file": file, "action": "unshare"}))["shared"], false);
    pump(&mut [&mut a, &mut b], "Bo out", |s| !s[1].live().status.active() && live(s[0]));
    assert_eq!(a.live_summary()["share"]["shared"], false, "the open tab follows");
    let mut c = session("Cy");
    ex(&mut c, "pref.publishServer", json!({"url": url}));
    ex(&mut c, "collab.join", json!({"code": code}));
    pump(&mut [&mut c], "the old code refused", |s| matches!(s[0].live().status, figcraft_engine::live::Status::Failed { .. }));
    // Start again: a new code for the same file.
    let again = ex(&mut a, "collab.share", json!({}));
    let new_code = again["code"].as_str().unwrap().to_string();
    assert_ne!(new_code, code);
    assert_eq!(ex(&mut a, "collab.files", json!({}))["files"][0]["code"], new_code);
    let mut d = session("Di");
    ex(&mut d, "pref.publishServer", json!({"url": url}));
    ex(&mut d, "collab.join", json!({"code": new_code}));
    pump(&mut [&mut a, &mut d], "Di in with the new code", |s| live(s[1]));
    // Rename, then remove: the file leaves the server's disk and the list; the tab's session ends.
    ex(&mut a, "collab.manageFile", json!({"file": file, "action": "rename", "name": "Landing"}));
    assert_eq!(ex(&mut a, "collab.files", json!({"refresh": true}))["files"][0]["name"], "Landing");
    assert!(dir.path().join("files").join(&file).exists());
    assert_eq!(ex(&mut a, "collab.manageFile", json!({"file": file, "action": "remove"}))["done"], true);
    assert!(!dir.path().join("files").join(&file).exists(), "gone from the data directory");
    assert!(a.prefs.online_files.is_empty() && !a.live().status.active() && a.docs.len() == 2, "the copy stays open");
    server.stop();
}

#[test]
fn a_file_that_expired_or_was_removed_elsewhere_shows_as_gone() {
    let dir = TempDir::new("owned-gone");
    let cfg = open(dir.path());
    let mut server = crate::start(&cfg).unwrap();
    let url = format!("ws://{}/", server.addr());
    let mut a = share(&url);
    let file = a.prefs.online_files[0].file.clone();
    ex(&mut a, "collab.leave", json!({}));
    let mut a = restart(a, "Ada");
    // The operator removes it.
    crate::cli::run(&crate::cli::Cli { config: cfg.clone(), command: crate::cli::Command::RemoveFile { file: file.clone() } }).unwrap();
    let list = ex(&mut a, "collab.files", json!({"refresh": true}));
    assert_eq!((&list["files"][0]["state"], &list["files"][0]["shared"]), (&json!("gone"), &json!(false)));
    assert!(a.execute("collab.openFile", &json!({"file": file})).unwrap_err().to_string().contains("no longer on its server"));
    assert_eq!(ex(&mut a, "collab.forgetFile", json!({"file": file}))["onServer"], false);
    // The server stops: the list says it couldn't be reached and keeps what it has.
    let mut b = share(&url);
    ex(&mut b, "collab.leave", json!({}));
    server.stop();
    let list = ex(&mut b, "collab.files", json!({"refresh": true}));
    assert_eq!((&list["files"][0]["state"], &list["problems"][0]["state"]), (&json!("online"), &json!("offline")));
    assert_eq!(list["files"].as_array().unwrap().len(), 1);
}

#[test]
fn only_a_files_own_token_learns_about_it() {
    let dir = TempDir::new("owned-ask");
    let cfg = open(dir.path());
    let mut server = crate::start(&cfg).unwrap();
    let addr = server.addr();
    let a = share(&format!("ws://{addr}/"));
    let (file, owner) = (a.prefs.online_files[0].file.clone(), a.prefs.online_files[0].owner.clone());
    let opened_before = std::fs::read_to_string(dir.path().join("files").join(&file).join("meta.json")).unwrap();
    let mut raw = hosting(addr);
    let pair = |f: &str, o: &str| (f.to_string(), o.to_string());
    let reply = owned(
        &mut raw,
        vec![pair(&file, &owner), pair(&file, "WrongTokenWrongToken00"), pair("nosuchfile", &owner), pair("", ""), pair("../../etc", "x")],
    );
    let AdminReply::Owned(list) = reply else { panic!("answered {reply:?}") };
    assert_eq!(list.len(), 5, "one answer per pair, in order");
    let first = list[0].as_ref().expect("its own token");
    assert_eq!(first.name, "Untitled");
    assert_eq!(first.links.len(), 5, "the owner's two links and the three made for others");
    assert!(first.expires.is_some());
    assert!(list[1..].iter().all(Option::is_none), "a wrong token and a missing file read the same");
    // Nothing asked: nothing answered; too many: refused, and the connection stays usable.
    assert_eq!(owned(&mut raw, vec![]), AdminReply::Owned(vec![]));
    let many = vec![pair(&file, &owner); MAX_OWNED + 1];
    assert!(matches!(owned(&mut raw, many), AdminReply::Failed { reason } if reason.contains("at most")));
    assert!(matches!(owned(&mut raw, vec![pair(&file, &owner); MAX_OWNED]), AdminReply::Owned(l) if l.len() == MAX_OWNED));
    // Junk ids and tokens (long, control characters, other scripts) are just not theirs.
    let junk = vec![pair(&"x".repeat(100_000), &"y".repeat(100_000)), pair("\u{0}\u{7}", "\u{1F4A5}"), pair(&file, &format!("{owner}\u{0}"))];
    assert!(matches!(owned(&mut raw, junk), AdminReply::Owned(l) if l.iter().all(Option::is_none)));
    // On that connection the token still manages nothing: it named no file when it opened.
    raw.send(&ClientMsg::Admin(Admin::Remove { file: file.clone() }));
    assert!(matches!(raw.next(), ServerMsg::Admin(r) if matches!(*r, AdminReply::Failed { .. })));
    assert!(dir.path().join("files").join(&file).exists());
    // Asking opened nothing: when the file was last opened is as it was.
    assert_eq!(std::fs::read_to_string(dir.path().join("files").join(&file).join("meta.json")).unwrap(), opened_before);
    // A connection that manages one file with its token may ask too; the server key as well.
    let mut one = Raw::connect(addr);
    one.send(&ClientMsg::OpenHosting(OpenHosting { protocol: PROTOCOL, client_version: "test".into(), file: file.clone(), owner: owner.clone() }));
    assert!(matches!(one.next(), ServerMsg::Hosting(_)));
    assert!(matches!(owned(&mut one, vec![pair(&file, &owner)]), AdminReply::Owned(l) if l[0].is_some()));
    drop(a);
    server.stop();
}

#[test]
fn a_server_that_isnt_open_to_the_public_answers_that_nothing_is_owned() {
    let dir = TempDir::new("owned-private");
    let mut server = crate::start(&config(dir.path())).unwrap();
    let mut raw = hosting(server.addr());
    assert!(matches!(owned(&mut raw, vec![("f".into(), "t".into())]), AdminReply::Owned(l) if l == [None]));
    server.stop();
}
