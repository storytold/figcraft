//! A generation field damaged after an abrupt stop must not make the server drop a log that still
//! holds confirmed edits (#408): the log is replayed over the snapshot, which only the changes the
//! snapshot lacks survive, and the damage is reported. A genuine leftover of a compaction cut short
//! still restores silently.

use figcraft_collab::message::{Access, ClientMsg};
use figcraft_collab::{Client, Outgoing, Replica, Server};
use figcraft_collab_net::hub::Backend;
use figcraft_doc::Document;
use figcraft_server::config::Config;
use figcraft_server::disk::{self, LOG, SNAPSHOT};
use figcraft_server::store::{FILES, Store};
use std::fs;
use std::path::{Path, PathBuf};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("figcraft-header-recovery-{}", figcraft_collab_net::token::file_id().unwrap()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn page_name(server: &Server) -> String {
    let doc = server.document();
    doc.node(doc.first_page().unwrap()).unwrap().name.clone()
}

/// A published file whose first page was renamed "Kept", flushed to its log (not compacted), then
/// the store dropped without stopping (an abrupt stop). Returns the file id and the final state.
fn renamed_and_stopped(data: &Path) -> (String, Server) {
    let mut store = Store::open(data, Config::default().store()).unwrap();
    let snapshot = Replica::new(Document::new("Fixture")).snapshot().unwrap();
    let (info, _, _) = store.publish("Fixture", &snapshot).unwrap();
    let id = info.id;
    let mut server = store.open(&id).unwrap();
    let ClientMsg::Hello(hello) = Client::hello("f", "", "Editor", "#FF8800") else { panic!("not a hello") };
    let (_, messages) = server.join_as(hello, Access::Edit).unwrap();
    let Some(Outgoing::To(_, welcome)) = messages.into_iter().next() else { panic!("no welcome") };
    let mut client = Client::join(welcome).unwrap();
    store.changed(&id, &mut server);
    let mut edited = client.document().clone();
    let page = edited.first_page().unwrap();
    edited.node_mut(page).unwrap().name = "Kept".into();
    let change = client.edit(&edited).unwrap().unwrap();
    assert!(!server.receive(client.id(), change).is_empty());
    store.changed(&id, &mut server);
    assert!(store.flush().is_empty(), "the edit must reach disk");
    assert_eq!(page_name(&server), "Kept");
    let dir = data.join(FILES).join(&id);
    assert!(!disk::read_log(&fs::read(dir.join(LOG)).unwrap()).unwrap().entries.is_empty());
    assert_eq!(disk::decode_snapshot(&fs::read(dir.join(SNAPSHOT)).unwrap()).unwrap().1.seq, 0, "the rename is only in the log");
    drop(store);
    (id, server)
}

/// Reopen the data directory: the file's first page name and the problems reported.
fn reopen(data: &Path, id: &str) -> (String, Vec<String>) {
    let mut store = Store::open(data, Config::default().store()).unwrap();
    let server = store.open(id).unwrap();
    (page_name(&server), store.problems().to_vec())
}

fn flip(path: &Path, at: usize) {
    let mut bytes = fs::read(path).unwrap();
    bytes[at] ^= 1;
    fs::write(path, bytes).unwrap();
}

/// The generation field's first byte in both headers (after the magic and the format byte).
const GENERATION_AT: usize = 5;

#[test]
fn an_intact_log_restores_its_edits_without_a_report() {
    let fixture = Fixture::new();
    let (id, _) = renamed_and_stopped(&fixture.0);
    let (name, problems) = reopen(&fixture.0, &id);
    assert_eq!(name, "Kept");
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn a_damaged_log_generation_keeps_the_edits_and_is_reported() {
    let fixture = Fixture::new();
    let (id, _) = renamed_and_stopped(&fixture.0);
    let log = fixture.0.join(FILES).join(&id).join(LOG);
    flip(&log, GENERATION_AT);
    // The records themselves still read: only the header's generation changed.
    let read = disk::read_log(&fs::read(&log).unwrap()).unwrap();
    assert!(read.damaged.is_none() && !read.entries.is_empty());
    let (name, problems) = reopen(&fixture.0, &id);
    assert_eq!(name, "Kept", "the flushed rename must survive");
    assert!(problems.iter().any(|p| p.contains(&id) && p.contains("generation")), "{problems:?}");
    // And it stays: the recovered edit is now in the file's snapshot.
    assert_eq!(reopen(&fixture.0, &id).0, "Kept");
}

#[test]
fn a_damaged_snapshot_generation_keeps_the_edits_and_is_reported() {
    let fixture = Fixture::new();
    let (id, _) = renamed_and_stopped(&fixture.0);
    let snapshot = fixture.0.join(FILES).join(&id).join(SNAPSHOT);
    flip(&snapshot, GENERATION_AT);
    assert!(disk::check_snapshot(&fs::read(&snapshot).unwrap()).is_ok(), "the generation is outside the checksum");
    let (name, problems) = reopen(&fixture.0, &id);
    assert_eq!(name, "Kept", "the flushed rename must survive");
    assert!(problems.iter().any(|p| p.contains(&id) && p.contains("generation")), "{problems:?}");
    assert_eq!(reopen(&fixture.0, &id).0, "Kept");
}

#[test]
fn a_damaged_log_format_is_still_reported() {
    let fixture = Fixture::new();
    let (id, _) = renamed_and_stopped(&fixture.0);
    flip(&fixture.0.join(FILES).join(&id).join(LOG), 4);
    let (_, problems) = reopen(&fixture.0, &id);
    assert!(problems.iter().any(|p| p.contains(&id) && p.contains("format")), "{problems:?}");
}

#[test]
fn a_log_left_by_a_compaction_cut_short_restores_without_a_report() {
    let fixture = Fixture::new();
    let (id, server) = renamed_and_stopped(&fixture.0);
    // The next snapshot was written (it holds the rename); the old log wasn't replaced yet.
    let bytes = disk::encode_snapshot(2, &server.state().unwrap()).unwrap();
    fs::write(fixture.0.join(FILES).join(&id).join(SNAPSHOT), bytes).unwrap();
    let (name, problems) = reopen(&fixture.0, &id);
    assert_eq!(name, "Kept");
    assert!(problems.is_empty(), "a leftover log is expected, not damage: {problems:?}");
}
