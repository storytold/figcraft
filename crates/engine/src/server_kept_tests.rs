//! A file joined on a server that keeps it has nothing to save on this computer once the server
//! confirmed all of it: no "Unsaved changes" question when its tab closes or the app quits.

use std::sync::{Arc, Mutex};

use figcraft_collab::message::{ClientMsg, HostInfo, ServerMsg};
use figcraft_collab::{Client, Limits, Outgoing, Server};
use figcraft_doc::Document;
use serde_json::json;

use crate::Session;
use crate::live::{Event, Role};
use crate::live_tests::{Script, info, peer, push, relay, session};

/// You joined "Team file" on a server as a guest: the session, its server and your client id.
fn joined() -> (Session, Arc<Mutex<Script>>, Server, u32) {
    let (mut s, script) = session();
    let mut doc = Document::new("Team file");
    doc.repair();
    let mut server = Server::new(doc, "", Limits::default());
    let ClientMsg::Hello(h) = Client::hello("f", "", "Me", "") else { panic!("no hello") };
    let (me, outs) = server.join(h).unwrap();
    let Some(Outgoing::To(_, w)) = outs.into_iter().find(|o| matches!(o, Outgoing::To(_, ServerMsg::Welcome { .. }))) else { panic!("no welcome") };
    s.execute("collab.join", &json!({"link": "figcraft://join?host=a:1&file=f&token=t"})).unwrap();
    push(&script, vec![Event::Welcome { info: info(Role::Guest, me), peers: vec![peer(1, "Hana")] }, Event::Sync(w)]);
    s.live_poll();
    assert_eq!(s.doc().unwrap().name, "Team file");
    (s, script, server, me)
}

/// Draw a rectangle and let the server confirm it.
fn edit_and_confirm(s: &mut Session, script: &Arc<Mutex<Script>>, server: &mut Server, me: u32) {
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    s.live_sync();
    assert!(s.ds().unwrap().is_dirty(), "an edit the server hasn't confirmed is unsaved");
    push(script, relay(script, server, me));
    s.live_poll();
}

#[test]
fn a_file_the_server_keeps_has_nothing_to_save_once_confirmed() {
    let (mut s, script, mut server, me) = joined();
    push(&script, vec![Event::Hosting(HostInfo::default())]);
    s.live_poll();
    assert!(!s.ds().unwrap().is_dirty(), "joining a kept file asks for no save");
    edit_and_confirm(&mut s, &script, &mut server, me);
    assert!(!s.ds().unwrap().is_dirty(), "the server has every edit");
    // Someone else's edit is on the server too.
    let ClientMsg::Hello(h) = Client::hello("f", "", "Ada", "") else { panic!("no hello") };
    let (ada, outs) = server.join(h).unwrap();
    let Some(Outgoing::To(_, w)) = outs.into_iter().find(|o| matches!(o, Outgoing::To(_, ServerMsg::Welcome { .. }))) else { panic!("no welcome") };
    let mut other = Client::join(w).unwrap();
    let mut doc = other.document().clone();
    doc.name = "Renamed".into();
    let batch = other.edit(&doc).unwrap().unwrap();
    let remote: Vec<Event> =
        server.receive(ada, batch).into_iter().filter_map(|o| if let Outgoing::All(m) = o { Some(Event::Sync(m)) } else { None }).collect();
    push(&script, remote);
    s.live_poll();
    assert_eq!(s.doc().unwrap().name, "Renamed");
    assert!(!s.ds().unwrap().is_dirty());
}

#[test]
fn a_session_an_app_hosts_still_asks() {
    // No hosting info: an app hosts the session, and nothing keeps the file once it leaves.
    let (mut s, script, mut server, me) = joined();
    edit_and_confirm(&mut s, &script, &mut server, me);
    assert!(s.ds().unwrap().is_dirty());
}

#[test]
fn a_file_that_expires_unopened_still_asks() {
    let (mut s, script, mut server, me) = joined();
    push(&script, vec![Event::Hosting(HostInfo { open: true, expires: Some(4_102_444_800), ..HostInfo::default() })]);
    s.live_poll();
    edit_and_confirm(&mut s, &script, &mut server, me);
    assert!(s.ds().unwrap().is_dirty(), "a public server keeps no backups: keep a copy");
}

#[test]
fn a_tab_with_a_file_of_its_own_still_asks() {
    let (mut s, script, mut server, me) = joined();
    push(&script, vec![Event::Hosting(HostInfo::default())]);
    s.live_poll();
    s.ds_mut().unwrap().path = Some("team.figcraft".into());
    edit_and_confirm(&mut s, &script, &mut server, me);
    assert!(s.ds().unwrap().is_dirty(), "the file on this computer is behind");
}
