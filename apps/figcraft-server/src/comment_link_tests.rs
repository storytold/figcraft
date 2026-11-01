//! Comment links end to end over loopback: sharing online makes a comment link and code beside the
//! edit and view ones; someone who joins with it comments and everyone sees it, their app refuses
//! their other edits, and the server refuses them too for a client that skips the check; a client
//! that can't read comment links gets in as a viewer, and links listed to it read as view links.

use std::time::Duration;

use figcraft_collab::message::{Access, Admin, AdminReply, ClientMsg, Manage, PROTOCOL, ServerMsg};
use figcraft_collab::{COMMENT_ONLY, Change, Client, Entry, Json};
use figcraft_collab_net::{Endpoint, Invite, NetLimits, manage};
use figcraft_doc::{Comment, Message};
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::test_util::{Raw, TempDir, config, join, key, live, publish, pump, session};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

/// The welcome `raw` gets after saying hello with `client_version` through `link`.
fn welcome(raw: &mut Raw, link: &str, client_version: &str) -> Client {
    let invite = Invite::parse(link).unwrap();
    let ClientMsg::Hello(mut h) = Client::hello(&invite.file, &invite.token, "Raw", "") else { panic!("not a hello") };
    h.client_version = client_version.into();
    raw.send(&ClientMsg::Hello(h));
    loop {
        if let w @ ServerMsg::Welcome { .. } = raw.next() {
            return Client::join(w).unwrap();
        }
    }
}

/// What the server says to the batch `raw` sends next.
fn answer(raw: &mut Raw) -> ServerMsg {
    loop {
        if let m @ (ServerMsg::Reject { .. } | ServerMsg::Changes(_)) = raw.next() {
            return m;
        }
    }
}

#[test]
fn a_comment_link_comments_and_the_server_refuses_its_edits() {
    let dir = TempDir::new("comment-link");
    let mut server = crate::start(&config(dir.path())).unwrap();
    let addr = server.addr();
    let mut a = session("Ada");
    ex(&mut a, "shape.rectangle", json!({"x": 0, "y": 0, "width": 80, "height": 80}));
    let p = publish(&mut a, dir.path(), addr);
    let file = Invite::parse(&p.edit).unwrap().file;
    let endpoint = Endpoint::plain(addr.to_string());
    let ask = |req: Admin| manage::request(&endpoint, &key(dir.path()), vec![req], &NetLimits::default(), Duration::from_secs(5)).unwrap().remove(0);
    let AdminReply::Link(made) = ask(Admin::CreateLink { file: file.clone(), access: Access::Comment, label: "Client".into() }) else { panic!() };
    assert_eq!(made.link.access, Access::Comment);
    let link = Invite { server: endpoint.clone(), file: file.clone(), token: made.token.clone() }.link();
    // Cy joins with it and comments; Ada sees the thread.
    let mut c = join(&link, "Cy");
    pump(&mut [&mut a, &mut c], "Cy in", |s| live(s[1]) && s[0].live().peers.len() == 1);
    assert_eq!((&c.live_summary()["access"], &c.live_summary()["readOnly"]), (&json!("comment"), &json!(true)));
    let thread = ex(&mut c, "comment.add", json!({"x": 10, "y": 10, "text": "Bigger logo?"}))["id"].as_u64().unwrap();
    pump(&mut [&mut a, &mut c], "the thread at Ada", |s| s[0].doc().unwrap().comment(thread).is_some());
    ex(&mut a, "comment.reply", json!({"id": thread, "text": "On it"}));
    pump(&mut [&mut a, &mut c], "the reply at Cy", |s| s[1].doc().unwrap().comment(thread).is_some_and(|t| t.replies.len() == 1));
    ex(&mut c, "comment.resolve", json!({"id": thread}));
    pump(&mut [&mut a, &mut c], "resolved at Ada", |s| s[0].doc().unwrap().comment(thread).is_some_and(|t| t.resolved));
    // Cy's app refuses anything else before it is sent.
    let e = c.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap_err().to_string();
    assert!(e.contains("Can comment"), "{e}");
    // A client that skips the app's check: its comment goes through, its edit is refused.
    let mut raw = Raw::connect(addr);
    let mut client = welcome(&mut raw, &link, &figcraft_collab::message::client_version());
    assert_eq!(client.access(), Access::Comment);
    let mut d = client.document().clone();
    let page = d.first_page().unwrap();
    let id = d.new_comment_id();
    d.comments.push(Comment {
        id,
        page,
        x: 1.0,
        y: 1.0,
        anchor: None,
        first: Message::new("Raw", "Hi", 0),
        resolved: false,
        replies: vec![],
        read_by: None,
    });
    raw.send(&client.edit(&d).unwrap().unwrap());
    let ServerMsg::Changes(acc) = answer(&mut raw) else { panic!("refused") };
    client.receive(ServerMsg::Changes(acc), &figcraft_collab::NoRecompute).unwrap();
    let mut d = client.document().clone();
    d.node_mut(page).unwrap().name = "Sneaky".into();
    raw.send(&client.edit(&d).unwrap().unwrap());
    assert!(matches!(answer(&mut raw), ServerMsg::Reject { reason, .. } if reason == COMMENT_ONLY));
    // So is a forged batch that hides an edit among comment changes.
    let forged = Change::Put { entry: Entry::Comment(id), value: Json::of(&client.document().comments[0]).unwrap() };
    let rename = Change::SetDoc { prop: figcraft_collab::DocProp::Name, value: Json::of("Sneaky").unwrap() };
    raw.send(&ClientMsg::Changes { base_seq: 0, batch_id: 99, changes: vec![forged, rename] });
    assert!(matches!(answer(&mut raw), ServerMsg::Reject { reason, .. } if reason == COMMENT_ONLY));
    pump(&mut [&mut a], "the raw comment, nothing else", |s| {
        let d = s[0].doc().unwrap();
        d.comment(id).is_some() && d.node(page).is_some_and(|n| n.name != "Sneaky") && d.name != "Sneaky"
    });
    // A client from before comment links gets in as a viewer.
    let mut old = Raw::connect(addr);
    assert_eq!(welcome(&mut old, &link, "0.12.1 signals hosting").access(), Access::View);
    // And lists the link as a view link.
    let mut m = Raw::connect(addr);
    m.send(&ClientMsg::Manage(Manage { protocol: PROTOCOL, key: key(dir.path()), client_version: "0.12.1".into() }));
    assert!(matches!(m.next(), ServerMsg::Admin(r) if *r == AdminReply::Ready));
    m.send(&ClientMsg::Admin(Admin::Links { file: file.clone() }));
    let ServerMsg::Admin(r) = m.next() else { panic!("no listing") };
    let AdminReply::Links(links) = *r else { panic!("{r:?}") };
    assert!(links.iter().all(|l| l.access != Access::Comment) && links.iter().any(|l| l.id == made.link.id && l.access == Access::View));
    let AdminReply::Links(links) = ask(Admin::Links { file }) else { panic!() };
    assert!(links.iter().any(|l| l.id == made.link.id && l.access == Access::Comment), "this build reads it");
    server.stop();
}

#[test]
fn sharing_online_offers_a_comment_code() {
    let dir = TempDir::new("comment-code");
    let mut server = crate::start(&config(dir.path())).unwrap();
    let url = format!("ws://{}/?key={}", server.addr(), key(dir.path()));
    let mut a = session("Ada");
    ex(&mut a, "collab.share", json!({"server": url}));
    pump(&mut [&mut a], "the share", |s| live(s[0]) && s[0].live().published.is_some() && s[0].live_summary()["share"]["shared"] == true);
    let shared = a.live_summary()["share"].clone();
    let comment = shared["commentCode"].as_str().unwrap().to_string();
    assert!(comment != shared["editCode"] && comment != shared["viewCode"], "{shared}");
    assert_eq!(ex(&mut a, "collab.share", json!({"access": "comment"}))["code"], comment.as_str());
    // Someone types it: they can comment, not edit.
    let mut c = session("Cy");
    ex(&mut c, "pref.publishServer", json!({"url": format!("ws://{}/", server.addr())}));
    ex(&mut c, "collab.join", json!({"code": comment}));
    pump(&mut [&mut a, &mut c], "Cy in", |s| live(s[1]) && s[0].live().peers.len() == 1);
    assert_eq!(c.live_summary()["access"], "comment");
    let thread = ex(&mut c, "comment.add", json!({"x": 10, "y": 10, "text": "Looks good"}))["id"].as_u64().unwrap();
    pump(&mut [&mut a, &mut c], "the thread at Ada", |s| s[0].doc().unwrap().comment(thread).is_some());
    assert!(c.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).is_err());
    // Stopping revokes it with the others: Cy is out.
    ex(&mut a, "collab.stop", json!({}));
    pump(&mut [&mut a, &mut c], "Cy out", |s| !s[1].live().status.active() && s[0].live().peers.is_empty());
    server.stop();
}
