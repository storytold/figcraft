//! Your online files against a scripted link: a file published without a key is recorded with
//! its owner token, which never shows in a command's result, a summary or debug output; the list
//! survives a restart (and junk in the preferences file); it is capped; a recorded file opens
//! again as its owner, where sharing stops and starts; `collab.manageFile` takes its server and
//! token from the record; one request per server says what became of the files (gone, renamed,
//! unshared elsewhere), and a server that can't be reached or is too old leaves the list as it is;
//! and what happens in a file that isn't in front names the file.

use std::sync::{Arc, Mutex};

use figcraft_collab::message::{Admin, AdminReply, ClientMsg, HostInfo, OwnedFile, Peer};
use serde_json::{Value, json};

use crate::live::{Access, Event, HostOptions, LiveLink, Presence, PublishOptions, Published, Role, SessionInfo, Shared, SharedLink, Status};
use crate::online::{FileState, MAX_ONLINE_FILES, OnlineFile, ServerProblem};
use crate::{Prefs, Session};

const SERVER: &str = "wss://share.example/";
const OTHER: &str = "ws://127.0.0.1:47900/";
const OWNER: &str = "OwnerSecretOwnerSecret";
const EDIT_TOKEN: &str = "EditSecretEditSecret00";

/// What the scripted server keeps, what it was asked, and the events the link hands out next.
#[derive(Default)]
struct Script {
    joined: Vec<String>,
    published: Vec<PublishOptions>,
    /// (server, file, owner, request) of every hosting request.
    hosting: Vec<(String, String, String, Option<Admin>)>,
    /// (server, file, owner, revoke, create) of every share request.
    shares: Vec<(String, String, String, Vec<String>, bool)>,
    /// The files there, by id: `Admin::Owned` answers from it.
    files: Vec<(String, OwnedFile)>,
    /// Servers (by a part of their address) that can't be reached, or that can't read `Owned`.
    offline: Vec<&'static str>,
    old: Vec<&'static str>,
    left: Vec<u64>,
    events: Vec<Event>,
    session: u64,
}

struct Link(Arc<Mutex<Script>>);

impl LiveLink for Link {
    fn host(&mut self, _: u64, _: HostOptions) -> Result<SessionInfo, String> {
        Err("not in this test".into())
    }
    fn join(&mut self, session: u64, link: &str, _: &str) -> Result<(), String> {
        let mut s = self.0.lock().unwrap();
        s.session = session;
        s.joined.push(link.to_string());
        Ok(())
    }
    fn publish(&mut self, session: u64, opts: PublishOptions) -> Result<(), String> {
        let mut s = self.0.lock().unwrap();
        s.session = session;
        s.published.push(opts);
        Ok(())
    }
    fn hosting(&mut self, server: &str, file: &str, owner: &str, req: Option<Admin>) -> Result<(HostInfo, Option<AdminReply>), String> {
        let mut s = self.0.lock().unwrap();
        s.hosting.push((server.into(), file.into(), owner.into(), req.clone()));
        if s.offline.iter().any(|o| server.contains(o)) {
            return Err("no answer from the server (timed out)".into());
        }
        let reply = match req {
            Some(Admin::Owned { .. }) if s.old.iter().any(|o| server.contains(o)) => {
                return Err("the server refused: that message couldn't be read".into());
            }
            Some(Admin::Owned { files }) => {
                let known = |(file, owner): &(String, String)| s.files.iter().find(|(id, _)| id == file && owner == OWNER).map(|(_, f)| f.clone());
                AdminReply::Owned(files.iter().map(known).collect())
            }
            Some(Admin::Remove { file }) => {
                s.files.retain(|(id, _)| *id != file);
                AdminReply::Done
            }
            Some(Admin::Rename { .. }) => AdminReply::Done,
            _ => AdminReply::Failed { reason: "not in this test".into() },
        };
        Ok((HostInfo::default(), Some(reply)))
    }
    fn share(&mut self, server: &str, file: &str, owner: &str, revoke: &[String], create: bool) -> Result<Option<Shared>, String> {
        let mut s = self.0.lock().unwrap();
        if s.offline.iter().any(|o| server.contains(o)) {
            return Err("no answer from the server (timed out)".into());
        }
        s.shares.push((server.into(), file.into(), owner.into(), revoke.to_vec(), create));
        let n = s.shares.iter().filter(|a| a.4).count();
        Ok(create.then(|| links(file, n)))
    }
    fn leave(&mut self, session: u64) {
        self.0.lock().unwrap().left.push(session);
    }
    fn presence(&mut self, _: u64, _: &Presence) {}
    fn send(&mut self, _: u64, _: ClientMsg) {}
    fn poll(&mut self) -> Vec<(u64, Event)> {
        let mut s = self.0.lock().unwrap();
        let session = s.session;
        std::mem::take(&mut s.events).into_iter().map(|e| (session, e)).collect()
    }
}

/// The links a server makes for others the `n`th time `file` is shared, with their codes.
fn links(file: &str, n: usize) -> Shared {
    let link = |what: &str, code: String| SharedLink {
        id: format!("{what}{n}"),
        link: format!("figcraft://join?server={SERVER}&file={file}&token=Shared{what}Shared{what}0{n}"),
        code: Some(code),
    };
    Shared { edit: link("edit", format!("NEWED1T0{n}")), view: Some(link("view", format!("NEWV1EW0{n}"))), comment: None }
}

fn session() -> (Session, Arc<Mutex<Script>>) {
    with_prefs(Prefs::default())
}

/// A run of the app with `prefs` loaded.
fn with_prefs(prefs: Prefs) -> (Session, Arc<Mutex<Script>>) {
    let mut s = Session::new();
    s.prefs = prefs;
    let script = Arc::new(Mutex::new(Script::default()));
    s.live.install(Box::new(Link(script.clone())));
    (s, script)
}

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn err(s: &mut Session, cmd: &str, p: Value) -> String {
    s.execute(cmd, &p).unwrap_err().to_string()
}

fn edit_link(server: &str, file: &str) -> String {
    format!("figcraft://join?server={server}&file={file}&token={EDIT_TOKEN}")
}

fn info(role: Role, name: &str, link: &str) -> SessionInfo {
    SessionInfo {
        role,
        me: 1,
        name: "Me".into(),
        color: "#0D99FF".into(),
        file_name: name.into(),
        pages: vec![],
        link: link.into(),
        url: String::new(),
        address: SERVER.into(),
        network: true,
        access: Access::Edit,
    }
}

/// The open file was shared online (`collab.share`) and reached the server as file `file`: its
/// owner's links and token, the links for others, and the owner in it.
fn shared_online(s: &mut Session, script: &Arc<Mutex<Script>>, file: &str) {
    ex(s, "pref.publishServer", json!({"url": SERVER}));
    ex(s, "collab.share", json!({}));
    let published = Published {
        server: "share.example".into(),
        file: file.into(),
        name: "Homepage".into(),
        edit: edit_link(SERVER, file),
        view: format!("figcraft://join?server={SERVER}&file={file}&token=ViewSecretViewSecret00"),
        owner: Some(OWNER.into()),
        expires: Some(1_900_000_000),
    };
    let welcome = Event::Welcome { info: info(Role::Guest, "Homepage", &published.edit), peers: vec![] };
    let mut sc = script.lock().unwrap();
    sc.files.push((
        file.into(),
        OwnedFile { name: "Homepage".into(), links: vec!["own-edit".into(), "own-view".into(), "edit0".into(), "view0".into()], expires: None },
    ));
    sc.events.extend([Event::Published(published), Event::Shared(Ok(links(file, 0))), welcome]);
    drop(sc);
    s.live_poll();
}

/// A record as the preferences file holds it.
fn record(server: &str, file: &str, opened: u64) -> Value {
    json!({"server": server, "file": file, "name": format!("File {file}"), "owner": OWNER, "edit": edit_link(server, file), "view": "", "created": 1, "opened": opened})
}

/// The preferences saved and loaded again (a restart).
fn restarted(s: &Session) -> Prefs {
    serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap()
}

fn no_secrets(what: &str, text: &str) {
    for secret in [OWNER, EDIT_TOKEN] {
        assert!(!text.contains(secret), "{what} carries a secret: {text}");
    }
}

#[test]
fn a_file_shared_online_is_recorded_and_its_token_never_shows() {
    let (mut s, script) = session();
    assert!(ex(&mut s, "collab.files", json!({}))["files"].as_array().unwrap().is_empty());
    shared_online(&mut s, &script, "f1");
    assert!(std::mem::take(&mut s.live.unsaved), "the app saves the preferences at once");
    let [f] = s.prefs.online_files.clone().try_into().unwrap_or_else(|v: Vec<OnlineFile>| panic!("{} recorded", v.len()));
    assert_eq!((f.server.as_str(), f.file.as_str(), f.name.as_str(), f.owner.as_str()), (SERVER, "f1", "Homepage", OWNER));
    assert_eq!((f.edit.as_str(), f.state, f.expires), (edit_link(SERVER, "f1").as_str(), FileState::Online, Some(1_900_000_000)));
    assert!(f.created > 1_700_000_000 && f.opened == f.created);
    assert_eq!(f.shared.as_ref().and_then(|l| l.edit.code.as_deref()), Some("NEWED1T00"), "what others join with is recorded too");
    // The list a script gets: the code to hand out, no token and none of the owner's links.
    let list = ex(&mut s, "collab.files", json!({}));
    let row = &list["files"][0];
    assert_eq!(
        (&row["host"], &row["file"], &row["name"], &row["shared"], &row["code"], &row["state"], &row["hasOwnerToken"], &row["document"]),
        (&json!("share.example"), &json!("f1"), &json!("Homepage"), &json!(true), &json!("NEW-ED1-T00"), &json!("online"), &json!(true), &json!(0))
    );
    assert_eq!(list["max"], MAX_ONLINE_FILES);
    no_secrets("collab.files", &list.to_string());
    // Debug output of the record and of everything that holds it.
    no_secrets("the record's debug output", &format!("{f:?}"));
    no_secrets("the preferences' debug output", &format!("{:?}", s.prefs));
    assert!(script.lock().unwrap().hosting.is_empty(), "listing asks no server");
}

#[test]
fn a_file_published_with_the_servers_key_is_the_operators_and_is_not_recorded() {
    let (mut s, script) = session();
    ex(&mut s, "collab.publish", json!({"server": "ws://127.0.0.1:47900/?key=ServerKeyServerKey"}));
    let published = Published {
        server: "127.0.0.1:47900".into(),
        file: "k1".into(),
        name: "Keyed".into(),
        edit: edit_link(OTHER, "k1"),
        view: String::new(),
        owner: None,
        expires: None,
    };
    script.lock().unwrap().events.push(Event::Published(published));
    s.live_poll();
    assert!(s.prefs.online_files.is_empty() && !s.live.unsaved);
}

#[test]
fn the_list_survives_a_restart_and_junk_in_the_preferences_file() {
    let (mut s, script) = session();
    shared_online(&mut s, &script, "f1");
    let saved = serde_json::to_value(&s.prefs).unwrap();
    assert_eq!(saved["online_files"][0]["owner"], OWNER, "the token is kept on this computer, with the preferences");
    assert!(saved["online_files"][0].get("state").is_none(), "what the server said last isn't saved");
    let loaded = restarted(&s);
    let [f] = loaded.online_files.as_slice() else { panic!("{} loaded", loaded.online_files.len()) };
    assert_eq!((f.file.as_str(), f.owner.as_str(), f.state, f.shared.is_some()), ("f1", OWNER, FileState::Unknown, true));
    // Preferences from before there was a list, and ones without any, load and save as before.
    let old: Prefs = serde_json::from_value(json!({"theme": "dark"})).unwrap();
    assert!(old.online_files.is_empty() && serde_json::to_value(&old).unwrap().get("online_files").is_none());
    // Junk where the list should be never costs the other preferences.
    for junk in [json!("junk"), json!(7), json!(null), json!({"server": SERVER}), json!([1, "x", null, [], {}]), json!([[record(SERVER, "a", 1)]])] {
        let p: Prefs = serde_json::from_value(json!({"theme": "dark", "online_files": junk})).unwrap();
        assert!(p.theme == "dark" && p.online_files.is_empty(), "{junk}");
    }
    // Entries that can't be records are dropped one by one; the usable ones stay, cleaned.
    let with = |change: fn(&mut Value)| {
        let mut r = record(SERVER, "bad", 1);
        change(&mut r);
        r
    };
    // Usable: a key in the address is dropped, control characters leave the name, a view link
    // and share links that aren't links go, a code that isn't one goes.
    let mut usable = record("wss://share.example/?key=ServerKey", "ok1", 9);
    usable["name"] = json!("  Home\u{0}page\u{7}  ");
    usable["view"] = json!("nope");
    usable["shared"] = json!({"edit": {"id": "e", "link": edit_link(SERVER, "ok1"), "code": "not-a-code!"}, "view": {"id": "v", "link": "nope"}});
    let list = json!([
        with(|r| r["server"] = json!("https://share.example/")),
        with(|r| r["server"] = json!(5)),
        with(|r| r["file"] = json!("../../etc")),
        with(|r| r["file"] = json!("")),
        with(|r| r["owner"] = json!("")),
        with(|r| r["owner"] = json!("two words")),
        with(|r| r["owner"] = json!("x".repeat(5000))),
        with(|r| r["edit"] = json!("not a link")),
        with(|r| r["opened"] = json!("yesterday")),
        with(|r| r["shared"] = json!("yes")),
        usable,
        record(SERVER, "ok2", 3),
        // The same file twice: once.
        record("wss://share.example:443", "ok2", 2),
    ]);
    let p: Prefs = serde_json::from_value(json!({"online_files": list})).unwrap();
    let ids: Vec<&str> = p.online_files.iter().map(|f| f.file.as_str()).collect();
    assert_eq!(ids, ["ok1", "ok2"]);
    let f = &p.online_files[0];
    assert_eq!((f.server.as_str(), f.name.as_str(), f.view.as_str()), (SERVER, "Homepage", ""));
    let shared = f.shared.as_ref().unwrap();
    assert!(shared.edit.code.is_none() && shared.view.is_none());
}

#[test]
fn the_list_is_capped_and_the_file_opened_longest_ago_goes_first() {
    // More on disk than the cap: the first ones (most recently opened) are kept.
    let on_disk: Vec<Value> = (0..MAX_ONLINE_FILES + 50).map(|i| record(SERVER, &format!("f{i}"), 10_000 - i as u64)).collect();
    let p: Prefs = serde_json::from_value(json!({"online_files": on_disk})).unwrap();
    assert_eq!(p.online_files.len(), MAX_ONLINE_FILES);
    assert_eq!(p.online_files.last().map(|f| f.file.as_str()), Some(format!("f{}", MAX_ONLINE_FILES - 1).as_str()));
    // One more put online: it leads the list and the one opened longest ago is dropped.
    let (mut s, script) = with_prefs(p);
    shared_online(&mut s, &script, "new");
    assert_eq!(s.prefs.online_files.len(), MAX_ONLINE_FILES);
    assert_eq!(s.prefs.online_files.first().map(|f| f.file.as_str()), Some("new"));
    assert!(!s.prefs.online_files.iter().any(|f| f.file == format!("f{}", MAX_ONLINE_FILES - 1)));
    assert!(s.prefs.online_files.iter().any(|f| f.file == format!("f{}", MAX_ONLINE_FILES - 2)));
}

#[test]
fn a_recorded_file_opens_again_as_its_owner_after_a_restart() {
    let (mut s, script) = session();
    shared_online(&mut s, &script, "f1");
    // The app quits; the next run has only the preferences.
    let (mut s, script) = with_prefs(restarted(&s));
    assert!(s.live.iter().next().is_none());
    let r = ex(&mut s, "collab.openFile", json!({"file": "f1"}));
    assert_eq!(
        (&r["status"], &r["alreadyOpen"], &r["name"], &r["server"]),
        (&json!("connecting"), &json!(false), &json!("Homepage"), &json!(SERVER))
    );
    no_secrets("collab.openFile", &r.to_string());
    assert_eq!(script.lock().unwrap().joined, [edit_link(SERVER, "f1")], "joined with the owner's own edit link");
    assert!(s.prefs.online_files[0].opened >= s.prefs.online_files[0].created && std::mem::take(&mut s.live.unsaved));
    // The file arrives in a tab of its own, synced.
    let (_, _, sync) = crate::live_tests::serve(&crate::Session::new().doc().unwrap().clone());
    let welcome = Event::Welcome { info: info(Role::Guest, "Homepage", &edit_link(SERVER, "f1")), peers: vec![] };
    script.lock().unwrap().events.extend([welcome, sync]);
    s.live_poll();
    assert_eq!((s.docs.len(), s.active), (2, 1));
    let peers = ex(&mut s, "collab.peers", json!({}));
    assert_eq!(
        (&peers["status"], &peers["syncing"], &peers["code"], &peers["share"]["shared"]),
        (&json!("live"), &json!(true), &json!("NEW-ED1-T00"), &json!(true))
    );
    assert!(!peers.to_string().contains(OWNER), "the token stays in the record: {peers}");
    assert_eq!(
        s.online_of(s.docs[1].uid).map(|(server, file, _)| (server, file)),
        Some((SERVER.to_string(), "f1".to_string())),
        "what a crash recovery finds it by"
    );
    assert_eq!(s.online_of(s.docs[0].uid), None);
    assert_eq!(ex(&mut s, "collab.files", json!({}))["files"][0]["document"], 1);
    // Stop sharing as the owner: the recorded token, the recorded links.
    let stopped = ex(&mut s, "collab.stop", json!({}));
    assert_eq!((&stopped["ended"], &stopped["shared"]), (&json!(false), &json!(false)));
    {
        let sc = script.lock().unwrap();
        let (server, file, owner, revoke, create) = sc.shares.last().unwrap();
        assert_eq!((server.as_str(), file.as_str(), owner.as_str(), *create), (SERVER, "f1", OWNER, false));
        assert_eq!(revoke, &["edit0", "view0"]);
    }
    assert!(s.prefs.online_files[0].shared.is_none(), "the record follows");
    assert_eq!(ex(&mut s, "collab.files", json!({}))["files"][0]["shared"], false);
    // Share again: new links and a new code, recorded.
    let again = ex(&mut s, "collab.share", json!({}));
    assert_eq!(again["code"], "NEW-ED1-T01");
    assert_eq!(s.prefs.online_files[0].shared.as_ref().map(|l| l.edit.id.as_str()), Some("edit1"));
    assert!(script.lock().unwrap().published.is_empty(), "nothing is published twice");
    // Opening it once more brings its tab to the front.
    ex(&mut s, "doc.switch", json!({"index": 0}));
    let r = ex(&mut s, "collab.openFile", json!({"file": "f1", "server": "wss://share.example"}));
    assert_eq!((&r["alreadyOpen"], &r["document"], s.active), (&json!(true), &json!(1), 1));
    assert_eq!(script.lock().unwrap().joined.len(), 1);
}

#[test]
fn manage_file_takes_its_server_and_token_from_the_record() {
    let (mut s, script) = session();
    shared_online(&mut s, &script, "f1");
    let (mut s, script) = with_prefs(restarted(&s));
    // Unshare and share from the list, without opening the file.
    let r = ex(&mut s, "collab.manageFile", json!({"file": "f1", "action": "unshare"}));
    assert_eq!(r["shared"], false);
    assert_eq!(
        script.lock().unwrap().shares.last().map(|a| (a.2.clone(), a.3.clone(), a.4)),
        Some((OWNER.to_string(), vec!["edit0".into(), "view0".into()], false))
    );
    assert_eq!(ex(&mut s, "collab.manageFile", json!({"file": "f1", "action": "unshare"}))["shared"], false);
    assert_eq!(script.lock().unwrap().shares.len(), 1, "not shared already: the server isn't asked");
    let r = ex(&mut s, "collab.manageFile", json!({"file": "f1", "action": "share"}));
    assert_eq!((&r["shared"], &r["code"], &r["viewCode"]), (&json!(true), &json!("NEW-ED1-T01"), &json!("NEW-V1E-W01")));
    no_secrets("collab.manageFile share", &r.to_string());
    assert_eq!(ex(&mut s, "collab.manageFile", json!({"file": "f1", "action": "share"}))["code"], "NEW-ED1-T01", "shared already: the same code");
    assert_eq!(script.lock().unwrap().shares.len(), 2);
    assert!(restarted(&s).online_files[0].shared.is_some());
    // Rename: the record follows.
    assert_eq!(ex(&mut s, "collab.manageFile", json!({"file": "f1", "action": "rename", "name": "Landing"}))["done"], true);
    assert_eq!(s.prefs.online_files[0].name, "Landing");
    {
        let sc = script.lock().unwrap();
        let (server, file, owner, req) = sc.hosting.last().unwrap();
        assert_eq!((server.as_str(), file.as_str(), owner.as_str()), (SERVER, "f1", OWNER));
        assert!(matches!(req, Some(Admin::Rename { name, .. }) if name == "Landing"));
    }
    // A server or token given explicitly is used as given (a file that isn't recorded here).
    ex(&mut s, "collab.manageFile", json!({"server": OTHER, "file": "x9", "owner": "TheirToken", "action": "rename", "name": "N"}));
    assert_eq!(script.lock().unwrap().hosting.last().map(|h| (h.0.clone(), h.2.clone())), Some((OTHER.to_string(), "TheirToken".to_string())));
    // Neither given nor recorded.
    let e = err(&mut s, "collab.manageFile", json!({"file": "nope", "action": "remove"}));
    assert!(e.contains("`server` is missing") && e.contains("collab.files"), "{e}");
    let e = err(&mut s, "collab.manageFile", json!({"server": OTHER, "file": "f1", "action": "remove"}));
    assert!(e.contains("`owner` is missing"), "{e}");
    // Remove: off the server, out of the list, and a session open on it ends.
    ex(&mut s, "collab.openFile", json!({"file": "f1"}));
    script.lock().unwrap().events.push(Event::Welcome { info: info(Role::Guest, "Landing", &edit_link(SERVER, "f1")), peers: vec![] });
    s.live_poll();
    let session = s.live.iter().next().map(|l| l.id).unwrap();
    assert_eq!(ex(&mut s, "collab.manageFile", json!({"file": "f1", "action": "remove"}))["done"], true);
    assert!(s.prefs.online_files.is_empty() && script.lock().unwrap().files.is_empty());
    assert!(s.live.get(session).is_none() && script.lock().unwrap().left.contains(&session));
    assert!(err(&mut s, "collab.openFile", json!({"file": "f1"})).contains("no online file with that id"));
}

#[test]
fn each_server_is_asked_once_what_became_of_its_files() {
    let mut unshared = record(SERVER, "unshared", 6);
    unshared["shared"] = json!({"edit": {"id": "e9", "link": edit_link(SERVER, "unshared"), "code": "K7M2QX9TD"}});
    let list = json!([record(SERVER, "kept", 9), record(OTHER, "there", 8), record(SERVER, "gone", 7), unshared]);
    let prefs: Prefs = serde_json::from_value(json!({"online_files": list})).unwrap();
    let (mut s, script) = with_prefs(prefs);
    {
        let mut sc = script.lock().unwrap();
        let file = |name: &str, links: &[&str]| OwnedFile {
            name: name.into(),
            links: links.iter().map(|l| l.to_string()).collect(),
            expires: Some(1_900_000_000),
        };
        sc.files = vec![
            ("kept".into(), file("Renamed elsewhere", &["a", "b"])),
            ("there".into(), file("File there", &[])),
            ("unshared".into(), file("File unshared", &["a"])),
        ];
    }
    // Listing alone asks nothing.
    let list = ex(&mut s, "collab.files", json!({}));
    assert!(list["files"].as_array().unwrap().iter().all(|f| f["state"] == "unknown") && script.lock().unwrap().hosting.is_empty());
    let list = ex(&mut s, "collab.files", json!({"refresh": true}));
    {
        let sc = script.lock().unwrap();
        let asked: Vec<(&str, usize)> = sc
            .hosting
            .iter()
            .map(|(server, file, owner, req)| {
                assert!(file.is_empty() && owner.is_empty(), "no file is named when the connection opens");
                let Some(Admin::Owned { files }) = req else { panic!("asked {req:?}") };
                (server.as_str(), files.len())
            })
            .collect();
        assert_eq!(asked, [(SERVER, 3), (OTHER, 1)], "one request per server, with all its files");
    }
    let by = |id: &str| list["files"].as_array().unwrap().iter().find(|f| f["file"] == id).cloned().unwrap();
    assert_eq!(
        (&by("kept")["state"], &by("kept")["name"], &by("kept")["expires"]),
        (&json!("online"), &json!("Renamed elsewhere"), &json!(1_900_000_000u64))
    );
    assert_eq!(by("there")["state"], "online");
    assert_eq!((&by("gone")["state"], &by("gone")["shared"]), (&json!("gone"), &json!(false)));
    assert_eq!(
        (&by("unshared")["state"], &by("unshared")["shared"], &by("unshared")["code"]),
        (&json!("online"), &json!(false), &Value::Null),
        "its link was revoked elsewhere"
    );
    assert!(list["problems"].as_array().unwrap().is_empty());
    no_secrets("collab.files {refresh}", &list.to_string());
    // A file that is gone can't be opened; it can be cleared from the list.
    assert!(err(&mut s, "collab.openFile", json!({"file": "gone"})).contains("no longer on its server"));
    let r = ex(&mut s, "collab.forgetFile", json!({"file": "gone"}));
    assert_eq!((&r["forgotten"], &r["onServer"]), (&json!(true), &json!(false)));
    // Forgetting one that is there says the file stays, and asks no server.
    let before = script.lock().unwrap().hosting.len();
    let r = ex(&mut s, "collab.forgetFile", json!({"file": "there"}));
    assert_eq!((&r["forgotten"], &r["onServer"], &r["server"]), (&json!(true), &json!(true), &json!(OTHER)));
    assert_eq!(script.lock().unwrap().hosting.len(), before);
    assert_eq!(s.prefs.online_files.len(), 2);
    assert!(err(&mut s, "collab.forgetFile", json!({"file": "there"})).contains("no online file with that id"));
}

#[test]
fn a_server_that_cant_be_reached_or_is_too_old_leaves_the_list_as_it_is() {
    let prefs: Prefs = serde_json::from_value(json!({"online_files": [record(SERVER, "a", 9), record(OTHER, "b", 8)]})).unwrap();
    let (mut s, script) = with_prefs(prefs);
    script.lock().unwrap().offline = vec!["share.example"];
    script.lock().unwrap().old = vec!["127.0.0.1"];
    let list = ex(&mut s, "collab.files", json!({"refresh": true}));
    assert_eq!(list["files"].as_array().unwrap().len(), 2, "what this computer remembers still shows");
    assert!(list["files"].as_array().unwrap().iter().all(|f| f["state"] == "unknown"));
    let problems = list["problems"].as_array().unwrap();
    let of = |host: &str| problems.iter().find(|p| p["host"] == host).cloned().unwrap();
    assert_eq!(of("share.example")["state"], "offline");
    assert!(of("share.example")["reason"].as_str().unwrap().contains("no answer"));
    assert_eq!(of("127.0.0.1:47900")["state"], "unsupported");
    assert_eq!(s.live.servers.get(OTHER), Some(&ServerProblem::Unsupported));
    // Sharing while offline changes nothing and says why.
    let e = err(&mut s, "collab.manageFile", json!({"file": "a", "action": "share"}));
    assert!(e.contains("no answer") && s.prefs.online_files[0].shared.is_none(), "{e}");
    // Back online: the problem clears.
    script.lock().unwrap().offline.clear();
    script.lock().unwrap().files.push(("a".into(), OwnedFile::default()));
    let list = ex(&mut s, "collab.files", json!({"refresh": true}));
    assert_eq!(list["problems"].as_array().unwrap().len(), 1);
    assert_eq!(list["files"][0]["state"], "online");
    // Without the network (the command line) a refresh says so; the list still reads.
    let mut headless = Session::new();
    headless.prefs = s.prefs.clone();
    assert!(err(&mut headless, "collab.files", json!({"refresh": true})).contains("desktop app"));
    assert_eq!(ex(&mut headless, "collab.files", json!({}))["files"].as_array().unwrap().len(), 2);
}

#[test]
fn files_with_the_same_id_on_two_servers_need_the_server_named() {
    let prefs: Prefs = serde_json::from_value(json!({"online_files": [record(SERVER, "same", 9), record(OTHER, "same", 8)]})).unwrap();
    let (mut s, script) = with_prefs(prefs);
    assert!(err(&mut s, "collab.openFile", json!({"file": "same"})).contains("say which with `server`"));
    ex(&mut s, "collab.openFile", json!({"file": "same", "server": OTHER}));
    assert_eq!(script.lock().unwrap().joined, [edit_link(OTHER, "same")]);
    assert_eq!(s.prefs.online_files[0].server, OTHER, "most recently opened first");
}

#[test]
fn what_happens_in_a_file_that_isnt_in_front_names_the_file() {
    let (mut s, script) = session();
    ex(&mut s, "file.rename", json!({"name": "Homepage"}));
    shared_online(&mut s, &script, "f1");
    let peer = |id, name: &str| Peer { client_id: id, name: name.into(), color: "#9747FF".into() };
    // In front: as before.
    script.lock().unwrap().events.push(Event::PeerJoined(peer(2, "Ada")));
    s.live_poll();
    assert_eq!(s.message.take().as_deref(), Some("Ada joined"));
    // Another file in front.
    ex(&mut s, "file.new", json!({}));
    script.lock().unwrap().events.push(Event::PeerJoined(peer(3, "Hana")));
    s.live_poll();
    assert_eq!(s.message.take().as_deref(), Some("Hana joined Homepage"));
    script.lock().unwrap().events.push(Event::PeerLeft(2));
    s.live_poll();
    assert_eq!(s.message.take().as_deref(), Some("Ada left Homepage"));
    script.lock().unwrap().events.push(Event::Ended { reason: "The server stopped".into() });
    s.live_poll();
    assert_eq!(s.message.take().as_deref(), Some("Homepage: The server stopped"));
    assert_eq!(s.live.iter().filter(|l| l.status == Status::Live).count(), 0);
}

#[test]
fn the_new_commands_survive_junk_with_files_recorded() {
    let prefs: Prefs = serde_json::from_value(json!({"online_files": [record(SERVER, "a", 9), record(OTHER, "b", 8)]})).unwrap();
    let junk = [
        json!({"file": [1], "server": 7, "refresh": "maybe", "action": "share", "name": {"x": 1}}),
        json!({"file": "../x", "server": "wss://\u{0}", "refresh": true, "action": "unshare", "name": 5}),
        json!({"file": "a", "server": "\u{1F4A5}".repeat(64), "refresh": 1e308, "action": "rename", "name": ""}),
        json!({"file": "a", "server": null, "owner": [], "refresh": null, "action": 3}),
        json!({"file": "", "server": "", "owner": "", "action": ""}),
        json!([1, 2]),
        json!("a"),
    ];
    for cmd in ["collab.files", "collab.openFile", "collab.forgetFile", "collab.manageFile"] {
        for j in &junk {
            let (mut s, _) = with_prefs(prefs.clone());
            if let Err(crate::EngineError::Internal { cmd, msg }) = s.execute(cmd, j) {
                panic!("{cmd} panicked on {j}: {msg}");
            }
            // Nothing a junk call returns or leaves behind names a secret.
            if let Ok(v) = s.execute("collab.files", &json!({})) {
                no_secrets(cmd, &v.to_string());
            }
        }
    }
}

/// The record type itself: what `Debug` prints.
#[test]
fn debug_output_of_a_record_leaves_the_secrets_out() {
    let f = OnlineFile {
        server: SERVER.into(),
        file: "f".into(),
        name: "N".into(),
        owner: OWNER.into(),
        edit: edit_link(SERVER, "f"),
        ..OnlineFile::default()
    };
    let text = format!("{f:?} {:#?}", vec![f.clone()]);
    no_secrets("debug", &text);
    assert!(text.contains("share.example") && text.contains(".."), "{text}");
}

#[test]
fn same_server_ignores_the_default_port_before_a_path() {
    use crate::online::same_server;
    assert!(same_server("wss://share.example:443/figcraft/", "wss://share.example/figcraft/"));
    assert!(same_server("ws://host:80/x", "ws://host/x"));
    assert!(!same_server("wss://share.example:8443/figcraft/", "wss://share.example/figcraft/"));
}
