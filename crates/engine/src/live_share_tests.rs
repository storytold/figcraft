//! Sharing with a join code against a scripted link: `collab.share` online and on the local
//! network, the codes in `collab.peers`, switching what the code lets people do, stopping (online:
//! the others' links go, the file and its owner stay; sharing again makes new ones), and
//! `collab.join` with a code of either kind or a link (what typing tolerates, and what it says
//! when it can't be a code).

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};

use figcraft_collab::code::{self, Local};
use figcraft_collab::message::ClientMsg;
use serde_json::{Value, json};

use crate::live::{
    Access, Bind, DEFAULT_SERVER, Event, HostOptions, LiveLink, Presence, PublishOptions, Published, Role, SessionInfo, Shared, SharedLink, Status,
    join_target,
};
use crate::{EngineError, Session};

/// What the link was asked to do, and the events it hands out next (as the session started last).
#[derive(Default)]
struct Script {
    hosted: Vec<HostOptions>,
    published: Vec<PublishOptions>,
    joined: Vec<String>,
    /// What the server was asked about a file's sharing: (server, file, owner, revoke, create).
    asked: Vec<(String, String, String, Vec<String>, bool)>,
    /// The server can't be reached.
    offline: bool,
    left: Vec<u64>,
    events: Vec<Event>,
    session: u64,
}

struct Link(Arc<Mutex<Script>>);

/// Where the scripted host listens, and its session's secret.
const HOST: Local = Local { ip: Ipv4Addr::new(192, 168, 1, 20), port: 47_900, secret: 0x123_4567_89AB };

impl LiveLink for Link {
    fn host(&mut self, session: u64, opts: HostOptions) -> Result<SessionInfo, String> {
        let mut s = self.0.lock().unwrap();
        s.session = session;
        // As the network link does: a short secret when the session is shared with a code.
        let link = if opts.short { HOST.link() } else { "figcraft://join?host=192.168.1.20:47900&file=f&token=AbCdEfGhIjKlMnOpQrStUv".to_string() };
        s.hosted.push(opts);
        Ok(SessionInfo {
            role: Role::Host,
            me: 1,
            name: "Me".into(),
            color: "#0D99FF".into(),
            file_name: "Shared".into(),
            pages: vec![],
            url: link.replace("figcraft://join?host=", "ws://"),
            link,
            address: "192.168.1.20:47900".into(),
            network: true,
            access: Access::Edit,
        })
    }
    fn join(&mut self, session: u64, link: &str, _: &str) -> Result<(), String> {
        if !link.starts_with("figcraft://join?") {
            return Err("that's not a live session invite".into());
        }
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
    fn share(&mut self, server: &str, file: &str, owner: &str, revoke: &[String], create: bool) -> Result<Option<Shared>, String> {
        let mut s = self.0.lock().unwrap();
        if s.offline {
            return Err("no answer from the server".into());
        }
        s.asked.push((server.into(), file.into(), owner.into(), revoke.to_vec(), create));
        // Each time the file is shared the server makes new links, with new codes.
        let n = s.asked.iter().filter(|a| a.4).count();
        Ok(create.then(|| links(n, Some((&format!("NEWED1T0{n}"), &format!("NEWV1EW0{n}"))))))
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

fn session() -> (Session, Arc<Mutex<Script>>) {
    let mut s = Session::new();
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

/// The links a server makes for others the `n`th time a file is shared, with their codes (a
/// server from before join codes has none).
fn links(n: usize, codes: Option<(&str, &str)>) -> Shared {
    let link = |what: &str, code: Option<&str>| SharedLink {
        id: format!("{what}{n}"),
        link: format!("figcraft://join?server=wss://share.example:443/&file=f1&token=Share{what}Share{what}0{n}"),
        code: code.map(str::to_string),
    };
    Shared { edit: link("edit", codes.map(|c| c.0)), view: Some(link("view", codes.map(|c| c.1))), comment: None }
}

/// The file is on the server and joined: its owner's links, and what others join with (codes
/// too, when given).
fn on_the_server(s: &mut Session, script: &Arc<Mutex<Script>>, codes: Option<(&str, &str)>) {
    let link = |token: &str| format!("figcraft://join?server=wss://share.example:443/&file=f1&token={token}");
    let published = Published {
        server: "wss://share.example:443/".into(),
        file: "f1".into(),
        name: "Untitled".into(),
        edit: link("EditEditEditEditEdit00"),
        view: link("ViewViewViewViewView00"),
        owner: Some("OwnerOwnerOwnerOwner00".into()),
        expires: None,
    };
    let info = SessionInfo {
        role: Role::Guest,
        me: 1,
        name: "Me".into(),
        color: "#0D99FF".into(),
        file_name: "Untitled".into(),
        pages: vec![],
        link: published.edit.clone(),
        url: String::new(),
        address: "wss://share.example:443/".into(),
        network: true,
        access: Access::Edit,
    };
    let events = vec![Event::Published(published), Event::Shared(Ok(links(0, codes))), Event::Welcome { info, peers: vec![] }];
    script.lock().unwrap().events.extend(events);
    s.live_poll();
}

/// The file reached the server and its owner is in it, without links for others.
fn on_the_server_unshared(s: &mut Session, script: &Arc<Mutex<Script>>) {
    on_the_server(s, script, None);
    if let Some(sh) = s.live_mut().and_then(|l| l.share.as_mut()) {
        sh.links = None;
    }
}

#[test]
fn the_built_in_server_is_the_default_and_can_be_changed_or_turned_off() {
    let (mut s, _) = session();
    assert_eq!(s.prefs.publish_server, DEFAULT_SERVER);
    // A build may ship without one (`FIGCRAFT_DEFAULT_SERVER=`); any it ships is TLS.
    assert!(
        DEFAULT_SERVER.is_empty() || (DEFAULT_SERVER.starts_with("wss://") && DEFAULT_SERVER.ends_with('/')),
        "TLS, and an address the preference itself accepts"
    );
    assert_eq!(ex(&mut s, "pref.publishServer", json!({"url": DEFAULT_SERVER}))["url"], DEFAULT_SERVER);
    // Another server; back to the built-in one; off.
    assert_eq!(ex(&mut s, "pref.publishServer", json!({"url": "wss://share.example/"}))["url"], "wss://share.example/");
    let reset = ex(&mut s, "pref.publishServer", json!({}));
    assert_eq!((&reset["url"], &reset["default"]), (&json!(DEFAULT_SERVER), &json!(DEFAULT_SERVER)));
    assert_eq!(ex(&mut s, "pref.publishServer", json!({"url": ""}))["url"], "");
    // Off is saved as off (an empty address isn't left out, or the default would come back).
    let saved = serde_json::to_value(&s.prefs).unwrap();
    assert_eq!(saved["publish_server"], "");
    let loaded: crate::Prefs = serde_json::from_value(saved).unwrap();
    assert!(loaded.publish_server.is_empty());
    // Preferences saved before there was a default get it.
    let old: crate::Prefs = serde_json::from_value(json!({"theme": "dark"})).unwrap();
    assert_eq!(old.publish_server, DEFAULT_SERVER);
}

/// The middle of the online-sharing test: re-asking switches codes, and an offline stop stops nothing.
fn asking_again_and_offline_stop(s: &mut Session, script: &Arc<Mutex<Script>>) {
    // Asking again only switches which code is the one to give out.
    let r = ex(s, "collab.share", json!({"access": "view"}));
    assert_eq!((&r["code"], &r["share"]["access"]), (&json!("4FH-J8R-2WN"), &json!("view")));
    assert_eq!(ex(s, "collab.share", json!({"via": "online"}))["code"], "4FH-J8R-2WN", "nothing new starts");
    assert_eq!(ex(s, "collab.share", json!({"access": "edit"}))["code"], "K7M-2QX-9TD");
    assert_eq!(script.lock().unwrap().published.len(), 1);
    assert!(err(s, "collab.share", json!({"via": "local"})).contains("stop sharing it first"));
    assert!(err(s, "collab.share", json!({"access": "admin"})).contains("`access` must be"));
    // Stopping while the server can't be reached stops nothing, and says so.
    script.lock().unwrap().offline = true;
    assert!(err(s, "collab.stop", json!({})).contains("couldn't stop sharing"));
    assert_eq!(ex(s, "collab.peers", json!({}))["code"], "K7M-2QX-9TD");
    script.lock().unwrap().offline = false;
}

/// The sharing-again step of the online-sharing test: new links and codes, nothing published twice.
fn sharing_again(s: &mut Session, script: &Arc<Mutex<Script>>) {
    // Sharing again: new links and codes for the same file, nothing published twice.
    let r = ex(s, "collab.share", json!({}));
    assert_eq!((&r["code"], &r["share"]["shared"], &r["share"]["viewCode"]), (&json!("NEW-ED1-T01"), &json!(true), &json!("NEW-V1E-W01")));
    assert!(r["share"]["editLink"].as_str().unwrap().contains("ShareeditShareedit01"));
    {
        let sc = script.lock().unwrap();
        assert_eq!(sc.published.len(), 1);
        assert_eq!(sc.asked[1], ("wss://share.example/".to_string(), "f1".to_string(), "OwnerOwnerOwnerOwner00".to_string(), vec![], true));
    }
    assert_eq!(ex(s, "collab.share", json!({}))["code"], "NEW-ED1-T01", "shared already: nothing more is asked");
    assert_eq!(script.lock().unwrap().asked.len(), 2);
}

#[test]
fn sharing_online_publishes_to_the_server_in_preferences_and_shows_its_code() {
    let (mut s, script) = session();
    ex(&mut s, "pref.publishServer", json!({"url": "wss://share.example/"}));
    let r = ex(&mut s, "collab.share", json!({}));
    assert_eq!(
        (&r["status"], &r["share"]["via"], &r["share"]["access"], &r["code"]),
        (&json!("connecting"), &json!("online"), &json!("edit"), &Value::Null)
    );
    assert_eq!(r["readOnly"], true, "as for any publish, until the file is there");
    {
        let sc = script.lock().unwrap();
        let [p] = sc.published.as_slice() else { panic!("published {}", sc.published.len()) };
        assert_eq!((p.server.as_str(), p.key.as_deref(), p.name.as_str(), p.share), ("wss://share.example/", None, "Untitled", true));
        assert!(sc.hosted.is_empty());
    }
    on_the_server(&mut s, &script, Some(("K7M2QX9TD", "4FHJ8R2WN")));
    let r = ex(&mut s, "collab.peers", json!({}));
    assert_eq!((&r["status"], &r["code"]), (&json!("live"), &json!("K7M-2QX-9TD")));
    assert_eq!(
        (&r["share"]["shared"], &r["share"]["editCode"], &r["share"]["viewCode"]),
        (&json!(true), &json!("K7M-2QX-9TD"), &json!("4FH-J8R-2WN"))
    );
    // What others get are links of their own, not the owner's.
    let (edit_link, own) = (r["share"]["editLink"].as_str().unwrap(), r["published"]["edit"].as_str().unwrap());
    assert!(edit_link.contains("ShareeditShareedit00") && edit_link != own && r["link"] == own, "{edit_link}");
    assert!(s.message.is_none(), "the popover shows the code: no toast about publishing");
    asking_again_and_offline_stop(&mut s, &script);
    // Stopping: the server is asked, as the file's owner, to revoke the others' links. The file
    // stays there and its owner stays in it: nothing is left, nothing is removed.
    assert_eq!(ex(&mut s, "collab.stop", json!({})), json!({"ended": false, "shared": false}));
    {
        let sc = script.lock().unwrap();
        assert!(sc.left.is_empty(), "the owner stays in the file");
        let want = (
            "wss://share.example/".to_string(),
            "f1".to_string(),
            "OwnerOwnerOwnerOwner00".to_string(),
            vec!["edit0".to_string(), "view0".to_string()],
            false,
        );
        assert_eq!(sc.asked, [want]);
    }
    let r = ex(&mut s, "collab.peers", json!({}));
    assert_eq!((&r["status"], &r["syncing"].is_boolean(), &r["code"]), (&json!("live"), &true, &Value::Null));
    assert_eq!(
        r["share"],
        json!({"via": "online", "access": "edit", "shared": false, "editCode": null, "viewCode": null, "commentCode": null, "editLink": null, "viewLink": null, "commentLink": null})
    );
    assert!(r["published"]["owner"].is_string(), "still the owner's file on the server");
    assert!(s.message.as_deref().unwrap_or_default().contains("stays on the server"));
    sharing_again(&mut s, &script);
    // Stopping again revokes the new ones; then `collab.stop` on a file that isn't shared leaves
    // its session like any other (the file stays on the server).
    ex(&mut s, "collab.stop", json!({}));
    assert_eq!(script.lock().unwrap().asked[2].3, ["edit1", "view1"]);
    assert_eq!(ex(&mut s, "collab.stop", json!({}))["ended"], true);
    {
        let sc = script.lock().unwrap();
        assert_eq!((sc.left.clone(), sc.asked.len()), (vec![1], 3));
    }
    let r = ex(&mut s, "collab.peers", json!({}));
    assert_eq!((&r["status"], &r["code"], &r["share"]), (&json!("off"), &Value::Null, &Value::Null));
}

#[test]
fn sharing_online_uses_a_separate_key_param() {
    // The Share popover sends the server key as its own field when the address (from Preferences,
    // which never holds a key) needs one. It must reach `publish` and not leak into the summary.
    let (mut s, script) = session();
    ex(&mut s, "pref.publishServer", json!({"url": "wss://share.example/"}));
    let r = ex(&mut s, "collab.share", json!({"key": "KeyKeyKeyKeyKeyKeyKey1"}));
    assert_eq!(r["status"], "connecting");
    {
        let sc = script.lock().unwrap();
        let [p] = sc.published.as_slice() else { panic!("published {}", sc.published.len()) };
        assert_eq!(p.server, "wss://share.example/");
        assert_eq!(p.key.as_deref(), Some("KeyKeyKeyKeyKeyKeyKey1"));
    }
    assert!(!r.to_string().contains("KeyKey"), "the server's key is nowhere in the summary");
}

#[test]
fn a_server_without_join_codes_leaves_the_links() {
    let (mut s, script) = session();
    ex(&mut s, "collab.share", json!({"via": "online", "server": "wss://old.example/?key=KeyKeyKeyKeyKeyKeyKey0", "name": "Poster"}));
    {
        let sc = script.lock().unwrap();
        assert_eq!((sc.published[0].server.as_str(), sc.published[0].name.as_str()), ("wss://old.example/?key=KeyKeyKeyKeyKeyKeyKey0", "Poster"));
    }
    on_the_server(&mut s, &script, None);
    let r = ex(&mut s, "collab.peers", json!({}));
    assert_eq!((&r["status"], &r["code"], &r["share"]["editCode"]), (&json!("live"), &Value::Null, &Value::Null));
    assert!(r["share"]["editLink"].as_str().unwrap().contains("token="), "the links are what there is to share");
    assert!(!r.to_string().contains("KeyKey"), "the server's key is nowhere in the summary");
    // Codes that aren't codes are not shown either.
    script.lock().unwrap().events.push(Event::Shared(Ok(links(0, Some(("<script>", "K7M2QX9TDX"))))));
    s.live_poll();
    let r = ex(&mut s, "collab.peers", json!({}));
    assert_eq!((&r["share"]["editCode"], &r["share"]["viewCode"], &r["share"]["shared"]), (&Value::Null, &Value::Null, &json!(true)));
    // Stopping works there too (links are older than codes); the operator's key goes with the
    // address, since there is no owner token.
    assert_eq!(ex(&mut s, "collab.stop", json!({}))["shared"], false);
    {
        let sc = script.lock().unwrap();
        assert_eq!(sc.asked[0].0, "wss://old.example/?key=KeyKeyKeyKeyKeyKeyKey0");
    }
    // `collab.leave` keeps the file on the server: nothing is asked of it.
    ex(&mut s, "collab.leave", json!({}));
    assert_eq!(script.lock().unwrap().asked.len(), 1);
    // A server that couldn't make the links: the file is there, not shared, and it says why.
    ex(&mut s, "collab.share", json!({"server": "wss://full.example/"}));
    script.lock().unwrap().events.extend([Event::Shared(Err("a file has at most 100 links: revoke one first".into()))]);
    on_the_server_unshared(&mut s, &script);
    assert!(s.message.as_deref().unwrap_or_default().contains("couldn't be shared: a file has at most 100 links"));
    assert_eq!(ex(&mut s, "collab.peers", json!({}))["share"]["shared"], false);
}

#[test]
fn online_sharing_is_off_without_a_server() {
    let (mut s, script) = session();
    ex(&mut s, "pref.publishServer", json!({"url": ""}));
    assert!(matches!(s.execute("collab.share", &json!({})), Err(EngineError::Disabled(_, why)) if why.contains("online sharing is off")));
    assert!(err(&mut s, "collab.join", json!({"code": "K7M-2QX-9TD"})).contains("online sharing is off"));
    // The local network needs no server, and a server named for one share is enough.
    ex(&mut s, "collab.share", json!({"server": "wss://share.example/"}));
    assert_eq!(script.lock().unwrap().published[0].server, "wss://share.example/");
    ex(&mut s, "collab.stop", json!({}));
    assert_eq!(ex(&mut s, "collab.share", json!({"via": "local"}))["status"], "live");
    // Without a link (the one-shot CLI) there is nothing to share with.
    let mut bare = Session::new();
    assert!(matches!(bare.execute("collab.share", &json!({"via": "local"})), Err(EngineError::Disabled(..))));
    assert!(matches!(bare.execute("collab.join", &json!({"code": HOST.code()})), Err(EngineError::Disabled(..))));
}

#[test]
fn sharing_on_the_local_network_shows_a_code_that_carries_the_address() {
    let (mut s, script) = session();
    let r = ex(&mut s, "collab.share", json!({"via": "local"}));
    assert_eq!((&r["status"], &r["role"], &r["share"]["via"]), (&json!("live"), &json!("host"), &json!("local")));
    let shown = r["code"].as_str().unwrap();
    assert_eq!(code::parse(shown), Ok(code::Code::Local(HOST)), "{shown}");
    assert_eq!(shown, code::grouped(&HOST.code()));
    assert_eq!((&r["link"], &r["share"]["viewCode"]), (&json!(HOST.link()), &Value::Null));
    {
        let sc = script.lock().unwrap();
        let [h] = sc.hosted.as_slice() else { panic!("hosted {}", sc.hosted.len()) };
        assert!(h.short && h.bind == Bind::Lan && h.port.is_none(), "the local network unless told otherwise");
    }
    // Everyone edits there.
    assert!(err(&mut s, "collab.share", json!({"access": "view"})).contains("share it online"));
    assert_eq!(ex(&mut s, "collab.stop", json!({}))["ended"], true);
    assert!(script.lock().unwrap().asked.is_empty(), "no server to tell");
    assert!(err(&mut s, "collab.share", json!({"via": "local", "access": "view"})).contains("share it online"));
    // Where to listen is the advanced part.
    ex(&mut s, "collab.share", json!({"via": "local", "bind": "local", "port": 5000}));
    let sc = script.lock().unwrap();
    assert!(sc.hosted[1].bind == Bind::Local && sc.hosted[1].port == Some(5000));
    drop(sc);
    ex(&mut s, "collab.stop", json!({}));
    for bad in [json!({"via": "post"}), json!({"via": "local", "port": 70000}), json!({"via": "local", "bind": "everywhere"})] {
        assert!(matches!(s.execute("collab.share", &bad), Err(EngineError::BadParams { .. })), "{bad}");
    }
    // A plain `collab.host` session has an invite link, not a code.
    let r = ex(&mut s, "collab.host", json!({}));
    assert_eq!((&r["code"], &r["share"]), (&Value::Null, &Value::Null));
    assert!(!script.lock().unwrap().hosted[2].short);
}

#[test]
fn joining_takes_a_code_of_either_kind_or_a_link() {
    let (mut s, script) = session();
    ex(&mut s, "pref.publishServer", json!({"url": "wss://share.example/"}));
    let asks = "figcraft://join?server=wss://share.example/&code=K7M2QX9TD";
    let local = HOST.link();
    let link = "figcraft://join?host=10.0.0.5:47900&file=abc&token=AbCdEfGhIjKlMnOpQrStUv";
    let shown = code::grouped(&HOST.code());
    let table: [(Value, &str); 8] = [
        (json!({"code": "K7M-2QX-9TD"}), asks),
        (json!({"code": "  k7m 2qx 9td \n"}), asks),
        (json!({"code": "k7m2qx9td"}), asks),
        (json!({"link": "K7M2QX9TD"}), asks),
        (json!({"code": shown}), &local),
        (json!({"code": shown.to_lowercase().replace('-', "")}), &local),
        (json!({"link": link}), link),
        // The one field takes either: a link where a code is expected.
        (json!({"code": format!("  {link} ")}), link),
    ];
    for (params, want) in table {
        let r = ex(&mut s, "collab.join", params.clone());
        assert_eq!(r["status"], "connecting", "{params}");
        assert_eq!(script.lock().unwrap().joined.last().map(String::as_str), Some(want), "{params}");
    }
    // O for 0 and I or L for 1 are the same code.
    assert_eq!(join_target("wss://s.example/", "O1l-iLo-0oO"), join_target("wss://s.example/", "011-110-000"));
    // What can't be a code says why, and nothing is asked of the network.
    let before = script.lock().unwrap().joined.len();
    for (typed, why) in [
        ("", "type the join code"),
        ("K7M-2QX", "9 characters"),
        ("K7M-2QX-9TDX", "that one has 10"),
        ("K7M-2QU-9TD", "no U"),
        ("hello world!", "only letters and digits"),
        (&shown.replace(&shown[..1], if shown.starts_with('Z') { "Y" } else { "Z" }), "typing mistake"),
    ] {
        let e = err(&mut s, "collab.join", json!({"code": typed}));
        assert!(e.contains(why), "{typed:?}: {e}");
    }
    assert!(err(&mut s, "collab.join", json!({})).contains("give the join `code`"));
    assert!(err(&mut s, "collab.join", json!({"code": 7})).contains("9 characters"));
    assert!(s.execute("collab.join", &json!({"code": {"x": 1}})).is_err());
    // Something that looks like a link goes to the link's reader.
    assert!(err(&mut s, "collab.join", json!({"code": "https://example.com/x"})).contains("not a live session invite"));
    assert_eq!(script.lock().unwrap().joined.len(), before);
    // A failed lookup shows on the tab it was started from (the link's event).
    ex(&mut s, "collab.join", json!({"code": "K7M-2QX-9TD"}));
    script.lock().unwrap().events.push(Event::Status(Status::Failed { reason: "No session with that code".into() }));
    s.live_poll();
    let r = ex(&mut s, "collab.peers", json!({}));
    assert_eq!((&r["status"], &r["reason"]), (&json!("failed"), &json!("No session with that code")));
    assert_eq!(s.message.as_deref(), Some("No session with that code"));
}
