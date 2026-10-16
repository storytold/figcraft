//! Open hosting against resource exhaustion: each test reproduces an attack from the security
//! review and checks what bounds it — a zip-bomb library (H1), a client that never reads its
//! replies (H2), a file growing past its cap through batches and a log that can't be written (H3),
//! usage reports that rewrote a file per request (M1), oversized messages and a busy hub (M2),
//! many addresses from one IPv6 block and one-shot requests taking every connection (M3), revoked
//! links on library connections (M4), data directory permissions (L6) and downloads per address
//! (L7).

use std::io::{Cursor, Write};
use std::net::{IpAddr, TcpListener};
use std::time::{Duration, Instant};

use figcraft_collab::message::{
    Access, Admin, AdminReply, ClientMsg, LibraryReply, LibraryReq, OpenHosting, OpenLibrary, OpenPlugins, PROTOCOL, PluginReq, ServerMsg, Usage,
};
use figcraft_collab::sim::Rng;
use figcraft_collab::{Client, Limits, NoRecompute, Outgoing, Replica, Server};
use figcraft_collab_net::hub::{Backend, Hosting};
use figcraft_collab_net::{Endpoint, Invite, NetLimits, manage};
use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_engine::Session;
use serde_json::json;

use crate::abuse::{Abuse, AbuseConfig, Action, DOWNLOAD_BURST};
use crate::config::Config;
use crate::public_tests::via_proxy;
use crate::store::{BUSY_DOWNLOADS, NO_PUBLIC_LIBRARY, Store};
use crate::test_util::{Raw, TempDir, config};

fn open(dir: &std::path::Path) -> Config {
    Config { open_publish: true, operator_contact: "abuse@example.com".into(), ..config(dir) }
}

fn snapshot(name: &str) -> Vec<figcraft_collab::Change> {
    let mut d = Document::new(name);
    let page = d.first_page().unwrap();
    d.create(page, NodeKind::Rectangle, "Card", 0.0, 0.0, 100.0, 60.0).unwrap();
    d.repair();
    Replica::new(d).snapshot().unwrap()
}

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

/// A team library archive as the app makes it; with `bomb` bytes of spaces as its snapshot
/// (deflated: a few KiB) and `padding` incompressible bytes stored beside it.
fn archive(bomb: usize, padding: usize) -> Vec<u8> {
    let mut s = Session::new();
    let f = s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 50, "height": 20, "name": "Button"})).unwrap()["id"].as_u64().unwrap();
    s.execute("component.create", &json!({"ids": [f]})).unwrap();
    s.execute("library.publish", &json!({})).unwrap();
    let lib = s.doc().unwrap().published.clone().unwrap();
    let bytes = figcraft_engine::library_server::archive(&lib).unwrap();
    if bomb == 0 && padding == 0 {
        return bytes;
    }
    let mut src = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..src.len() {
        let f = src.by_index_raw(i).unwrap();
        if bomb > 0 && f.name() == "library/document.json" {
            continue;
        }
        out.raw_copy_file(f).unwrap();
    }
    let opts = |m| zip::write::SimpleFileOptions::default().compression_method(m);
    if bomb > 0 {
        out.start_file("library/document.json", opts(zip::CompressionMethod::Deflated)).unwrap();
        let chunk = vec![b' '; 1 << 20];
        for _ in 0..bomb >> 20 {
            out.write_all(&chunk).unwrap();
        }
    }
    if padding > 0 {
        out.start_file("padding.bin", opts(zip::CompressionMethod::Stored)).unwrap();
        let mut rng = Rng::new(3);
        out.write_all(&(0..padding).map(|_| rng.next_u64() as u8).collect::<Vec<u8>>()).unwrap();
    }
    out.finish().unwrap().into_inner()
}

fn open_library(link: &Invite) -> ClientMsg {
    ClientMsg::OpenLibrary(OpenLibrary { protocol: PROTOCOL, file: link.file.clone(), token: link.token.clone(), client_version: String::new() })
}

fn hosting() -> ClientMsg {
    ClientMsg::OpenHosting(OpenHosting { protocol: PROTOCOL, client_version: String::new(), file: String::new(), owner: String::new() })
}

/// Poll `f` until it holds or `secs` pass (bounded: never a fixed sleep for the outcome).
fn until(secs: u64, what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    while !f() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A client in `server` (as the hub would let it in).
fn join(server: &mut Server) -> Client {
    let ClientMsg::Hello(h) = Client::hello("f", "", "Ada", "#FF8800") else { panic!() };
    let (_, out) = server.join_as(h, Access::Edit).unwrap();
    let Some(Outgoing::To(_, w)) = out.into_iter().next() else { panic!() };
    Client::join(w).unwrap()
}

/// Rename the first page through `c`, as the server takes it.
fn rename(server: &mut Server, c: &mut Client, name: &str) {
    let mut d = c.document().clone();
    let page = d.first_page().unwrap();
    d.node_mut(page).unwrap().name = name.into();
    let m = c.edit(&d).unwrap().unwrap();
    for o in server.receive(c.id(), m) {
        if let Outgoing::All(m) | Outgoing::To(_, m) = o {
            c.receive(m, &NoRecompute).unwrap();
        }
    }
}

// H1 -------------------------------------------------------------------------------------------

#[test]
fn a_zip_bomb_library_is_refused_before_it_unpacks() {
    let dir = TempDir::new("bomb");
    let mut c = config(dir.path());
    c.max_file_mb = Some(1);
    let mut store = Store::open(dir.path(), c.store()).unwrap();
    let (keyed, ..) = store.publish("Library", &snapshot("Library")).unwrap();
    let bomb = archive(64 << 20, 0);
    assert!(bomb.len() < 1 << 20, "a small archive ({} bytes) that unpacks to 64 MiB", bomb.len());
    let e = store.publish_library(&keyed.id, &bomb).unwrap_err();
    assert!(e.contains("too large"), "{e}");
    assert_eq!(store.publish_library(&keyed.id, &archive(0, 0)).unwrap(), 1, "a real library still goes in");
    drop(store);
    // Open to the public: a file published without the key holds no library at all.
    let mut store = Store::open(dir.path(), open(dir.path()).store()).unwrap();
    let (mine, ..) = store.publish_open(ip("203.0.113.5"), "Mine", &snapshot("Mine")).unwrap();
    assert_eq!(store.publish_library(&mine.id, &archive(0, 0)).unwrap_err(), NO_PUBLIC_LIBRARY);
    assert_ne!(store.publish_library(&keyed.id, &archive(0, 0)).unwrap_err(), NO_PUBLIC_LIBRARY, "the operator's files still can");
}

// H2 -------------------------------------------------------------------------------------------

#[test]
fn a_client_that_never_reads_can_queue_only_so_much() {
    let dir = TempDir::new("noread");
    let cfg = Config { max_file_mb: Some(2), max_plugin_mb: 1, ..open(dir.path()) };
    let cap = cfg.net().max_queued_bytes;
    assert_eq!(cap, 4 << 20, "the largest message and a margin");
    let view = {
        let mut store = Store::open(dir.path(), cfg.store()).unwrap();
        let (file, _, view) = store.publish("Library", &snapshot("Library")).unwrap();
        store.publish_library(&file.id, &archive(0, 1_500_000)).unwrap();
        Invite { server: Endpoint::plain("x"), file: file.id, token: view.token }
    };
    let mut server = crate::start(&cfg).unwrap();
    let mut raw = Raw::connect(server.addr());
    raw.send(&open_library(&view));
    assert!(matches!(raw.next(), ServerMsg::Library(r) if matches!(*r, LibraryReply::Ready { .. })));
    // Ask for the 1.5 MB library over and over without reading an answer.
    for _ in 0..100 {
        if raw.try_send(&ClientMsg::Library(LibraryReq::Fetch { have: None, usage: None })).is_err() {
            break;
        }
    }
    let mut most = 0;
    let end = Instant::now() + Duration::from_secs(3);
    while Instant::now() < end {
        most = most.max(server.queued_bytes());
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(most <= cap, "{most} bytes were queued for one client (cap {cap})");
    // Its queue is let go once it is dropped.
    until(15, "the stuck client's queue to be freed", || server.queued_bytes() == 0);
    server.stop();
}

// H3 -------------------------------------------------------------------------------------------

#[test]
fn a_file_cant_grow_past_its_cap_through_batches() {
    let dir = TempDir::new("grow");
    let cfg = Config { max_file_mb: Some(1), max_plugin_mb: 1, ..open(dir.path()) };
    let max = 1u64 << 20;
    let mut server = crate::start(&cfg).unwrap();
    let addr = server.addr();
    let (info, replies) = manage::hosting(
        &Endpoint::plain(addr.to_string()),
        "",
        "",
        vec![Admin::Publish { name: "Grow".into(), snapshot: snapshot("Grow") }],
        &NetLimits::default(),
        Duration::from_secs(5),
    )
    .unwrap();
    assert!(info.open);
    let Some(AdminReply::Hosted { file, edit, .. }) = replies.into_iter().next() else { panic!("not hosted") };
    let mut raw = Raw::connect(addr);
    raw.send(&Client::hello(&file.id, &edit.token, "Grower", ""));
    let welcome = loop {
        if let w @ ServerMsg::Welcome { .. } = raw.next() {
            break w;
        }
    };
    let mut client = Client::join(welcome).unwrap();
    // Each batch adds a layer with a 200 KB name: the file grows until the server refuses.
    let mut refused = None;
    for i in 0..12 {
        let mut d = client.document().clone();
        let page = d.first_page().unwrap();
        d.create(page, NodeKind::Rectangle, &format!("{i}{}", "x".repeat(200_000)), 0.0, 0.0, 10.0, 10.0).unwrap();
        let Some(m) = client.edit(&d).unwrap() else { continue };
        raw.send(&m);
        let answer = loop {
            if let m @ (ServerMsg::Changes(_) | ServerMsg::Reject { .. }) = raw.next() {
                break m;
            }
        };
        if let ServerMsg::Reject { reason, .. } = &answer {
            refused = Some(reason.clone());
        }
        client.receive(answer, &NoRecompute).unwrap();
        if refused.is_some() {
            break;
        }
    }
    let reason = refused.expect("the server refused a batch once the file was full");
    assert!(reason.contains("reached the server's limit of 1 MiB"), "{reason}");
    let on_disk: u64 = ["snapshot.bin", "log.bin"]
        .iter()
        .filter_map(|n| std::fs::metadata(dir.path().join("files").join(&file.id).join(n)).ok())
        .map(|m| m.len())
        .sum();
    // Its snapshot holds the document and the recent batches kept for reconnecting clients (at
    // most a quarter of the cap), and the last batch let in may finish what the cap allows.
    assert!(on_disk <= max + max / 4 + 210_000, "{on_disk} bytes on disk");
    drop(raw);
    server.stop();
    // Everything confirmed opens again with the same settings, after compacting on the way out.
    let kept = client.document().nodes().count();
    let mut server = crate::start(&cfg).unwrap();
    let mut raw = Raw::connect(server.addr());
    raw.send(&Client::hello(&file.id, &edit.token, "Grower", ""));
    let welcome = loop {
        match raw.next() {
            w @ ServerMsg::Welcome { .. } => break w,
            ServerMsg::Refused { reason } => panic!("the file doesn't open again: {reason}"),
            _ => {}
        }
    };
    assert_eq!(Client::join(welcome).unwrap().document().nodes().count(), kept);
    server.stop();
}

#[test]
fn the_sessions_memory_of_batches_is_bounded_in_bytes() {
    let mut server = Server::new(Document::new("Log"), "", Limits { max_log_bytes: 100_000, ..Limits::default() });
    let mut c = join(&mut server);
    for i in 0..20 {
        rename(&mut server, &mut c, &format!("{i}{}", "y".repeat(30_000)));
        assert!(server.log_bytes() <= 100_000, "{} bytes kept", server.log_bytes());
    }
    assert!(server.log_bytes() > 0, "recent batches are still kept");
}

#[test]
fn changes_that_cant_be_saved_are_never_confirmed() {
    let dir = TempDir::new("unsaved");
    let mut store = Store::open(dir.path(), config(dir.path()).store()).unwrap();
    let (info, ..) = store.publish("Durable", &snapshot("Durable")).unwrap();
    let mut server = Backend::open(&mut store, &info.id).unwrap();
    let mut c = join(&mut server);
    rename(&mut server, &mut c, "Kept");
    Backend::changed(&mut store, &info.id, &mut server);
    assert!(Backend::flush(&mut store).is_empty());
    rename(&mut server, &mut c, "Lost");
    Backend::changed(&mut store, &info.id, &mut server);
    store.break_log(&info.id);
    assert_eq!(Backend::flush(&mut store), vec![info.id.clone()], "the hub hears it couldn't be saved");
    // The file opens again from what is on disk: the saved change, not the unsaved one.
    drop(server);
    let mut server = Backend::open(&mut store, &info.id).unwrap();
    let c = join(&mut server);
    let page = c.document().first_page().unwrap();
    assert_eq!(c.document().node(page).unwrap().name, "Kept");
}

// M1 -------------------------------------------------------------------------------------------

#[test]
fn usage_reports_dont_rewrite_a_file_per_request() {
    let dir = TempDir::new("usage");
    let cfg = config(dir.path());
    let view = {
        let mut store = Store::open(dir.path(), cfg.store()).unwrap();
        let (file, _, view) = store.publish("Library", &snapshot("Library")).unwrap();
        store.publish_library(&file.id, &archive(0, 0)).unwrap();
        Invite { server: Endpoint::plain("x"), file: file.id, token: view.token }
    };
    let usage_file = dir.path().join("files").join(&view.file).join("usage.json");
    let mut server = crate::start(&cfg).unwrap();
    let mut raw = Raw::connect(server.addr());
    raw.send(&open_library(&view));
    raw.next();
    for i in 0..50u32 {
        let usage = Usage { file: format!("User {}", i % 7), components: vec![(NodeId(2), i + 1)] };
        raw.send(&ClientMsg::Library(LibraryReq::Fetch { have: Some(1), usage: Some(usage) }));
        assert!(matches!(raw.next(), ServerMsg::Library(r) if matches!(*r, LibraryReply::Unchanged { .. })));
    }
    assert!(!usage_file.exists(), "kept in memory, not written 50 times");
    raw.send(&ClientMsg::Library(LibraryReq::Usage));
    let ServerMsg::Library(r) = raw.next() else { panic!() };
    let LibraryReply::Usage(files) = *r else { panic!() };
    assert_eq!(files.len(), 7);
    server.stop();
    assert!(usage_file.exists(), "written when the server stops");
}

// M2 -------------------------------------------------------------------------------------------

/// Panics in the server's connection threads (`live-…`) since the hook went in: they mustn't
/// happen, but a thread's panic doesn't fail a test by itself.
static CONNECTION_PANICS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn count_connection_panics() {
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let before = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if std::thread::current().name().is_some_and(|n| n.starts_with("live-")) {
                CONNECTION_PANICS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            before(info);
        }));
    });
}

#[test]
fn messages_are_capped_by_what_the_connection_is_for() {
    count_connection_panics();
    let dir = TempDir::new("caps");
    let mut server = crate::start(&open(dir.path())).unwrap();
    let addr = server.addr();
    // Browsing a plugin directory takes small requests only; the oversized frame is refused and
    // the connection closed cleanly (its reader isn't asked for the rest of that frame).
    let mut raw = Raw::connect(addr);
    raw.send(&ClientMsg::OpenPlugins(OpenPlugins { protocol: PROTOCOL, key: String::new(), client_version: String::new() }));
    raw.next();
    raw.send_header(200_000);
    assert!(matches!(raw.next(), ServerMsg::Refused { reason } if reason.contains("too large") && reason.contains("KiB")));
    drop(raw);
    // A joined client whose batch is bigger than a file may be is told the limit.
    let (_, replies) = manage::hosting(
        &Endpoint::plain(addr.to_string()),
        "",
        "",
        vec![Admin::Publish { name: "Small".into(), snapshot: snapshot("Small") }],
        &NetLimits::default(),
        Duration::from_secs(5),
    )
    .unwrap();
    let Some(AdminReply::Hosted { file, edit, .. }) = replies.into_iter().next() else { panic!("not hosted") };
    let mut joined = Raw::connect(addr);
    joined.send(&Client::hello(&file.id, &edit.token, "Big", ""));
    while !matches!(joined.next(), ServerMsg::Welcome { .. }) {}
    joined.send_header(40 << 20);
    let reason = loop {
        if let ServerMsg::Refused { reason } = joined.next() {
            break reason;
        }
    };
    assert!(reason.contains("at most 17 MiB"), "{reason}");
    drop(joined);
    until(10, "the refused connections to close", || server.connections() == 0);
    assert_eq!(CONNECTION_PANICS.load(std::sync::atomic::Ordering::SeqCst), 0, "a connection thread panicked");
    // Open to the public, a message is at most a file's size (and a margin), not 64 MiB.
    assert_eq!(open(dir.path()).net().max_client_message, 17 << 20);
    server.stop();
    // Past the bytes the hub may have waiting, a connection is dropped as busy.
    let cfg = open(dir.path());
    let store = Store::open(dir.path(), cfg.store()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut busy = Hosting::serve(listener, store, NetLimits { max_in_flight: 10_000, ..cfg.net() }).unwrap();
    let mut raw = Raw::connect(busy.addr());
    raw.send(&hosting());
    assert!(matches!(raw.next(), ServerMsg::Hosting(_)));
    let mut big = Document::new("Big");
    let page = big.first_page().unwrap();
    big.create(page, NodeKind::Rectangle, &"z".repeat(30_000), 0.0, 0.0, 1.0, 1.0).unwrap();
    raw.send(&ClientMsg::Admin(Admin::Publish { name: "Big".into(), snapshot: Replica::new(big).snapshot().unwrap() }));
    assert!(matches!(raw.next(), ServerMsg::Refused { reason } if reason.contains("busy")));
    busy.stop();
}

// M3 -------------------------------------------------------------------------------------------

#[test]
fn one_ipv6_block_gets_rising_but_bounded_limits() {
    let dir = TempDir::new("blocks");
    let cfg = Config { max_connections_per_ip: 1, trusted_proxy: Some("127.0.0.1".into()), ..open(dir.path()) };
    let mut server = crate::start(&cfg).unwrap();
    let addr = server.addr();
    let mut held = Vec::new();
    // Two /64s of one /56 fit (2 × the cap), the third doesn't.
    for n in 1..=2 {
        let (ws, r) = via_proxy(addr, &format!("2001:db8:1:{n}::1"));
        assert!(matches!(r, ServerMsg::Hosting(_)), "/64 number {n}");
        held.push(ws);
    }
    assert!(matches!(via_proxy(addr, "2001:db8:1:3::1").1, ServerMsg::Refused { reason } if reason.contains("too many connections")));
    // Four of one /48 fit (4 × the cap) — two of them taken above — the fifth doesn't.
    for n in 1..=2u16 {
        let (ws, r) = via_proxy(addr, &format!("2001:db8:1:{:x}::1", n << 8));
        assert!(matches!(r, ServerMsg::Hosting(_)), "/56 number {n}");
        held.push(ws);
    }
    assert!(matches!(via_proxy(addr, "2001:db8:1:300::1").1, ServerMsg::Refused { .. }), "the /48 is full");
    assert!(matches!(via_proxy(addr, "2001:db8:2::1").1, ServerMsg::Hosting(_)), "another /48 isn't");
    drop(held);
    server.stop();
}

#[test]
fn rates_count_per_block_and_publishing_for_everyone() {
    let mut a = Abuse::new(AbuseConfig { open: true, connects_per_minute: 1, publishes_per_hour: 1, messages_per_second: 1 });
    let t = Instant::now();
    // One connection a minute per /64, two per /56, four per /48.
    let in_one_56 = (0..64u16).filter(|n| a.allow(IpAddr::from([0x2001, 0xdb8, 7, *n, 0, 0, 0, 0]), Action::Connect, t)).count();
    assert_eq!(in_one_56, 2);
    let in_one_48 = (1..64u16).filter(|n| a.allow(IpAddr::from([0x2001, 0xdb8, 7, n << 8, 0, 0, 0, 0]), Action::Connect, t)).count();
    assert_eq!(in_one_48, 2, "the /48's four, two of them taken above");
    // Publishing: one an hour per address, 32 for everyone together.
    let published = (0..60u32).filter(|n| a.allow(IpAddr::from((0xC000_0200 + n).to_be_bytes()), Action::Publish, t)).count();
    assert_eq!(published, 32);
}

#[test]
fn one_shot_requests_leave_connections_for_sessions() {
    let dir = TempDir::new("reserve");
    let cfg = Config { max_connections: 8, ..open(dir.path()) };
    assert_eq!(cfg.net().reserve_for_sessions, 1);
    let edit = {
        let mut store = Store::open(dir.path(), cfg.store()).unwrap();
        let (file, edit, _) = store.publish("Team", &snapshot("Team")).unwrap();
        (file.id, edit.token)
    };
    let mut server = crate::start(&cfg).unwrap();
    let addr = server.addr();
    let mut held = Vec::new();
    for _ in 0..7 {
        let mut raw = Raw::connect(addr);
        raw.send(&hosting());
        assert!(matches!(raw.next(), ServerMsg::Hosting(_)));
        held.push(raw);
    }
    let mut eighth = Raw::connect(addr);
    eighth.send(&hosting());
    assert!(matches!(eighth.next(), ServerMsg::Refused { reason } if reason.contains("busy")));
    drop(eighth);
    until(5, "the refused connection to close", || server.connections() <= 7);
    let mut session = Raw::connect(addr);
    session.send(&Client::hello(&edit.0, &edit.1, "Ada", ""));
    assert!(matches!(session.next(), ServerMsg::Welcome { .. }), "a session still gets in");
    server.stop();
}

// M4 -------------------------------------------------------------------------------------------

#[test]
fn revoking_or_removing_cuts_library_connections_too() {
    let dir = TempDir::new("libkick");
    let cfg = config(dir.path());
    let (file, view) = {
        let mut store = Store::open(dir.path(), cfg.store()).unwrap();
        let (file, _, view) = store.publish("Library", &snapshot("Library")).unwrap();
        store.publish_library(&file.id, &archive(0, 0)).unwrap();
        (file.id, view)
    };
    let mut server = crate::start(&cfg).unwrap();
    let addr = server.addr();
    let invite = Invite { server: Endpoint::plain("x"), file: file.clone(), token: view.token.clone() };
    let mut lib = Raw::connect(addr);
    lib.send(&open_library(&invite));
    lib.next();
    let key = crate::test_util::key(dir.path());
    manage::request(
        &Endpoint::plain(addr.to_string()),
        &key,
        vec![Admin::Revoke { file: file.clone(), link: view.link.id.clone() }],
        &NetLimits::default(),
        Duration::from_secs(5),
    )
    .unwrap();
    assert!(matches!(lib.next(), ServerMsg::Refused { reason } if reason.contains("revoked")));
    // A library connection with another link is cut when the file is removed.
    let made = manage::request(
        &Endpoint::plain(addr.to_string()),
        &key,
        vec![Admin::CreateLink { file: file.clone(), access: Access::View, label: String::new() }],
        &NetLimits::default(),
        Duration::from_secs(5),
    )
    .unwrap();
    let Some(AdminReply::Link(l)) = made.into_iter().next() else { panic!() };
    let mut lib = Raw::connect(addr);
    lib.send(&open_library(&Invite { token: l.token, ..invite }));
    lib.next();
    manage::request(&Endpoint::plain(addr.to_string()), &key, vec![Admin::Remove { file }], &NetLimits::default(), Duration::from_secs(5)).unwrap();
    assert!(matches!(lib.next(), ServerMsg::Refused { reason } if reason.contains("removed")));
    server.stop();
}

// L6, L7 ---------------------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn only_the_server_reads_its_data_directory_and_key() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new("perms");
    let data = dir.path().join("data");
    let store = Store::open(&data, config(&data).store()).unwrap();
    drop(store);
    assert_eq!(std::fs::metadata(&data).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(std::fs::metadata(data.join("server.key")).unwrap().permissions().mode() & 0o777, 0o600);
}

#[test]
fn downloads_have_a_share_per_address() {
    let mut a = Abuse::new(AbuseConfig { open: false, connects_per_minute: 1, publishes_per_hour: 1, messages_per_second: 1 });
    let t = Instant::now();
    let me = ip("198.51.100.1");
    let chunk = 16 << 20;
    let served = (0..10).filter(|_| a.allow(me, Action::Download(chunk), t)).count();
    assert_eq!(served, (DOWNLOAD_BURST as usize) / chunk, "a burst, then wait");
    assert!(a.allow(ip("198.51.100.2"), Action::Download(chunk), t), "another address has its own share");
    // A team library is served within it, then refused, saying why; another address still gets it.
    let dir = TempDir::new("libshare");
    let mut store = Store::open(dir.path(), config(dir.path()).store()).unwrap();
    let (file, ..) = store.publish("Library", &snapshot("Library")).unwrap();
    store.publish_library(&file.id, &archive(0, 4 << 20)).unwrap();
    let fetch = |s: &mut Store, who: &str| s.library(ip(who), &file.id, Access::View, LibraryReq::Fetch { have: None, usage: None });
    let served = (0..40).take_while(|_| matches!(fetch(&mut store, "198.51.100.1"), LibraryReply::Library { .. })).count();
    assert!((3..=5).contains(&served), "{served} downloads of 4 MiB in a burst of 16 MiB");
    assert!(matches!(fetch(&mut store, "198.51.100.1"), LibraryReply::Failed { reason } if reason == BUSY_DOWNLOADS));
    assert!(matches!(fetch(&mut store, "198.51.100.9"), LibraryReply::Library { .. }));
}

// Second review: N2, L7, N3, M3, N5, N7 ------------------------------------------------------

#[test]
fn a_file_at_its_cap_is_compacted_at_most_once_a_minute() {
    let dir = TempDir::new("forced");
    let mut c = config(dir.path());
    c.max_file_mb = Some(1);
    let mut store = Store::open(dir.path(), c.store()).unwrap();
    let (info, ..) = store.publish("Full", &snapshot("Full")).unwrap();
    let mut server = Backend::open(&mut store, &info.id).unwrap();
    let mut client = join(&mut server);
    let before = store.compactions();
    // Alternate a small accepted batch with one that would take the file over its cap.
    for i in 0..50 {
        rename(&mut server, &mut client, &format!("page {i}"));
        Backend::changed(&mut store, &info.id, &mut server);
        assert!(Backend::room(&mut store, &info.id, &server, 2 << 20).is_err());
    }
    assert!(store.compactions() - before <= 1, "{} compactions for 50 over-cap batches", store.compactions() - before);
}

/// A plugin directory with one plugin whose bundle is about `mib` MiB.
fn plugin_store(dir: &std::path::Path, mib: usize) -> Store {
    let mut store = Store::open(dir, config(dir).store()).unwrap();
    let bundle = figcraft_plugin::bundle::Bundle {
        manifest: crate::directory_tests::manifest("org.example.big", "1.0.0"),
        wasm: crate::directory_tests::wasm(),
        assets: vec![figcraft_plugin::bundle::Asset { name: "data.bin".into(), data: vec![9; mib << 20] }],
    };
    store.directory().publish(&bundle.encode()).unwrap();
    store
}

#[test]
fn one_address_cant_spend_everyones_plugin_downloads() {
    let dir = TempDir::new("plugshare");
    let mut store = plugin_store(dir.path(), 4);
    let fetch = || PluginReq::Fetch { id: "org.example.big".into(), version: None };
    let greedy = ip("198.51.100.1");
    // Its share runs out after a few; then 200 more tries cost nobody else anything.
    let served =
        (0..10).take_while(|_| matches!(store.plugins(greedy, false, fetch()), figcraft_collab::message::PluginReply::Bundle { .. })).count();
    assert!((2..=5).contains(&served), "{served}");
    for _ in 0..200 {
        let r = store.plugins(greedy, false, fetch());
        assert!(matches!(&r, figcraft_collab::message::PluginReply::Failed { reason } if reason == BUSY_DOWNLOADS));
    }
    assert!(
        matches!(store.plugins(ip("198.51.100.2"), false, fetch()), figcraft_collab::message::PluginReply::Bundle { .. }),
        "others still download"
    );
}

#[test]
fn half_read_messages_count_against_a_reading_budget() {
    let dir = TempDir::new("reading");
    let cfg = open(dir.path());
    let store = Store::open(dir.path(), cfg.store()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut busy = Hosting::serve(listener, store, NetLimits { max_reading_block: 64 << 10, max_reading_total: 1 << 20, ..cfg.net() }).unwrap();
    let mut raw = Raw::connect(busy.addr());
    raw.send(&hosting());
    assert!(matches!(raw.next(), ServerMsg::Hosting(_)), "small messages are read");
    let mut big = Document::new("Big");
    let page = big.first_page().unwrap();
    big.create(page, NodeKind::Rectangle, &"r".repeat(200_000), 0.0, 0.0, 1.0, 1.0).unwrap();
    let _ = raw.try_send(&ClientMsg::Admin(Admin::Publish { name: "Big".into(), snapshot: Replica::new(big).snapshot().unwrap() }));
    assert!(matches!(raw.next(), ServerMsg::Refused { reason } if reason.contains("busy reading")));
    until(10, "the reading budget to be given back", || busy.connections() == 0);
    // Given back: another address reads again.
    let mut raw = Raw::connect(busy.addr());
    raw.send(&hosting());
    assert!(matches!(raw.next(), ServerMsg::Hosting(_)));
    busy.stop();
}

#[test]
fn one_ipv6_block_takes_at_most_an_eighth_of_the_connections() {
    let dir = TempDir::new("eighth");
    let cfg = Config { max_connections: 64, trusted_proxy: Some("127.0.0.1".into()), ..open(dir.path()) };
    let mut server = crate::start(&cfg).unwrap();
    let addr = server.addr();
    let mut held = Vec::new();
    for n in 1..=8u16 {
        let (ws, r) = via_proxy(addr, &format!("2001:db8:9:{:x}::1", n << 8));
        assert!(matches!(r, ServerMsg::Hosting(_)), "connection {n}");
        held.push(ws);
    }
    assert!(matches!(via_proxy(addr, "2001:db8:9:900::1").1, ServerMsg::Refused { reason } if reason.contains("too many connections")), "64 / 8");
    assert!(matches!(via_proxy(addr, "2001:db8:a::1").1, ServerMsg::Hosting(_)), "another /48 still gets in");
    drop(held);
    server.stop();
}

#[test]
fn no_block_can_use_up_a_shared_budget() {
    let mut a = Abuse::new(AbuseConfig { open: true, connects_per_minute: 30, publishes_per_hour: 10, messages_per_second: 200 });
    let t = Instant::now();
    // A /48 publishes at most an eighth of what everyone may; everyone else keeps the rest.
    let from_48 = (0..200u16).filter(|n| a.allow(IpAddr::from([0x2001, 0xdb8, 5, n << 8, 0, 0, 0, 0]), Action::Publish, t)).count();
    assert_eq!(from_48, 40, "four addresses' worth");
    let others = (0..400u32).filter(|n| a.allow(IpAddr::from((0xC633_6400 + n).to_be_bytes()), Action::Publish, t)).count();
    assert_eq!(others, 320 - 40, "the rest of everyone's 320 an hour");
    // Downloads: a /48's burst is at most an eighth of everyone's (checked when compiling, in `store`).
}

#[test]
fn problems_stay_bounded_and_a_full_disk_refuses_writes() {
    let dir = TempDir::new("problems");
    let mut store = Store::open(dir.path(), config(dir.path()).store()).unwrap();
    let (info, ..) = store.publish("Broken", &snapshot("Broken")).unwrap();
    std::fs::write(dir.path().join("files").join(&info.id).join("snapshot.bin"), b"junk").unwrap();
    for _ in 0..500 {
        assert!(Backend::open(&mut store, &info.id).is_err());
    }
    assert!(store.problems().len() <= 200, "{} problems kept", store.problems().len());
    drop(store);
    // Where the free space can be told, below the threshold nothing more is written.
    if crate::disk::free_bytes(dir.path()).is_some() {
        let mut c = config(dir.path()).store();
        c.min_free_bytes = u64::MAX;
        let mut store = Store::open(dir.path(), c).unwrap();
        let e = store.publish("More", &snapshot("More")).unwrap_err();
        assert!(e.contains("disk is nearly full"), "{e}");
    }
}

// Third review: R1, N3, R2 -------------------------------------------------------------------

/// Read messages until `pick` takes one (others in between are skipped).
fn wait_for<T>(raw: &mut Raw, what: &str, mut pick: impl FnMut(ServerMsg) -> Option<T>) -> T {
    loop {
        match raw.next() {
            ServerMsg::Refused { reason } => panic!("refused while waiting for {what}: {reason}"),
            m => {
                if let Some(t) = pick(m) {
                    return t;
                }
            }
        }
    }
}

#[test]
fn an_office_behind_one_address_works_together() {
    let dir = TempDir::new("office");
    let cfg = Config { max_file_mb: Some(4), max_plugin_mb: 1, ..open(dir.path()) };
    let mut server = crate::start(&cfg).unwrap();
    let addr = server.addr();
    // A file near the size limit.
    let mut d = Document::new("Office");
    let page = d.first_page().unwrap();
    d.create(page, NodeKind::Rectangle, &"o".repeat(3_000_000), 0.0, 0.0, 10.0, 10.0).unwrap();
    let (_, replies) = manage::hosting(
        &Endpoint::plain(addr.to_string()),
        "",
        "",
        vec![Admin::Publish { name: "Office".into(), snapshot: Replica::new(d).snapshot().unwrap() }],
        &NetLimits::default(),
        Duration::from_secs(10),
    )
    .unwrap();
    let Some(AdminReply::Hosted { file, edit, .. }) = replies.into_iter().next() else { panic!("not hosted") };
    // Twenty colleagues, one address, all joining at once: twenty welcomes of 3 MB.
    let mut people: Vec<Raw> = (0..20).map(|_| Raw::connect(addr)).collect();
    for (i, p) in people.iter_mut().enumerate() {
        p.send(&Client::hello(&file.id, &edit.token, &format!("Person {i}"), ""));
    }
    let mut clients: Vec<Client> = people
        .iter_mut()
        .map(|p| Client::join(wait_for(p, "the welcome", |m| matches!(m, ServerMsg::Welcome { .. }).then_some(m))).unwrap())
        .collect();
    // Everyone moves their cursor (twenty people at ~30 a second share the address's budget).
    for p in people.iter_mut() {
        for k in 0..60 {
            p.send(&ClientMsg::Presence(figcraft_collab::Presence { cursor: Some((page, f64::from(k), 0.0)), ..Default::default() }));
        }
    }
    // One pastes 1 MB: the broadcast reaches all twenty.
    let mut doc = clients[0].document().clone();
    doc.create(page, NodeKind::Rectangle, &"p".repeat(1_000_000), 0.0, 0.0, 5.0, 5.0).unwrap();
    let batch = clients[0].edit(&doc).unwrap().unwrap();
    people[0].send(&batch);
    for (i, p) in people.iter_mut().enumerate() {
        let seq = wait_for(p, "the paste", |m| {
            if let ServerMsg::Changes(a) = m {
                a.changes.iter().any(|c| matches!(c, figcraft_collab::Change::Create { .. })).then_some(a.seq)
            } else {
                None
            }
        });
        assert!(seq > 0, "person {i}");
    }
    // Still all connected.
    for p in people.iter_mut() {
        p.send(&ClientMsg::Ping);
        wait_for(p, "the pong", |m| (m == ServerMsg::Pong).then_some(()));
    }
    assert_eq!(server.connections(), 20);
    drop(clients.drain(..));
    server.stop();
}

#[test]
fn the_biggest_reader_is_cut_off_when_everyones_reading_budget_runs_out() {
    count_connection_panics();
    let dir = TempDir::new("readers");
    let cfg = open(dir.path());
    let store = Store::open(dir.path(), cfg.store()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let limits = NetLimits { max_reading_block: 10 << 20, max_reading_total: 300_000, ..cfg.net() };
    let mut busy = Hosting::serve(listener, store, limits).unwrap();
    let addr = busy.addr();
    // A slow sender: it starts a 300 KB message, sends 250 KB of it and stops.
    let mut hog = Raw::connect(addr);
    hog.send(&hosting());
    assert!(matches!(hog.next(), ServerMsg::Hosting(_)));
    let mut frame = vec![0x82, 0x80 | 127];
    frame.extend_from_slice(&300_000u64.to_be_bytes());
    frame.extend_from_slice(&[1, 2, 3, 4]);
    frame.extend(std::iter::repeat_n(0x55, 250_000));
    hog.raw_write(&frame);
    until(10, "the server to read the hog's bytes", || busy.reading_bytes() >= 250_000);
    // Someone else sends a 100 KB message: the hog is cut off, not them.
    let mut other = Raw::connect(addr);
    other.send(&hosting());
    assert!(matches!(other.next(), ServerMsg::Hosting(_)));
    let mut d = Document::new("Other");
    let page = d.first_page().unwrap();
    d.create(page, NodeKind::Rectangle, &"q".repeat(100_000), 0.0, 0.0, 1.0, 1.0).unwrap();
    other.send(&ClientMsg::Admin(Admin::Publish { name: "Other".into(), snapshot: Replica::new(d).snapshot().unwrap() }));
    assert!(matches!(other.next(), ServerMsg::Admin(_)), "the message was read and answered");
    assert!(hog.closed_within(Duration::from_secs(10)), "the hog's connection was cut");
    until(10, "the hog's bytes to be given back", || busy.reading_bytes() == 0);
    assert_eq!(CONNECTION_PANICS.load(std::sync::atomic::Ordering::SeqCst), 0);
    busy.stop();
}

#[test]
fn the_server_forgets_addresses_whose_connections_are_gone() {
    let dir = TempDir::new("forget");
    let cfg = Config { trusted_proxy: Some("127.0.0.1".into()), ..open(dir.path()) };
    let mut server = crate::start(&cfg).unwrap();
    let addr = server.addr();
    for n in 1..=10u8 {
        let (ws, r) = via_proxy(addr, &format!("203.0.113.{n}"));
        assert!(matches!(r, ServerMsg::Hosting(_)));
        drop(ws);
    }
    until(10, "the connections to close", || server.connections() == 0);
    let (_last, _) = via_proxy(addr, "203.0.113.200");
    assert!(server.blocks_remembered() <= 2, "{} blocks remembered", server.blocks_remembered());
    server.stop();
}

#[test]
fn a_library_publish_that_fails_keeps_serving_the_old_version() {
    let dir = TempDir::new("libfail");
    let mut store = Store::open(dir.path(), config(dir.path()).store()).unwrap();
    let (file, ..) = store.publish("Library", &snapshot("Library")).unwrap();
    // Version 1, then a changed library as version 2.
    let mut s = Session::new();
    let f = s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 50, "height": 20, "name": "Button"})).unwrap()["id"].as_u64().unwrap();
    let c = s.execute("component.create", &json!({"ids": [f]})).unwrap()["id"].as_u64().unwrap();
    s.execute("library.publish", &json!({})).unwrap();
    let v1 = figcraft_engine::library_server::archive(&s.doc().unwrap().published.clone().unwrap()).unwrap();
    s.execute("node.set", &json!({"id": c, "props": {"fill": "#00ff00"}})).unwrap();
    s.execute("library.publish", &json!({})).unwrap();
    let v2 = figcraft_engine::library_server::archive(&s.doc().unwrap().published.clone().unwrap()).unwrap();
    assert_eq!(store.publish_library(&file.id, &v1).unwrap(), 1);
    let files = dir.path().join(crate::store::FILES).join(&file.id);
    // The description can't be written: a directory sits where its temporary file goes.
    let tmp = crate::disk::temp_of(&files.join(crate::disk::META));
    std::fs::create_dir(&tmp).unwrap();
    assert!(store.publish_library(&file.id, &v2).is_err());
    let fetch = |s: &mut Store| s.library(ip("198.51.100.1"), &file.id, Access::View, LibraryReq::Fetch { have: None, usage: None });
    let served = |reply: LibraryReply| match reply {
        LibraryReply::Library { version, archive } => (version, archive),
        other => panic!("{other:?}"),
    };
    assert_eq!(served(fetch(&mut store)), (1, v1.clone()), "version 1, with its own bytes");
    // A crash right after the archive was written leaves it on disk, the description unchanged.
    std::fs::write(files.join(crate::store::library_name(2)), &v2).unwrap();
    drop(store);
    std::fs::remove_dir(&tmp).unwrap();
    let mut store = Store::open(dir.path(), config(dir.path()).store()).unwrap();
    assert_eq!(served(fetch(&mut store)), (1, v1), "still version 1 after a restart");
    // Publishing again works, and serves the new version.
    assert_eq!(store.publish_library(&file.id, &v2).unwrap(), 2);
    assert_eq!(served(fetch(&mut store)), (2, v2));
    assert!(!files.join(crate::store::library_name(1)).exists(), "the old version is deleted");
}
