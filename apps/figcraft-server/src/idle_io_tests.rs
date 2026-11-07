//! Joining and leaving a file nobody is editing costs no disk I/O while it is kept loaded
//! (#1074), and what changes is still saved as before.

use std::fs;

use figcraft_collab::message::{Access, ClientMsg, Hello};
use figcraft_collab::{Client, NoRecompute, Outgoing, Replica, Server};
use figcraft_collab_net::hub::Backend;
use figcraft_doc::Document;

use crate::disk::{self, META};
use crate::store::{FILES, Store, StoreConfig};
use crate::test_util::{TempDir, config};

fn hello() -> Hello {
    let ClientMsg::Hello(h) = Client::hello("f", "", "Ada", "#FF8800") else { panic!() };
    h
}

/// Let `hello` into `server` and hand its messages to `client` (a new one when `None`); the
/// client and its id there.
fn enter(server: &mut Server, client: Option<Client>, hello: Hello) -> (Client, u32) {
    let (id, out) = server.join_as(hello, Access::Edit).unwrap();
    let mut msgs = out.into_iter().filter_map(|o| match o {
        Outgoing::To(to, m) if to == id => Some(m),
        _ => None,
    });
    let client = match client {
        Some(mut c) => {
            for m in msgs {
                c.receive(m, &NoRecompute).unwrap();
            }
            c
        }
        None => Client::join(msgs.next().unwrap()).unwrap(),
    };
    (client, id)
}

/// A file opened, joined and left once: the store, the file's id and the client, ready to resume.
fn published(dir: &TempDir) -> (Store, String, Client, StoreConfig) {
    let c = StoreConfig { compact_after: 1000, ..config(dir.path()).store() };
    let mut store = Store::open(dir.path(), c.clone()).unwrap();
    let d = Document::new("Idle");
    let (info, _, _) = store.publish("Idle", &Replica::new(d).snapshot().unwrap()).unwrap();
    let id = info.id;
    let mut server = store.open(&id).unwrap();
    let (client, cid) = enter(&mut server, None, hello());
    server.leave(cid);
    store.changed(&id, &mut server);
    store.flush();
    assert!(store.idle(&id, &mut server));
    (store, id, client, c)
}

/// One rejoin and leave, as the hub drives it; `edit` renames the page on the way.
fn cycle(store: &mut Store, id: &str, client: Client, edit: Option<&str>) -> Client {
    let mut server = store.open(id).unwrap();
    let (mut client, cid) = {
        let mut c = client;
        let ClientMsg::Hello(h) = c.reconnect(hello()) else { panic!() };
        enter(&mut server, Some(c), h)
    };
    if let Some(name) = edit {
        let mut d = client.document().clone();
        let page = d.first_page().unwrap();
        d.node_mut(page).unwrap().name = name.into();
        let m = client.edit(&d).unwrap().unwrap();
        for o in server.receive(cid, m) {
            if let Outgoing::All(m) | Outgoing::To(_, m) = o {
                client.receive(m, &NoRecompute).unwrap();
            }
        }
    }
    server.leave(cid);
    store.changed(id, &mut server);
    store.flush();
    assert!(store.idle(id, &mut server));
    client
}

fn page_name(server: &Server) -> String {
    let d = server.replica().document();
    d.first_page().and_then(|p| d.node(p)).map(|n| n.name.clone()).unwrap_or_default()
}

#[test]
fn rejoining_an_unchanged_file_reads_and_writes_nothing() {
    let dir = TempDir::new("idle-io");
    let (mut store, id, mut client, _) = published(&dir);
    let log = dir.path().join(FILES).join(&id).join(crate::disk::LOG);
    let log_len = fs::metadata(&log).unwrap().len();
    disk::take_io();
    for _ in 0..20 {
        client = cycle(&mut store, &id, client, None);
    }
    assert_eq!(disk::take_io(), (0, 0), "20 join/leave cycles: no file read or written");
    assert_eq!(fs::metadata(&log).unwrap().len(), log_len);
    assert_eq!(store.compactions(), 0);
}

#[test]
fn edits_while_kept_loaded_are_still_saved() {
    let dir = TempDir::new("idle-edits");
    let (mut store, id, mut client, c) = published(&dir);
    for i in 0..3 {
        client = cycle(&mut store, &id, client, Some(&format!("Page {i}")));
    }
    // A cycle with an edit compacts it on leaving, as before; the next one does nothing again.
    assert!(store.compactions() > 0);
    disk::take_io();
    let _ = cycle(&mut store, &id, client, None);
    assert_eq!(disk::take_io(), (0, 0));
    // Saved: another store (a restart) reads it back.
    drop(store);
    let mut again = Store::open(dir.path(), c).unwrap();
    disk::take_io();
    let server = again.open(&id).unwrap();
    assert_eq!(page_name(&server), "Page 2");
    assert_eq!(disk::take_io().0, 2, "one load: the snapshot and the log");
}

#[test]
fn the_file_kept_loaded_longest_lets_go_first() {
    let dir = TempDir::new("idle-many");
    let mut store = Store::open(dir.path(), config(dir.path()).store()).unwrap();
    let ids: Vec<String> =
        (0..17).map(|i| store.publish(&format!("F{i}"), &Replica::new(Document::new("F")).snapshot().unwrap()).unwrap().0.id).collect();
    for id in &ids {
        let mut server = store.open(id).unwrap();
        assert!(store.idle(id, &mut server));
    }
    disk::take_io();
    let _ = store.open(&ids[16]).unwrap();
    assert_eq!(disk::take_io(), (0, 0), "the latest is still loaded");
    let _ = store.open(&ids[0]).unwrap();
    assert_eq!(disk::take_io(), (2, 0), "past 16, the first let go: read again");
}

#[test]
fn the_open_time_is_written_hourly_and_when_the_server_stops() {
    let dir = TempDir::new("idle-opened");
    let (mut store, id, client, _) = published(&dir);
    let opened = || -> u64 {
        let meta: serde_json::Value = serde_json::from_slice(&fs::read(dir.path().join(FILES).join(&id).join(META)).unwrap()).unwrap();
        meta["opened"].as_u64().unwrap()
    };
    // Last written half an hour ago: a join doesn't write it yet; stopping does.
    store.age(&id, 1800);
    let before = opened();
    disk::take_io();
    let client = cycle(&mut store, &id, client, None);
    assert_eq!(disk::take_io(), (0, 0));
    assert_eq!(opened(), before);
    store.stop(Vec::new());
    assert!(opened() >= before + 1800);
    // Written over an hour ago: the next join writes it.
    store.age(&id, 4000);
    disk::take_io();
    let _ = cycle(&mut store, &id, client, None);
    assert_eq!(disk::take_io(), (0, 1));
}
