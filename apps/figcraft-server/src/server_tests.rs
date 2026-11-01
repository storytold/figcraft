//! The server end to end over loopback, in-process with a temporary data directory: an app
//! publishes its file and syncs through the server, others join with the edit link and the view
//! link, edits converge and the viewer's are refused (by its app, and by the server for a client
//! that skips the check), the server restarts mid-session and everyone resumes without losing a
//! confirmed edit, revoked links stop working at once, and fuzzed frames don't hurt it.

use std::net::TcpStream;
use std::time::Duration;

use figcraft_collab::message::{self, Access, Admin, AdminReply, ClientMsg, ServerMsg};
use figcraft_collab::sim::Rng;
use figcraft_collab::{Client, VIEW_ONLY};
use figcraft_collab_net::{Endpoint, Invite, NetLimits, manage};
use figcraft_doc::NodeId;
use figcraft_engine::live::Status;
use serde_json::{Value, json};
use tungstenite::{Bytes, Message};

use crate::test_util::{Raw, TempDir, config, join, key, live, publish, pump, same, session};

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

#[test]
fn publish_then_edit_and_view_together() {
    let dir = TempDir::new("share");
    let mut server = crate::start(&config(dir.path())).unwrap();
    let mut a = session("Ada");
    a.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 80, "height": 80})).unwrap();
    let addr = server.addr();
    let r = a.execute("collab.publish", &json!({"server": format!("ws://{addr}/?key={}", key(dir.path()))})).unwrap();
    assert_eq!(r["status"], "connecting");
    // Nothing changes it until it is on the server.
    let e = a.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap_err().to_string();
    assert!(e.contains("being published"), "{e}");
    pump(&mut [&mut a], "the publish", |s| live(s[0]) && s[0].live().published.is_some());
    let p = a.live().published.clone().unwrap();
    assert_eq!(a.docs.len(), 1, "the open tab syncs; no new tab");
    assert_eq!(a.live_summary()["readOnly"], false);
    assert!(p.edit.starts_with("figcraft://join?host=127.0.0.1:") && p.view.starts_with("figcraft://join?host=127.0.0.1:"));
    assert_ne!(p.edit, p.view);
    // Bo joins with the edit link, Cy with the view link.
    let mut b = join(&p.edit, "Bo");
    let mut c = join(&p.view, "Cy");
    pump(&mut [&mut a, &mut b, &mut c], "everyone in", |s| s.iter().all(|x| live(x)) && s[0].live().peers.len() == 2);
    assert_eq!(b.live_summary()["access"], "edit");
    assert_eq!(c.live_summary()["access"], "view");
    assert_eq!(c.live_summary()["readOnly"], true);
    assert!(same(a.doc().unwrap(), b.doc().unwrap()) && same(a.doc().unwrap(), c.doc().unwrap()));
    // Ada and Bo edit; everyone converges, Cy included.
    let rect = id_of(&a.execute("shape.rectangle", &json!({"x": 200, "y": 0, "width": 40, "height": 40})).unwrap());
    pump(&mut [&mut a, &mut b, &mut c], "Ada's rectangle at Bo", |s| s[1].doc().unwrap().contains(rect));
    b.execute("node.set", &json!({"ids": [rect.0], "props": {"name": "Card"}})).unwrap();
    pump(&mut [&mut a, &mut b, &mut c], "everyone agreeing", |s| {
        let (ad, bd, cd) = (s[0].doc().unwrap(), s[1].doc().unwrap(), s[2].doc().unwrap());
        same(ad, bd) && same(ad, cd) && cd.node(rect).is_some_and(|n| n.name == "Card")
    });
    // Cy can't change anything: commands and gestures are refused before anything is sent.
    let before = c.doc().unwrap().clone();
    let e = c.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap_err().to_string();
    assert!(e.contains("View only"), "{e}");
    assert!(c.execute("node.set", &json!({"ids": [rect.0], "props": {"name": "Mine"}})).is_err());
    assert!(c.begin_interaction("Move").is_err());
    assert!(same(&before, c.doc().unwrap()));
    // Cy is still seen: presence goes both ways.
    let cy = c.live().info.as_ref().unwrap().me;
    c.execute("collab.cursor", &json!({"x": 12, "y": 34})).unwrap();
    pump(&mut [&mut a, &mut b, &mut c], "Cy's cursor at Ada", |s| s[0].live().presence.get(&cy).and_then(|p| p.cursor).is_some_and(|c| c.1 == 12.0));
    // A client that skips the app's check is refused by the server.
    let view = Invite::parse(&p.view).unwrap();
    let mut raw = Raw::connect(addr);
    raw.send(&Client::hello(&view.file, &view.token, "Raw", ""));
    let welcome = loop {
        if let w @ ServerMsg::Welcome { .. } = raw.next() {
            break w;
        }
    };
    let mut client = Client::join(welcome).unwrap();
    assert_eq!(client.access(), Access::View);
    let mut d = client.document().clone();
    d.node_mut(rect).unwrap().name = "Sneaky".into();
    raw.send(&client.edit(&d).unwrap().unwrap());
    loop {
        if let ServerMsg::Reject { reason, .. } = raw.next() {
            assert_eq!(reason, VIEW_ONLY);
            break;
        }
    }
    pump(&mut [&mut a], "nothing changed", |s| s[0].doc().unwrap().node(rect).is_some_and(|n| n.name == "Card"));
    // Leaving keeps a local copy.
    c.execute("collab.leave", &json!({})).unwrap();
    assert_eq!(c.live().status, Status::Off);
    assert!(c.doc().unwrap().contains(rect));
    assert_eq!(c.live_summary()["readOnly"], false, "the copy is yours to edit");
    c.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
    server.stop();
}

#[test]
fn a_restart_mid_session_loses_no_confirmed_edit() {
    let dir = TempDir::new("restart");
    let mut cfg = config(dir.path());
    let mut server = crate::start(&cfg).unwrap();
    let addr = server.addr();
    let mut a = session("Ada");
    let p = publish(&mut a, dir.path(), addr);
    let mut b = join(&p.edit, "Bo");
    pump(&mut [&mut a, &mut b], "Bo in", |s| live(s[1]) && s[0].live().peers.len() == 1);
    let rect = id_of(&a.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap());
    pump(&mut [&mut a, &mut b], "the rectangle confirmed", |s| s[1].doc().unwrap().contains(rect));
    let (ada, bo) = (a.live().info.as_ref().unwrap().me, b.live().info.as_ref().unwrap().me);
    // The server stops: both reconnect, and what Ada makes meanwhile waits.
    server.stop();
    pump(&mut [&mut a, &mut b], "the drop", |s| s.iter().all(|x| matches!(x.live().status, Status::Reconnecting { .. })));
    let offline = id_of(&a.execute("shape.ellipse", &json!({"x": 100, "y": 10, "width": 30, "height": 30})).unwrap());
    cfg.listen = addr.to_string();
    let mut server = start_again(&cfg);
    pump(&mut [&mut a, &mut b], "everyone back with everything", |s| {
        s.iter().all(|x| live(x)) && s[1].doc().unwrap().contains(offline) && s[1].doc().unwrap().contains(rect)
    });
    assert_eq!(a.live().info.as_ref().unwrap().me, ada, "Ada resumed as herself");
    assert_eq!(b.live().info.as_ref().unwrap().me, bo, "so did Bo");
    assert!(same(a.doc().unwrap(), b.doc().unwrap()));
    // Someone new gets it all from the restarted server.
    let mut c = join(&p.view, "Cy");
    pump(&mut [&mut a, &mut b, &mut c], "Cy in", |s| live(s[2]));
    assert!(same(a.doc().unwrap(), c.doc().unwrap()));
    server.stop();
}

/// Start a server on `cfg.listen` again (the port may take a moment to come free).
fn start_again(cfg: &crate::config::Config) -> crate::Running {
    let start = std::time::Instant::now();
    loop {
        match crate::start(cfg) {
            Ok(s) => return s,
            Err(e) if start.elapsed() < Duration::from_secs(10) && e.contains("can't listen") => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => panic!("{e}"),
        }
    }
}

#[test]
fn revoked_links_stop_working_at_once() {
    let dir = TempDir::new("revoke");
    let mut server = crate::start(&config(dir.path())).unwrap();
    let addr = server.addr();
    let mut a = session("Ada");
    let p = publish(&mut a, dir.path(), addr);
    let file = Invite::parse(&p.edit).unwrap().file;
    let endpoint = Endpoint::plain(addr.to_string());
    let ask = |req: Admin| manage::request(&endpoint, &key(dir.path()), vec![req], &NetLimits::default(), Duration::from_secs(5)).unwrap().remove(0);
    let AdminReply::Link(review) = ask(Admin::CreateLink { file: file.clone(), access: Access::View, label: "Review".into() }) else { panic!() };
    let link = Invite { server: endpoint.clone(), file: file.clone(), token: review.token.clone() }.link();
    let mut b = join(&p.edit, "Bo");
    let mut c = join(&link, "Cy");
    pump(&mut [&mut a, &mut b, &mut c], "everyone in", |s| s.iter().all(|x| live(x)));
    let AdminReply::Files(files) = ask(Admin::Files) else { panic!() };
    assert_eq!((files.len(), files[0].people), (1, 3));
    // Revoked: Cy is out at once, the others stay.
    assert_eq!(ask(Admin::Revoke { file: file.clone(), link: review.link.id.clone() }), AdminReply::Done);
    pump(&mut [&mut a, &mut b, &mut c], "Cy out", |s| s[2].live().status == Status::Off && s[0].live().peers.len() == 1);
    // Told as when a host stops: the session is over for Cy, whose copy stays open (no retries).
    assert!(c.message.as_deref().unwrap_or_default().contains("Sharing was stopped"), "{:?}", c.message);
    assert!(live(&b));
    // The link no longer joins.
    c.execute("collab.join", &json!({"link": link})).unwrap();
    pump(&mut [&mut c], "the refusal", |s| matches!(&s[0].live().status, Status::Failed { reason } if reason.contains("isn't valid")));
    // Managing needs the key.
    let e = manage::request(&endpoint, "WrongWrongWrongWrong00", vec![Admin::Files], &NetLimits::default(), Duration::from_secs(5)).unwrap_err();
    assert!(e.contains("wrong server key"), "{e}");
    // Removing the file ends it for everyone in it.
    assert_eq!(ask(Admin::Remove { file: file.clone() }), AdminReply::Done);
    pump(&mut [&mut a, &mut b], "everyone out", |s| {
        s.iter().all(|x| matches!(&x.live().status, Status::Failed { reason } if reason.contains("removed")))
    });
    server.stop();
}

#[test]
fn fuzzed_frames_dont_hurt_the_server() {
    let dir = TempDir::new("fuzz");
    // Open to the public, so publishing without the key and owner tokens are fuzzed too (with
    // rates high enough that the noise gets in).
    let cfg = crate::config::Config { open_publish: true, connects_per_minute: 100_000, messages_per_second: 100_000, ..config(dir.path()) };
    let mut server = crate::start(&cfg).unwrap();
    let addr = server.addr();
    let mut a = session("Ada");
    let p = publish(&mut a, dir.path(), addr);
    let edit = Invite::parse(&p.edit).unwrap();
    let valid: Vec<Vec<u8>> = [
        Client::hello(&edit.file, &edit.token, "Fuzz", ""),
        ClientMsg::Manage(message::Manage { protocol: message::PROTOCOL, key: key(dir.path()), client_version: String::new() }),
        ClientMsg::Admin(Admin::Links { file: "0000000000000000".into() }),
        ClientMsg::OpenLibrary(message::OpenLibrary {
            protocol: message::PROTOCOL,
            file: edit.file.clone(),
            token: edit.token.clone(),
            client_version: String::new(),
        }),
        ClientMsg::Library(message::LibraryReq::Fetch {
            have: None,
            usage: Some(message::Usage { file: "Fuzz".into(), components: vec![(NodeId(1), 2)] }),
        }),
        ClientMsg::OpenPlugins(message::OpenPlugins { protocol: message::PROTOCOL, key: String::new(), client_version: String::new() }),
        ClientMsg::Plugins(message::PluginReq::List { query: "tiny".into() }),
        ClientMsg::OpenHosting(message::OpenHosting {
            protocol: message::PROTOCOL,
            client_version: String::new(),
            file: String::new(),
            owner: String::new(),
        }),
        ClientMsg::OpenHosting(message::OpenHosting {
            protocol: message::PROTOCOL,
            client_version: String::new(),
            file: edit.file.clone(),
            owner: "AAAAAAAAAAAAAAAAAAAAAA".into(),
        }),
        ClientMsg::Admin(Admin::Publish {
            name: "Fuzz".into(),
            snapshot: figcraft_collab::Replica::new(figcraft_doc::Document::new("F")).snapshot().unwrap(),
        }),
        ClientMsg::Admin(Admin::Rename { file: edit.file.clone(), name: "Fuzzed".into() }),
        ClientMsg::Admin(Admin::Listing),
        ClientMsg::Admin(Admin::Block { ip: "203.0.113.1".into() }),
        ClientMsg::Report { reason: "fuzz".into() },
        ClientMsg::Plugins(message::PluginReq::Fetch { id: "org.example.tiny".into(), version: None }),
        ClientMsg::Plugins(message::PluginReq::Publish {
            bundle: figcraft_plugin::bundle::Bundle { manifest: "{}".into(), wasm: b"\0asm".to_vec(), assets: Vec::new() }.encode(),
        }),
        ClientMsg::Ping,
    ]
    .iter()
    .map(|m| message::encode(m).unwrap())
    .collect();
    let mut rng = Rng::new(11);
    for round in 0..80 {
        let Ok(s) = TcpStream::connect(addr) else { continue };
        let _ = s.set_read_timeout(Some(Duration::from_millis(200)));
        let Ok((mut ws, _)) = tungstenite::client::client(format!("ws://{addr}/"), s) else { continue };
        for _ in 0..1 + rng.below(3) {
            let mut bytes = rng.pick(&valid).cloned().unwrap_or_default();
            if round % 5 == 0 {
                bytes = vec![0x5A; 100_000];
            }
            for _ in 0..1 + rng.below(4) {
                if bytes.is_empty() {
                    break;
                }
                let i = rng.below(bytes.len());
                match rng.below(3) {
                    0 => bytes[i] = rng.next_u64() as u8,
                    1 => bytes.insert(i, rng.next_u64() as u8),
                    _ => bytes.truncate(i.max(1)),
                }
            }
            let frame =
                if round % 7 == 0 { Message::Text(String::from_utf8_lossy(&bytes).into_owned().into()) } else { Message::Binary(Bytes::from(bytes)) };
            if ws.send(frame).is_err() {
                break;
            }
        }
        // read() returns Result<Message>, not a lock guard; the fuzz loop ends here on purpose and
        // the close frame from the server is not interesting.
        let _ = ws.read(); // NOSONAR
    }
    // Still serving, the file intact.
    let mut b = join(&p.edit, "Bo");
    pump(&mut [&mut a, &mut b], "Bo in after the noise", |s| live(s[1]) && live(s[0]));
    assert!(same(a.doc().unwrap(), b.doc().unwrap()));
    server.stop();
}
