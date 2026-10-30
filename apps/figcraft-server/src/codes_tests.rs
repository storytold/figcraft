//! Join codes on the server: sharing online from one app and joining by code from another (real
//! links over loopback; stopping revokes the others' links and codes and disconnects them while
//! the owner stays in the file, and sharing again makes new ones for the same file), codes kept
//! only hashed, revoked with their link and gone with their file,
//! the limits on wrong codes, junk requests, and clients from before join codes.

use std::net::IpAddr;
use std::time::{Duration, Instant};

use figcraft_collab::code;
use figcraft_collab::message::{Admin, AdminReply, ClientMsg, Hello, OpenHosting, PROTOCOL, Resolve, ServerMsg};
use figcraft_collab_net::hub::Backend;
use figcraft_engine::Session;
use figcraft_engine::live::Status;
use serde_json::{Value, json};

use crate::abuse::{Abuse, AbuseConfig, Action, CODE_LOCKOUT, CODES_PER_MINUTE, WRONG_CODES, WRONG_CODES_PER_HOUR};
use crate::codes::Key;
use crate::config::Config;
use crate::store::{CODE_GONE, CODES_BUSY, NO_CODE, Store};
use crate::test_util::{Raw, TempDir, config, ip, live, pump, session, snapshot};

fn open(dir: &std::path::Path) -> Config {
    Config { open_publish: true, operator_contact: "abuse@example.com".into(), terms_url: "https://example.com/terms".into(), ..config(dir) }
}

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

/// An app whose server for online sharing is the one at `url`.
fn app(name: &str, url: &str) -> Session {
    let mut s = session(name);
    ex(&mut s, "pref.publishServer", json!({"url": url}));
    s
}

fn layers(s: &Session) -> usize {
    s.doc().map_or(0, |d| d.node_count())
}

fn failed(s: &Session) -> Option<String> {
    match &s.live().status {
        Status::Failed { reason } => Some(reason.clone()),
        _ => None,
    }
}

fn is_code(shown: &str) -> bool {
    shown.len() == 11 && matches!(code::parse(shown), Ok(code::Code::Online(_)))
}

/// The links file keeps hashes only, never the codes or the token they wrap.
fn only_hashes_on_disk(dir: &std::path::Path, file: &str, wrapped: &str, edit: &str, view: &str) {
    let links = std::fs::read_to_string(dir.join("files").join(file).join("links.json")).unwrap();
    for secret in [edit, view, &edit.replace('-', ""), &view.replace('-', "")] {
        assert!(!links.contains(secret), "{links}");
    }
    assert!(!links.contains(wrapped.rsplit('=').next().unwrap()), "the wrapped token isn't the token");
    assert_eq!(links.matches("\"code\"").count(), 3);
}

#[test]
fn a_file_shared_online_is_joined_by_its_codes() {
    let dir = TempDir::new("codes-share");
    let mut server = crate::start(&open(dir.path())).unwrap();
    let url = format!("ws://{}/", server.addr());
    let mut a = app("Ada", &url);
    // Sharing: the file goes to the server in Preferences, and the server names its links.
    let started = ex(&mut a, "collab.share", json!({}));
    assert_eq!((&started["status"], &started["share"]["via"], &started["code"]), (&json!("connecting"), &json!("online"), &Value::Null));
    pump(&mut [&mut a], "the share", |s| live(s[0]) && s[0].live().published.is_some());
    let shared = a.live_summary();
    let (edit, view) = (shared["share"]["editCode"].as_str().unwrap().to_string(), shared["share"]["viewCode"].as_str().unwrap().to_string());
    assert!(is_code(&edit) && is_code(&view) && edit != view, "{edit} {view}");
    assert_eq!(shared["code"], edit.as_str(), "the code shown lets people edit unless asked otherwise");
    let p = a.live().published.clone().unwrap();
    assert!(p.owner.is_some(), "the owner token is kept as before");
    // Others get links of their own (edit, view and comment; the owner keeps the file's first
    // ones): five in all.
    let edit_link = shared["share"]["editLink"].as_str().unwrap().to_string();
    assert!(edit_link != p.edit && shared["share"]["viewLink"] != p.view.as_str() && shared["share"]["shared"] == true);
    let ids = |dir: &std::path::Path| -> Vec<String> {
        let links: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("files").join(&p.file).join("links.json")).unwrap()).unwrap();
        links.as_array().unwrap().iter().map(|l| l["id"].as_str().unwrap().to_string()).collect()
    };
    let before_stop = ids(dir.path());
    assert_eq!(before_stop.len(), 5);
    // On disk: hashes only, of the codes as of the links' tokens.
    only_hashes_on_disk(dir.path(), &p.file, &p.edit, &edit, &view);
    // Someone types the edit code, the way people type.
    let mut b = app("Bo", &url);
    assert_eq!(ex(&mut b, "collab.join", json!({"code": edit.to_lowercase().replace('-', " ")}))["status"], "connecting");
    pump(&mut [&mut a, &mut b], "Bo in", |s| live(s[1]) && s[0].live().peers.len() == 1);
    assert_eq!(b.live_summary()["access"], "edit");
    assert_eq!(b.live_summary()["link"], edit_link.as_str(), "joined as with the shared link: reconnects need no code");
    let before = layers(&a);
    ex(&mut a, "shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 30}));
    pump(&mut [&mut a, &mut b], "Ada's edit", |s| layers(s[1]) == before + 1);
    ex(&mut b, "shape.ellipse", json!({"x": 80, "y": 10, "width": 40, "height": 30}));
    pump(&mut [&mut a, &mut b], "Bo's edit", |s| layers(s[0]) == before + 2);
    // The view code lets someone look, not edit.
    assert_eq!(ex(&mut a, "collab.share", json!({"access": "view"}))["code"], view.as_str());
    let mut c = app("Cy", &url);
    ex(&mut c, "collab.join", json!({"code": view}));
    pump(&mut [&mut a, &mut b, &mut c], "Cy in", |s| live(s[2]) && layers(s[2]) == before + 2);
    assert_eq!((&c.live_summary()["access"], &c.live_summary()["readOnly"]), (&json!("view"), &json!(true)));
    assert!(c.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).is_err());
    // A code nobody was given.
    let mut d = app("Di", &url);
    ex(&mut d, "collab.join", json!({"code": "000-000-000"}));
    pump(&mut [&mut d], "the refusal", |s| failed(s[0]).is_some());
    assert_eq!(failed(&d).unwrap(), NO_CODE);
    // Stop sharing: the others are out at once and told why; the file and its owner stay.
    assert_eq!(ex(&mut a, "collab.stop", json!({})), json!({"ended": false, "shared": false}));
    pump(&mut [&mut a, &mut b, &mut c], "the others out", |s| {
        !s[1].live().status.active() && !s[2].live().status.active() && s[0].live().peers.is_empty()
    });
    for guest in [&b, &c] {
        assert!(guest.message.as_deref().unwrap_or_default().contains("Sharing was stopped"), "{:?}", guest.message);
        assert_eq!(guest.live().status, Status::Off, "over, not failed: nothing retries");
    }
    assert_eq!(layers(&b), before + 2, "Bo keeps the copy");
    let folder = dir.path().join("files").join(&p.file);
    assert!(folder.exists() && live(&a), "the file stays on the server and Ada stays in it");
    let stopped = a.live_summary();
    assert_eq!((&stopped["share"]["shared"], &stopped["code"], &stopped["published"]["file"]), (&json!(false), &Value::Null, &json!(p.file)));
    assert_eq!(ids(dir.path()), before_stop[..2], "the owner's links are left, the shared ones are gone");
    // Neither the codes nor the links get anyone in any more.
    let mut e = app("Ed", &url);
    for old in [&edit, &view] {
        ex(&mut e, "collab.join", json!({"code": old}));
        pump(&mut [&mut e], "the stopped code", |s| failed(s[0]).is_some());
        assert_eq!(failed(&e).unwrap(), CODE_GONE);
    }
    ex(&mut e, "collab.join", json!({"link": edit_link}));
    pump(&mut [&mut e], "the stopped link", |s| failed(s[0]).is_some_and(|why| why.contains("isn't valid")));
    // The owner still edits the server's file.
    ex(&mut a, "shape.rectangle", json!({"x": 200, "y": 10, "width": 40, "height": 30}));
    pump(&mut [&mut a], "the owner's edit confirmed", |s| layers(s[0]) == before + 3 && s[0].live().client().is_some_and(|c| c.pending() == 0));
    // Sharing again: new codes and links for the same file (nothing is published twice).
    let again = ex(&mut a, "collab.share", json!({}));
    let (edit2, link2) = (again["code"].as_str().unwrap().to_string(), again["share"]["editLink"].as_str().unwrap().to_string());
    assert!(is_code(&edit2) && edit2 != edit && link2 != edit_link && again["published"]["file"] == p.file.as_str(), "{again}");
    assert_eq!(dir.path().join("files").read_dir().unwrap().count(), 1, "still one file on the server");
    assert_eq!(ids(dir.path()).len(), 5);
    ex(&mut b, "collab.join", json!({"code": edit2}));
    pump(&mut [&mut a, &mut b], "Bo back in", |s| live(s[1]) && layers(s[1]) == before + 3 && s[0].live().peers.len() == 1);
    // The old code is still gone.
    ex(&mut e, "collab.join", json!({"code": edit}));
    pump(&mut [&mut e], "the old code", |s| failed(s[0]).is_some_and(|why| why == CODE_GONE));
    server.stop();
}

#[test]
fn a_keyed_share_gets_codes_and_stops_with_the_key() {
    let dir = TempDir::new("codes-keyed");
    let mut server = crate::start(&config(dir.path())).unwrap();
    let url = format!("ws://{}/", server.addr());
    let mut a = session("Ada");
    ex(&mut a, "collab.share", json!({"server": format!("{url}?key={}", crate::test_util::key(dir.path())), "name": "Team file"}));
    pump(&mut [&mut a], "the share", |s| live(s[0]) && s[0].live().published.is_some());
    let shown = a.live_summary()["code"].as_str().unwrap().to_string();
    assert!(is_code(&shown));
    assert!(!a.live_summary().to_string().contains("key="), "the key is nowhere in the summary");
    let mut b = app("Bo", &url);
    ex(&mut b, "collab.join", json!({"code": shown}));
    pump(&mut [&mut a, &mut b], "Bo in", |s| live(s[1]));
    // No owner token: the key the address carried stops the sharing (the file is the operator's).
    assert_eq!(ex(&mut a, "collab.stop", json!({}))["shared"], false);
    pump(&mut [&mut a, &mut b], "Bo out", |s| !s[1].live().status.active());
    assert!(live(&a) && !a.live_summary().to_string().contains("key="));
    ex(&mut b, "collab.join", json!({"code": shown}));
    pump(&mut [&mut b], "the stopped code", |s| failed(s[0]).is_some_and(|why| why == CODE_GONE));
    // Leaving keeps the file there.
    ex(&mut a, "collab.leave", json!({}));
    assert_eq!(dir.path().join("files").read_dir().unwrap().count(), 1);
    server.stop();
}

#[test]
fn codes_are_hashed_replaced_revoked_and_gone_with_the_file() {
    let dir = TempDir::new("codes-store");
    let cfg = open(dir.path());
    let me = ip("192.0.2.7");
    let (file, edit_token, view_link, shown, view_code) = {
        let mut store = Store::open(dir.path(), cfg.store()).unwrap();
        let (file, edit, view, _owner) = store.publish_open(me, "Poster", &snapshot("Poster")).unwrap();
        let shown = store.set_code(&file.id, &edit.token).unwrap();
        let view_code = store.set_code(&file.id, &view.token).unwrap();
        assert_ne!(shown, view_code, "a code per link");
        assert_eq!((shown.len(), code::normalize(&shown).as_deref()), (code::ONLINE_LEN, Ok(shown.as_str())));
        // The code gives the link's token back, and the token admits with the link's access.
        assert_eq!(store.resolve_code(me, &shown).unwrap(), (file.id.clone(), edit.token.clone()));
        let (f, t) = store.resolve_code(me, &code::grouped(&view_code).to_lowercase()).unwrap();
        assert_eq!(store.admit(&f, &t).unwrap().access, figcraft_collab::Access::View);
        // A token that isn't one of the file's links gets no code.
        assert!(store.set_code(&file.id, "AAAAAAAAAAAAAAAAAAAAAA").unwrap_err().contains("isn't one of this file's links"));
        assert!(store.set_code("0000000000000000", &edit.token).is_err());
        (file.id, edit.token, view.link.id, shown, view_code)
    };
    // What is kept: the stretched code's hash and the wrapped token, nothing that is either.
    let links = std::fs::read_to_string(dir.path().join("files").join(&file).join("links.json")).unwrap();
    assert!(!links.contains(&shown) && !links.contains(&edit_token), "{links}");
    assert!(links.contains(Key::new(&shown).hash()));
    // A restart keeps the codes (they are in the data directory).
    let mut store = Store::open(dir.path(), cfg.store()).unwrap();
    assert_eq!(store.resolve_code(me, &shown).unwrap().1, edit_token);
    // A new code for the link takes the old one's place.
    let newer = store.set_code(&file, &edit_token).unwrap();
    assert_ne!(newer, shown);
    assert_eq!(store.resolve_code(me, &shown).unwrap_err(), CODE_GONE);
    assert_eq!(store.resolve_code(me, &newer).unwrap().1, edit_token);
    // Revoking a link revokes its code.
    store.revoke(&file, &view_link).unwrap();
    assert_eq!(store.resolve_code(me, &view_code).unwrap_err(), CODE_GONE);
    // The file expiring takes the rest.
    store.age(&file, 31 * 86_400);
    assert_eq!(store.sweep(), vec![file.clone()]);
    assert_eq!(store.resolve_code(me, &newer).unwrap_err(), CODE_GONE);
    // A code never handed out is unknown, not expired.
    assert_eq!(store.resolve_code(ip("192.0.2.8"), "ZZZZZZZZZ").unwrap_err(), NO_CODE);
}

#[test]
fn wrong_codes_lock_an_address_out_but_not_the_others() {
    let dir = TempDir::new("codes-lockout");
    let mut store = Store::open(dir.path(), open(dir.path()).store()).unwrap();
    let (file, edit, _, _) = store.publish_open(ip("192.0.2.7"), "Poster", &snapshot("Poster")).unwrap();
    let shown = store.set_code(&file.id, &edit.token).unwrap();
    let guesser = ip("198.51.100.9");
    for n in 0..WRONG_CODES {
        assert_eq!(store.resolve_code(guesser, &code::online(u64::from(n) + 1)).unwrap_err(), NO_CODE, "guess {n}");
    }
    // Locked out: not even the right code is looked up for that address.
    for typed in [code::online(77), shown.clone()] {
        let e = store.resolve_code(guesser, &typed).unwrap_err();
        assert!(e.contains("Too many wrong codes") && e.contains("15 minutes") && e.contains("invite link"), "{e}");
    }
    // Its link still joins, and other addresses' codes still work.
    assert!(store.admit(&file.id, &edit.token).is_ok());
    assert_eq!(store.resolve_code(ip("198.51.100.10"), &shown).unwrap().0, file.id);
    // An IPv6 guesser is counted by its /64.
    for n in 0..WRONG_CODES {
        let from: IpAddr = format!("2001:db8:1:2::{n:x}").parse().unwrap();
        let _ = store.resolve_code(figcraft_collab_net::hub::ip_key(from), &code::online(1000 + u64::from(n)));
    }
    let neighbour = figcraft_collab_net::hub::ip_key(ip("2001:db8:1:2:ffff::1"));
    assert!(store.resolve_code(neighbour, &shown).unwrap_err().contains("Too many wrong codes"));
}

#[test]
fn the_limits_on_codes_over_time() {
    let t0 = Instant::now();
    let me = ip("198.51.100.9");
    let mut abuse = Abuse::new(AbuseConfig { open: false, connects_per_minute: 30, publishes_per_hour: 10, messages_per_second: 200 });
    // A right code, even from a neighbour in the same /64, doesn't forgive the wrong ones.
    let a = ip("2001:db8:1:2::1");
    let b = ip("2001:db8:1:2::2");
    for _ in 0..WRONG_CODES - 1 {
        abuse.code_tried(a, false, t0);
    }
    abuse.code_tried(b, true, t0);
    abuse.code_tried(a, true, t0);
    assert_eq!(abuse.code_locked(a, t0), None);
    abuse.code_tried(a, false, t0);
    assert_eq!(abuse.code_locked(a, t0), Some(CODE_LOCKOUT));
    // Wrong codes: locked for the lockout, then free again.
    for _ in 0..WRONG_CODES {
        abuse.code_tried(me, false, t0);
    }
    assert_eq!(abuse.code_locked(me, t0), Some(CODE_LOCKOUT));
    assert!(abuse.code_locked(me, t0 + CODE_LOCKOUT - Duration::from_secs(1)).is_some());
    assert_eq!(abuse.code_locked(me, t0 + CODE_LOCKOUT), None);
    // Wrong codes long apart don't add up.
    let other = ip("198.51.100.10");
    for n in 0..3 * WRONG_CODES {
        abuse.code_tried(other, false, t0 + (CODE_LOCKOUT + Duration::from_secs(1)) * n);
    }
    assert_eq!(abuse.code_locked(other, t0 + CODE_LOCKOUT * 4 * WRONG_CODES), None);
    // Codes tried a minute, right or wrong (these limits hold on private servers too).
    let typist = ip("198.51.100.11");
    let tried = (0..100).filter(|_| abuse.allow(typist, Action::Code, t0)).count();
    assert_eq!(tried as f64, CODES_PER_MINUTE);
    assert!(abuse.allow(typist, Action::Code, t0 + Duration::from_secs(7)), "one more every six seconds");
    // Everyone together: many addresses guessing pause codes for all (links still join).
    let mut abuse = Abuse::new(AbuseConfig { open: true, connects_per_minute: 30, publishes_per_hour: 10, messages_per_second: 200 });
    assert!(abuse.codes_open(t0));
    for n in 0..WRONG_CODES_PER_HOUR as u32 {
        abuse.code_tried(IpAddr::from([10, 1, (n >> 8) as u8, n as u8]), false, t0);
    }
    assert!(!abuse.codes_open(t0));
    assert!(abuse.codes_open(t0 + Duration::from_secs(60)), "it refills");
}

#[test]
fn everyone_guessing_pauses_codes() {
    let dir = TempDir::new("codes-busy");
    let mut store = Store::open(dir.path(), open(dir.path()).store()).unwrap();
    let (file, edit, _, _) = store.publish_open(ip("192.0.2.7"), "Poster", &snapshot("Poster")).unwrap();
    let shown = store.set_code(&file.id, &edit.token).unwrap();
    // Junk costs no hashing, so a thousand wrong "codes" from a thousand addresses are quick.
    for n in 0..WRONG_CODES_PER_HOUR as u32 {
        assert_eq!(store.resolve_code(IpAddr::from([10, 2, (n >> 8) as u8, n as u8]), "not a code").unwrap_err(), NO_CODE);
    }
    assert_eq!(store.resolve_code(ip("192.0.2.99"), &shown).unwrap_err(), CODES_BUSY);
    assert!(store.admit(&file.id, &edit.token).is_ok(), "links are not affected");
}

fn refused(raw: &mut Raw) -> String {
    match raw.next() {
        ServerMsg::Refused { reason } => reason,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn junk_requests_are_refused_and_never_hurt_the_server() {
    let dir = TempDir::new("codes-junk");
    let mut server = crate::start(&open(dir.path())).unwrap();
    let addr = server.addr();
    let url = format!("ws://{addr}/");
    let mut a = app("Ada", &url);
    ex(&mut a, "collab.share", json!({}));
    pump(&mut [&mut a], "the share", |s| live(s[0]) && s[0].live().published.is_some());
    let p = a.live().published.clone().unwrap();
    let ask = |code: &str, protocol: u32| {
        let mut raw = Raw::connect(addr);
        raw.send(&ClientMsg::Resolve(Resolve { protocol, client_version: String::new(), code: code.to_string() }));
        let why = refused(&mut raw);
        assert!(raw.closed_within(Duration::from_secs(5)), "the connection closes after the answer");
        why
    };
    // Junk in place of a code (each from this one address: after ten it is locked out).
    for junk in ["", " ", "💥💥💥", "K7M-2QX", "../../etc/passwd", "K7M2QX9TDK7M2QX9TD"] {
        assert_eq!(ask(junk, PROTOCOL), NO_CODE, "{junk:?}");
    }
    assert_eq!(ask(&"Z".repeat(4000), PROTOCOL), NO_CODE);
    assert!(ask("K7M2QX9TD", 1).contains("isn't supported"));
    // Asking for a code needs the file's owner token (or the key), and one of its link tokens.
    let mut raw = Raw::connect(addr);
    raw.send(&ClientMsg::OpenHosting(OpenHosting { protocol: PROTOCOL, client_version: String::new(), file: String::new(), owner: String::new() }));
    assert!(matches!(raw.next(), ServerMsg::Hosting(_)));
    raw.send(&ClientMsg::Admin(Admin::Code { file: p.file.clone(), token: "AAAAAAAAAAAAAAAAAAAAAA".into() }));
    assert!(matches!(raw.next(), ServerMsg::Admin(r) if matches!(*r, AdminReply::Failed { ref reason } if reason.contains("only its own"))));
    let mut raw = Raw::connect(addr);
    let owner = p.owner.clone().unwrap();
    raw.send(&ClientMsg::OpenHosting(OpenHosting { protocol: PROTOCOL, client_version: String::new(), file: p.file.clone(), owner }));
    assert!(matches!(raw.next(), ServerMsg::Hosting(_)));
    for token in ["", "AAAAAAAAAAAAAAAAAAAAAA", &"x".repeat(5000)] {
        raw.send(&ClientMsg::Admin(Admin::Code { file: p.file.clone(), token: token.to_string() }));
        assert!(
            matches!(raw.next(), ServerMsg::Admin(r) if matches!(*r, AdminReply::Failed { ref reason } if reason.contains("isn't one of this file's links")))
        );
    }
    // A code in place of a link's token joins nothing: codes only resolve.
    let mut raw = Raw::connect(addr);
    let shown = a.live_summary()["code"].as_str().unwrap().replace('-', "");
    let hello = Hello {
        protocol: PROTOCOL,
        file: p.file.clone(),
        token: shown,
        name: "Mal".into(),
        color: String::new(),
        client_version: String::new(),
        resume: None,
    };
    raw.send(&ClientMsg::Hello(hello));
    assert!(refused(&mut raw).contains("isn't valid"));
    // The session went on through all of it.
    assert_eq!(a.live_summary()["status"], "live");
    server.stop();
}

#[test]
fn clients_from_before_join_codes_are_served_as_before() {
    let dir = TempDir::new("codes-old-client");
    let mut server = crate::start(&open(dir.path())).unwrap();
    let url = format!("ws://{}/", server.addr());
    // An app that never asks for codes (`collab.publish`, what older apps do): links only, and
    // nothing about codes is kept for the file.
    let mut a = session("Ada");
    ex(&mut a, "collab.publish", json!({"server": url, "name": "Poster"}));
    pump(&mut [&mut a], "the publish", |s| live(s[0]) && s[0].live().published.is_some());
    let p = a.live().published.clone().unwrap();
    assert_eq!((&a.live_summary()["code"], &a.live_summary()["share"]), (&Value::Null, &Value::Null));
    let links = std::fs::read_to_string(dir.path().join("files").join(&p.file).join("links.json")).unwrap();
    assert!(!links.contains("\"code\""), "{links}");
    // Its links join as ever.
    let mut b = crate::test_util::join(&p.view, "Bo");
    pump(&mut [&mut a, &mut b], "Bo in", |s| live(s[1]));
    server.stop();
    // A data directory from before join codes (links without the field) opens unchanged.
    let old = r#"[{"id":"0123456789abcdef","access":"edit","label":"Published (edit)","created":1,"hash":"00"}]"#;
    let path = dir.path().join("files").join(&p.file).join("links.json");
    std::fs::write(&path, old).unwrap();
    let mut store = Store::open(dir.path(), open(dir.path()).store()).unwrap();
    assert_eq!(store.links(&p.file).unwrap().len(), 1);
    assert_eq!(store.resolve_code(ip("192.0.2.1"), "K7M2QX9TD").unwrap_err(), NO_CODE);
}
