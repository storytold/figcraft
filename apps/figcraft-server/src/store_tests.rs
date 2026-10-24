//! The data directory on its own: publishing, links and their hashes, persistence across a crash
//! (snapshot plus log) and compaction, damaged files skipped and reported, caps, the lock, and
//! temporary directories cleaned up.

use std::fs;

use figcraft_collab::message::{Access, ClientMsg};
use figcraft_collab::sim::Rng;
use figcraft_collab::{Client, Outgoing, Replica, Server};
use figcraft_collab_net::hub::Backend;
use figcraft_doc::{Document, NodeKind};

use crate::disk::{self, LINKS, LOG, META, SNAPSHOT};
use crate::store::{FILES, Store, StoreConfig, hash};
use crate::test_util::{TempDir, config};

fn cfg(dir: &std::path::Path) -> StoreConfig {
    config(dir).store()
}

fn doc(name: &str) -> Document {
    let mut d = Document::new(name);
    let page = d.first_page().unwrap();
    d.create(page, NodeKind::Rectangle, "Card", 0.0, 0.0, 100.0, 60.0).unwrap();
    d.repair();
    d
}

fn snapshot(d: Document) -> Vec<figcraft_collab::Change> {
    Replica::new(d).snapshot().unwrap()
}

/// Publish `name`; its id and the edit and view tokens.
fn publish(store: &mut Store, name: &str) -> (String, String, String) {
    let (info, edit, view) = store.publish(name, &snapshot(doc(name))).unwrap();
    assert_eq!(edit.link.access, Access::Edit);
    assert_eq!(view.link.access, Access::View);
    (info.id, edit.token, view.token)
}

/// A client in `server` (as the hub would let it in) and what it was told.
fn join(server: &mut Server, access: Access) -> Client {
    let ClientMsg::Hello(h) = Client::hello("f", "", "Ada", "#FF8800") else { panic!() };
    let (_, out) = server.join_as(h, access).unwrap();
    let Some(Outgoing::To(_, w)) = out.into_iter().next() else { panic!() };
    Client::join(w).unwrap()
}

/// Rename the first page through `c`; the server takes the batch.
fn rename(server: &mut Server, c: &mut Client, name: &str) {
    let mut d = c.document().clone();
    let page = d.first_page().unwrap();
    d.node_mut(page).unwrap().name = name.into();
    let m = c.edit(&d).unwrap().unwrap();
    for o in server.receive(c.id(), m) {
        if let Outgoing::All(m) | Outgoing::To(_, m) = o {
            c.receive(m, &figcraft_collab::NoRecompute).unwrap();
        }
    }
}

#[test]
fn files_links_and_access() {
    let dir = TempDir::new("links");
    let mut store = Store::open(dir.path(), cfg(dir.path())).unwrap();
    assert!(store.is_empty());
    let (id, edit, view) = publish(&mut store, "Café ☕");
    let files = store.list(&|_| None);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].name, "Café ☕");
    assert!(files[0].bytes > 0);
    // Each link lets in with its access; anything else doesn't.
    assert_eq!(store.admit(&id, &edit).unwrap().access, Access::Edit);
    assert_eq!(store.admit(&id, &view).unwrap().access, Access::View);
    assert!(store.admit(&id, "WrongWrongWrongWrong00").unwrap_err().contains("isn't valid"));
    assert!(store.admit("0000000000000000", &edit).unwrap_err().contains("isn't on the server"));
    // Tokens are kept only as hashes.
    let on_disk = fs::read_to_string(dir.path().join(FILES).join(&id).join(LINKS)).unwrap();
    assert!(!on_disk.contains(&edit) && !on_disk.contains(&view));
    assert!(on_disk.contains(&hash(&edit)) && on_disk.contains(&hash(&view)));
    // More links; revoking one stops it, the others keep working.
    let review = store.create_link(&id, Access::View, " Review\n").unwrap();
    assert_eq!(review.link.label, "Review");
    assert_eq!(store.links(&id).unwrap().len(), 3);
    assert_eq!(store.admit(&id, &review.token).unwrap().link.as_deref(), Some(review.link.id.as_str()));
    store.revoke(&id, &review.link.id).unwrap();
    assert!(store.admit(&id, &review.token).is_err());
    assert!(store.admit(&id, &view).is_ok());
    assert!(store.revoke(&id, &review.link.id).is_err(), "already gone");
    // Removed for good.
    store.remove(&id).unwrap();
    assert!(store.is_empty());
    assert!(!dir.path().join(FILES).join(&id).exists());
    assert!(store.admit(&id, &edit).is_err());
}

#[test]
fn a_file_survives_a_crash_and_compacts() {
    let dir = TempDir::new("crash");
    let c = StoreConfig { compact_after: 1000, ..cfg(dir.path()) };
    let (id, before, seq) = {
        let mut store = Store::open(dir.path(), c.clone()).unwrap();
        let (id, _, _) = publish(&mut store, "Crash");
        let mut server = store.open(&id).unwrap();
        let mut a = join(&mut server, Access::Edit);
        store.changed(&id, &mut server);
        for i in 0..5 {
            rename(&mut server, &mut a, &format!("Page {i}"));
            store.changed(&id, &mut server);
        }
        store.flush();
        // A crash: no compaction, nothing else written.
        (id, server.replica().clone(), server.seq())
    };
    let mut store = Store::open(dir.path(), c).unwrap();
    assert!(store.problems().is_empty(), "{:?}", store.problems());
    let mut server = store.open(&id).unwrap();
    assert_eq!(server.replica(), &before, "the log brought it back");
    assert_eq!(server.seq(), seq);
    // Compaction: everyone left; the snapshot has it all and the log starts over.
    assert!(store.idle(&id, &mut server));
    let snap = disk::decode_snapshot(&fs::read(dir.path().join(FILES).join(&id).join(SNAPSHOT)).unwrap()).unwrap();
    assert_eq!(snap.1.seq, seq);
    let log = disk::read_log(&fs::read(dir.path().join(FILES).join(&id).join(LOG)).unwrap()).unwrap();
    assert_eq!(log.generation, snap.0);
    assert!(log.entries.is_empty());
    assert_eq!(store.list(&|_| None)[0].seq, seq, "the description follows");
}

#[test]
fn read_only_sessions_keep_client_ids_without_rewriting_the_snapshot() {
    let dir = TempDir::new("read-only-reconnects");
    let c = StoreConfig { compact_after: 1000, ..cfg(dir.path()) };
    let (id, original) = {
        let mut store = Store::open(dir.path(), c.clone()).unwrap();
        let (id, _, _) = publish(&mut store, "Read only");
        let bytes = fs::read(dir.path().join(FILES).join(&id).join(SNAPSHOT)).unwrap();
        (id, bytes)
    };
    let mut last_client = 0;
    for _ in 0..6 {
        let mut store = Store::open(dir.path(), c.clone()).unwrap();
        let mut server = store.open(&id).unwrap();
        let client = join(&mut server, Access::View);
        assert!(client.id() > last_client, "issued ids survive closing and reopening the store");
        last_client = client.id();
        store.changed(&id, &mut server);
        // Idle must sync even if no periodic flush ran before the connection closed.
        assert!(store.idle(&id, &mut server));
        assert_eq!(store.compactions(), 0, "a client id does not require rebuilding the image-bearing snapshot");
        assert_eq!(fs::read(dir.path().join(FILES).join(&id).join(SNAPSHOT)).unwrap(), original);
        let log = disk::read_log(&fs::read(dir.path().join(FILES).join(&id).join(LOG)).unwrap()).unwrap();
        assert!(!log.entries.is_empty());
        assert_eq!(server.seq(), 0);
    }
}

#[test]
fn a_long_log_is_compacted_while_open() {
    let dir = TempDir::new("compact");
    let c = StoreConfig { compact_after: 3, ..cfg(dir.path()) };
    let mut store = Store::open(dir.path(), c.clone()).unwrap();
    let (id, _, _) = publish(&mut store, "Busy");
    let mut server = store.open(&id).unwrap();
    let mut a = join(&mut server, Access::Edit);
    store.changed(&id, &mut server);
    let generation = || disk::check_snapshot(&fs::read(dir.path().join(FILES).join(&id).join(SNAPSHOT)).unwrap()).unwrap();
    assert_eq!(generation(), 1);
    for i in 0..3 {
        rename(&mut server, &mut a, &format!("P{i}"));
        store.changed(&id, &mut server);
        store.flush();
    }
    assert_eq!(generation(), 2, "compacted after three entries");
    rename(&mut server, &mut a, "After");
    store.changed(&id, &mut server);
    store.flush();
    drop(store);
    let mut store = Store::open(dir.path(), c).unwrap();
    let restored = store.open(&id).unwrap();
    assert_eq!(restored.replica(), server.replica());
}

#[test]
fn damaged_files_are_skipped_and_reported() {
    let dir = TempDir::new("damaged");
    let (bad_snapshot, bad_meta, cut_log, good) = {
        let mut store = Store::open(dir.path(), cfg(dir.path())).unwrap();
        let ids: Vec<String> = ["A", "B", "C", "D"].iter().map(|n| publish(&mut store, n).0).collect();
        // C gets a log with entries.
        let mut server = store.open(&ids[2]).unwrap();
        let mut c = join(&mut server, Access::Edit);
        store.changed(&ids[2], &mut server);
        rename(&mut server, &mut c, "Kept");
        store.changed(&ids[2], &mut server);
        rename(&mut server, &mut c, "Lost");
        store.changed(&ids[2], &mut server);
        store.flush();
        (ids[0].clone(), ids[1].clone(), ids[2].clone(), ids[3].clone())
    };
    let folder = |id: &str| dir.path().join(FILES).join(id);
    // A: a flipped byte in its snapshot; B: garbage description; C: its log cut mid-record;
    // D: a write that was cut short left a temporary file.
    let mut snap = fs::read(folder(&bad_snapshot).join(SNAPSHOT)).unwrap();
    let last = snap.len() - 1;
    snap[last] ^= 0xFF;
    fs::write(folder(&bad_snapshot).join(SNAPSHOT), snap).unwrap();
    fs::write(folder(&bad_meta).join(META), b"{not json").unwrap();
    let log = fs::read(folder(&cut_log).join(LOG)).unwrap();
    fs::write(folder(&cut_log).join(LOG), &log[..log.len() - 3]).unwrap();
    fs::write(disk::temp_of(&folder(&good).join(SNAPSHOT)), b"half").unwrap();
    fs::create_dir_all(dir.path().join(FILES).join("not a file id")).unwrap();
    let mut store = Store::open(dir.path(), cfg(dir.path())).unwrap();
    let problems = store.problems().join("\n");
    assert!(problems.contains(&bad_snapshot) && problems.contains("checksum"), "{problems}");
    assert!(problems.contains(&bad_meta) && problems.contains(META), "{problems}");
    assert!(problems.contains("not a file id"), "{problems}");
    assert_eq!(store.len(), 2, "C and D are served");
    // Nothing is deleted.
    assert!(folder(&bad_snapshot).exists() && folder(&bad_meta).exists());
    assert!(!disk::temp_of(&folder(&good).join(SNAPSHOT)).exists(), "the leftover is gone");
    // C opens without the cut-short record, and says so.
    let server = store.open(&cut_log).unwrap();
    let page = server.document().first_page().unwrap();
    assert_eq!(server.document().node(page).unwrap().name, "Kept");
    assert!(store.problems().iter().any(|p| p.contains(&cut_log) && p.contains("cut short")));
    // Damage after startup: the file can't be opened, with a reason for the person joining.
    fs::write(folder(&good).join(SNAPSHOT), b"FCSN garbage").unwrap();
    assert!(store.open(&good).unwrap_err().contains("can't be opened"));
    assert!(store.problems().iter().any(|p| p.contains(&good)));
}

#[test]
fn caps_hold() {
    let dir = TempDir::new("caps");
    let c = StoreConfig { max_files: 1, max_links: 3, ..cfg(dir.path()) };
    let mut store = Store::open(dir.path(), c).unwrap();
    let (id, _, _) = publish(&mut store, "One");
    assert!(store.publish("Two", &snapshot(doc("Two"))).unwrap_err().contains("at most 1"));
    store.create_link(&id, Access::View, "").unwrap();
    assert!(store.create_link(&id, Access::View, "").unwrap_err().contains("at most 3"));
    let dir2 = TempDir::new("caps-size");
    let mut tiny = Store::open(dir2.path(), StoreConfig { max_file_bytes: 64, ..cfg(dir2.path()) }).unwrap();
    assert!(tiny.publish("Big", &snapshot(doc("Big"))).unwrap_err().contains("MiB"));
    assert!(tiny.publish("Junk", &[figcraft_collab::Change::Delete { id: figcraft_doc::NodeId(1) }]).is_err());
}

#[test]
fn one_server_per_data_directory() {
    let dir = TempDir::new("lock");
    let first = Store::open(dir.path(), cfg(dir.path())).unwrap();
    assert!(Store::open(dir.path(), cfg(dir.path())).err().unwrap().contains("is using"));
    assert!(Store::try_open(dir.path(), cfg(dir.path())).unwrap().is_none());
    let key = first.key().to_string();
    drop(first);
    let again = Store::open(dir.path(), cfg(dir.path())).unwrap();
    assert_eq!(again.key(), key, "the key is kept");
}

#[cfg(unix)]
#[test]
fn an_existing_data_directory_is_made_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new("perms");
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(crate::store::lock(dir.path()).unwrap().is_some());
    let mode = fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
}

#[test]
fn temporary_directories_are_removed() {
    let path = {
        let dir = TempDir::new("cleanup");
        let mut store = Store::open(dir.path(), cfg(dir.path())).unwrap();
        publish(&mut store, "Gone");
        drop(store);
        dir.path().to_path_buf()
    };
    assert!(!path.exists());
}

#[test]
fn snapshot_encoding_keeps_the_existing_framing_and_checksum() {
    let state = Server::new(doc("Snapshot framing"), "", figcraft_collab::Limits::default()).state().unwrap();
    let body = figcraft_collab::message::encode(&state).unwrap();
    let mut legacy = b"FCSN\x01".to_vec();
    legacy.extend_from_slice(&37u64.to_le_bytes());
    legacy.extend_from_slice(&crc32fast::hash(&body).to_le_bytes());
    legacy.extend_from_slice(&body);
    let encoded = disk::encode_snapshot(37, &state).unwrap();
    assert_eq!(encoded, legacy);
    assert_eq!(disk::decode_snapshot(&encoded).unwrap(), (37, state));
}

#[test]
fn random_bytes_never_panic_the_readers() {
    let mut rng = Rng::new(7);
    let state = Server::new(doc("Fuzz"), "", figcraft_collab::Limits::default()).state().unwrap();
    let snap = disk::encode_snapshot(3, &state).unwrap();
    let mut log = disk::log_header(3);
    log.extend(disk::record(&figcraft_collab::Journal::Issued { client: 9 }).unwrap());
    assert_eq!(disk::decode_snapshot(&snap).unwrap().1, state);
    assert_eq!(disk::read_log(&log).unwrap().entries.len(), 1);
    for round in 0..5_000 {
        let base = if round % 2 == 0 { &snap } else { &log };
        let mut bytes = base.clone();
        for _ in 0..1 + rng.below(4) {
            if bytes.is_empty() {
                break;
            }
            let i = rng.below(bytes.len());
            match rng.below(3) {
                0 => bytes[i] = rng.next_u64() as u8,
                1 => bytes.insert(i, rng.next_u64() as u8),
                _ => bytes.truncate(i),
            }
        }
        let _ = disk::decode_snapshot(&bytes);
        let _ = disk::check_snapshot(&bytes);
        let _ = disk::read_log(&bytes);
    }
}

/// The first page's name in `server`'s document.
fn page_name(server: &Server) -> String {
    let d = server.replica().document();
    d.node(d.first_page().unwrap()).unwrap().name.clone()
}

#[test]
fn edits_confirmed_after_a_compaction_cut_short_survive_a_restart() {
    let dir = TempDir::new("cutshort");
    let c = StoreConfig { compact_after: 3, ..cfg(dir.path()) };
    let folder = dir.path().join(FILES);
    let mut store = Store::open(dir.path(), c.clone()).unwrap();
    let (id, _, _) = publish(&mut store, "Busy");
    let mut server = store.open(&id).unwrap();
    let mut a = join(&mut server, Access::Edit);
    store.changed(&id, &mut server);
    // The next compaction writes its snapshot, then can't start its log.
    let blocker = disk::temp_of(&folder.join(&id).join(LOG));
    fs::create_dir(&blocker).unwrap();
    // What the hub would confirm: every change whose flush didn't report the file.
    let mut confirmed = String::new();
    let mut failed = false;
    let names = ["P0", "P1", "P2", "After 1", "After 2"];
    for name in names {
        rename(&mut server, &mut a, name);
        store.changed(&id, &mut server);
        if store.flush().contains(&id) {
            failed = true;
            break;
        }
        confirmed = name.to_string();
    }
    assert!(failed, "the hub hears the file can't be kept; confirmed {confirmed:?}");
    // A crash, then the disk recovers.
    drop(store);
    fs::remove_dir(&blocker).unwrap();
    let mut store = Store::open(dir.path(), c).unwrap();
    let restored = store.open(&id).unwrap();
    // Every confirmed change is there (the snapshot may also hold the one that was cut short).
    let at = |n: &str| names.iter().position(|x| *x == n);
    let name = page_name(&restored);
    assert!(at(&name) >= at(&confirmed), "restored {name:?}, confirmed {confirmed:?}");
}

#[test]
fn a_log_that_cant_be_read_is_never_overwritten() {
    let dir = TempDir::new("unreadlog");
    let c = StoreConfig { compact_after: 1000, ..cfg(dir.path()) };
    let log_path = dir.path().join(FILES);
    let last = "Confirmed page name number 39 with some padding";
    let id = {
        let mut store = Store::open(dir.path(), c.clone()).unwrap();
        let (id, _, _) = publish(&mut store, "Busy");
        let mut server = store.open(&id).unwrap();
        let mut a = join(&mut server, Access::Edit);
        store.changed(&id, &mut server);
        for i in 0..40 {
            rename(&mut server, &mut a, &format!("Confirmed page name number {i} with some padding"));
            store.changed(&id, &mut server);
        }
        assert!(store.flush().is_empty());
        id
    };
    let log_path = log_path.join(&id).join(LOG);
    let before = fs::read(&log_path).unwrap();
    // The cap lowered just under the log: it can't be read now, but it must still be there after.
    let lowered = StoreConfig { max_file_bytes: before.len() as u64 - 1, ..c.clone() };
    {
        let mut store = Store::open(dir.path(), lowered).unwrap();
        let opened = store.open(&id);
        assert!(opened.is_err(), "the file isn't opened without its log");
        assert!(!store.problems().is_empty(), "the problem is reported");
        assert_eq!(fs::read(&log_path).unwrap(), before, "the log is left as it was");
    }
    let mut store = Store::open(dir.path(), c).unwrap();
    let server = store.open(&id).unwrap();
    assert_eq!(page_name(&server), last, "every confirmed edit comes back");
}

#[test]
fn a_log_that_isnt_one_is_moved_aside() {
    let dir = TempDir::new("badlog");
    let mut store = Store::open(dir.path(), cfg(dir.path())).unwrap();
    let (id, _, _) = publish(&mut store, "Busy");
    let folder = dir.path().join(FILES).join(&id);
    let junk = b"not a log at all, but maybe worth keeping".to_vec();
    fs::write(folder.join(LOG), &junk).unwrap();
    drop(store);
    let mut store = Store::open(dir.path(), cfg(dir.path())).unwrap();
    store.open(&id).unwrap();
    assert!(store.problems().iter().any(|p| p.contains("moved aside")), "{:?}", store.problems());
    let kept = fs::read_dir(&folder).unwrap().filter_map(|e| e.ok()).any(|e| fs::read(e.path()).is_ok_and(|b| b == junk));
    assert!(kept, "the old log's bytes are kept beside the file");
}

const BIDI: [char; 12] =
    ['\u{61C}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}'];

#[test]
fn names_and_report_reasons_lose_bidi_controls() {
    let dir = TempDir::new("bidi");
    let mut store = Store::open(dir.path(), cfg(dir.path())).unwrap();
    for (n, m) in BIDI.into_iter().enumerate() {
        let ip: std::net::IpAddr = format!("192.0.2.{}", n + 1).parse().unwrap();
        let (id, _, _) = publish(&mut store, &format!("a{m}b"));
        assert_eq!(store.list(&|_| None)[0].name, "ab", "name U+{:04X}", m as u32);
        store.keep_report(&id, None, ip, &format!("a{m}b")).unwrap();
        assert_eq!(store.reports().last().unwrap().reason, "a b", "reason U+{:04X}", m as u32);
        store.remove(&id).unwrap();
    }
    // A reason that is only control characters says nothing.
    let (id, _, _) = publish(&mut store, "blank");
    for blank in ["\0", "\u{202E}", " \0\t\u{2066} "] {
        assert!(store.keep_report(&id, None, "192.0.2.99".parse().unwrap(), blank).is_err(), "{blank:?}");
    }
}

#[test]
fn a_maxed_out_generation_does_not_wrap() {
    let dir = TempDir::new("maxgen");
    let c = StoreConfig { compact_after: 3, ..cfg(dir.path()) };
    let mut store = Store::open(dir.path(), c.clone()).unwrap();
    let (id, _, _) = publish(&mut store, "Max");
    let path = dir.path().join(FILES).join(&id);
    let (_, state) = disk::decode_snapshot(&fs::read(path.join(SNAPSHOT)).unwrap()).unwrap();
    // Loading: no log goes with the snapshot, so the next generation would overflow.
    fs::write(path.join(SNAPSHOT), disk::encode_snapshot(u64::MAX, &state).unwrap()).unwrap();
    fs::remove_file(path.join(LOG)).unwrap();
    assert!(store.open(&id).is_err());
    // Compacting: the open log is at the largest generation.
    fs::write(path.join(SNAPSHOT), disk::encode_snapshot(u64::MAX - 1, &state).unwrap()).unwrap();
    let _ = fs::remove_file(path.join(LOG));
    let mut server = store.open(&id).unwrap();
    let mut a = join(&mut server, Access::Edit);
    store.changed(&id, &mut server);
    for i in 0..4 {
        rename(&mut server, &mut a, &format!("P{i}"));
        store.changed(&id, &mut server);
        store.flush();
    }
    let snap = disk::check_snapshot(&fs::read(path.join(SNAPSHOT)).unwrap()).unwrap();
    assert_eq!(snap, u64::MAX);
}

#[test]
fn every_leftover_temporary_file_is_removed_at_startup() {
    let dir = TempDir::new("leftovers");
    let id = {
        let mut store = Store::open(dir.path(), cfg(dir.path())).unwrap();
        let id = publish(&mut store, "A").0;
        store.flush();
        id
    };
    let folder = dir.path().join(FILES).join(&id);
    let leftovers = [
        dir.path().join("blocked.json.tmp"),
        dir.path().join("reports.json.tmp"),
        dir.path().join("server.key.tmp"),
        folder.join("usage.json.tmp"),
        folder.join("library-1.bin.tmp"),
    ];
    for path in &leftovers {
        fs::write(path, b"half").unwrap();
    }
    let _store = Store::open(dir.path(), cfg(dir.path())).unwrap();
    for path in &leftovers {
        assert!(!path.exists(), "leftover not cleaned at startup: {}", path.display());
    }
}
