//! A server open to the public: anyone publishes without the key and manages the file with its
//! owner token; the caps per address and for everyone, rate limits, idle expiry, blocking,
//! reports, the uptime check, `X-Forwarded-For` only from the trusted proxy, and old clients.

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use figcraft_collab::message::{self, Admin, AdminReply, ClientMsg, OpenHosting, PROTOCOL, ServerMsg};
use figcraft_collab_net::{Endpoint, Invite, NetLimits, manage};
use serde_json::{Value, json};
use tungstenite::client::IntoClientRequest;
use tungstenite::{Bytes, Message};

use crate::abuse::{Abuse, AbuseConfig, Action, MAX_ADDRESSES};
use crate::cli::{self, Cli, Command, Sort};
use crate::config::Config;
use crate::store::{KEY_ONLY, Store};
use crate::test_util::{Raw, TempDir, config, ip, join, key, live, pump, session, snapshot};

fn open(dir: &std::path::Path) -> Config {
    Config { open_publish: true, operator_contact: "abuse@example.com".into(), terms_url: "https://example.com/terms".into(), ..config(dir) }
}

fn ex(s: &mut figcraft_engine::Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn cli(cfg: &Config, command: Command) -> Result<String, String> {
    cli::run(&Cli { config: cfg.clone(), command })
}

#[test]
fn anyone_publishes_and_the_owner_token_manages_the_file() {
    let dir = TempDir::new("public");
    let cfg = open(dir.path());
    let mut server = crate::start(&cfg).unwrap();
    let addr = server.addr();
    let url = format!("ws://{addr}/");
    let mut a = session("Ada");
    // The server says it is open, who runs it and its terms before anything is uploaded.
    let info = ex(&mut a, "collab.serverInfo", json!({"server": url}));
    assert_eq!(info["open"], true);
    assert_eq!(info["operator"], "abuse@example.com");
    assert_eq!(info["terms"], "https://example.com/terms");
    assert_eq!(info["idleDays"], 30.0);
    // Published without a key: links, an owner token shown once, and an expiry date.
    ex(&mut a, "collab.publish", json!({"server": url, "name": "Poster"}));
    pump(&mut [&mut a], "the publish", |s| live(s[0]) && s[0].live().published.is_some() && s[0].live().hosting.is_some());
    let p = a.live().published.clone().unwrap();
    let owner = p.owner.clone().expect("an owner token");
    assert_eq!(owner.len(), 22, "128 bits");
    assert!(p.expires.unwrap() > crate::store::now() + 29 * 86_400);
    let summary = a.live_summary();
    assert_eq!(summary["published"]["owner"], owner.as_str());
    assert_eq!(summary["hosting"]["operator"], "abuse@example.com");
    // The owner token is kept only as a hash.
    let meta = std::fs::read_to_string(dir.path().join("files").join(&p.file).join("meta.json")).unwrap();
    assert!(!meta.contains(&owner) && meta.contains(&crate::store::hash(&owner)) && meta.contains("127.0.0.1"), "{meta}");
    // Someone joining hears about the hosting too (with the file's expiry).
    let mut b = join(&p.edit, "Bo");
    pump(&mut [&mut a, &mut b], "Bo in, with the hosting", |s| live(s[1]) && s[1].live().hosting.as_ref().is_some_and(|h| h.expires.is_some()));
    // The owner lists, makes, revokes links and renames the file.
    let manage = |s: &mut figcraft_engine::Session, extra: Value| {
        let mut p2 = json!({"server": url, "file": p.file, "owner": owner});
        p2.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        s.execute("collab.manageFile", &p2)
    };
    let links = manage(&mut a, json!({"action": "links"})).unwrap();
    assert_eq!(links["links"].as_array().unwrap().len(), 2);
    let made = manage(&mut a, json!({"action": "createLink", "access": "view", "label": "Review"})).unwrap();
    assert!(made["link"].as_str().unwrap().starts_with("figcraft://join?host="), "{made}");
    let view_id = links["links"].as_array().unwrap().iter().find(|l| l["access"] == "view").unwrap()["id"].as_str().unwrap().to_string();
    assert_eq!(manage(&mut a, json!({"action": "revoke", "link": view_id})).unwrap()["done"], true);
    manage(&mut a, json!({"action": "rename", "name": "Poster v2"})).unwrap();
    assert!(cli(&cfg, Command::Files { sort: Sort::Size }).unwrap().contains("Poster v2"));
    // A wrong token, or another file's, manages nothing.
    let e = a.execute("collab.manageFile", &json!({"server": url, "file": p.file, "owner": "AAAAAAAAAAAAAAAAAAAAAA", "action": "links"}));
    assert!(e.unwrap_err().to_string().contains("doesn't manage"));
    let mut raw = Raw::connect(addr);
    raw.send(&ClientMsg::OpenHosting(OpenHosting { protocol: PROTOCOL, client_version: String::new(), file: p.file.clone(), owner: owner.clone() }));
    assert!(matches!(raw.next(), ServerMsg::Hosting(_)));
    raw.send(&ClientMsg::Admin(Admin::Remove { file: "0000000000000000".into() }));
    assert!(matches!(raw.next(), ServerMsg::Admin(r) if matches!(*r, AdminReply::Failed { ref reason } if reason.contains("only its own"))));
    raw.send(&ClientMsg::Admin(Admin::Listing));
    assert!(matches!(raw.next(), ServerMsg::Admin(r) if matches!(*r, AdminReply::Failed { ref reason } if reason.contains("server key"))));
    // Removing it disconnects everyone in it (whoever removed it just leaves: no failure there).
    assert_eq!(manage(&mut a, json!({"action": "remove"})).unwrap()["done"], true);
    pump(&mut [&mut a, &mut b], "everyone told", |s| s[0].live_summary()["status"] == "off" && s[1].live_summary()["status"] == "failed");
    assert!(b.message.as_deref().unwrap_or_default().contains("removed"), "{:?}", b.message);
    assert!(!dir.path().join("files").join(&p.file).exists());
    server.stop();
}

#[test]
fn a_private_server_still_needs_its_key() {
    let dir = TempDir::new("private");
    let mut server = crate::start(&config(dir.path())).unwrap();
    let url = format!("ws://{}/", server.addr());
    let mut a = session("Ada");
    let info = ex(&mut a, "collab.serverInfo", json!({"server": url}));
    assert_eq!(info["open"], false);
    ex(&mut a, "collab.publish", json!({"server": url}));
    pump(&mut [&mut a], "the refusal", |s| s[0].live_summary()["status"] == "failed");
    assert!(a.message.as_deref().unwrap_or_default().contains("only from people who have its key"), "{:?}", a.message);
    assert!(!dir.path().join("files").read_dir().unwrap().any(|_| true), "nothing kept");
    // The operator's key publishes as before (no owner token, no expiry).
    let p = crate::test_util::publish(&mut a, dir.path(), server.addr());
    assert!(p.owner.is_none() && p.expires.is_none());
    server.stop();
}

#[test]
fn caps_per_address_and_for_everyone() {
    let dir = TempDir::new("caps");
    let mut c = open(dir.path());
    c.max_files_per_ip = 2;
    c.publishes_per_hour = 100;
    let mut store = Store::open(dir.path(), c.store()).unwrap();
    let (a, b) = (ip("203.0.113.1"), ip("203.0.113.2"));
    store.publish_open(a, "One", &snapshot("One")).unwrap();
    store.publish_open(a, "Two", &snapshot("Two")).unwrap();
    let e = store.publish_open(a, "Three", &snapshot("Three")).unwrap_err();
    assert!(e.contains("keeps 2 files"), "{e}");
    store.publish_open(b, "Other", &snapshot("Other")).unwrap();
    drop(store);
    // Bytes per address, and for everyone (a byte: nothing fits).
    let mut tight = c.store();
    tight.hosting.max_bytes_per_ip = 1;
    let mut store = Store::open(dir.path(), tight.clone()).unwrap();
    let e = store.publish_open(ip("203.0.113.3"), "Big", &snapshot("Big")).unwrap_err();
    assert!(e.contains("MiB it may keep"), "{e}");
    drop(store);
    tight.hosting.max_bytes_per_ip = u64::MAX;
    tight.hosting.max_total_bytes = 1;
    let mut store = Store::open(dir.path(), tight).unwrap();
    let e = store.publish_open(ip("203.0.113.3"), "Big", &snapshot("Big")).unwrap_err();
    assert!(e.contains("server is full"), "{e}");
    drop(store);
    // Publishing too often.
    let mut c3 = open(dir.path());
    c3.publishes_per_hour = 1;
    let mut store = Store::open(dir.path(), c3.store()).unwrap();
    store.publish_open(ip("198.51.100.7"), "Once", &snapshot("Once")).unwrap();
    let e = store.publish_open(ip("198.51.100.7"), "Twice", &snapshot("Twice")).unwrap_err();
    assert!(e.contains("too many files published"), "{e}");
    // Closed to the public: nobody publishes without the key.
    drop(store);
    let mut store = Store::open(dir.path(), config(dir.path()).store()).unwrap();
    assert_eq!(store.publish_open(ip("198.51.100.8"), "No", &snapshot("No")).unwrap_err(), KEY_ONLY);
}

#[test]
fn caps_per_address_count_the_wider_ipv6_blocks() {
    let dir = TempDir::new("block-caps");
    let mut c = open(dir.path());
    c.max_files_per_ip = 1;
    c.publishes_per_hour = 1000;
    let mut store = Store::open(dir.path(), c.store()).unwrap();
    // A /48 keeps 4 times one address's files: /64s in different /56s of it.
    for i in 0..4 {
        store.publish_open(ip(&format!("2001:db8:1:{:x}::1", i << 8)), "One", &snapshot("One")).unwrap();
    }
    let e = store.publish_open(ip("2001:db8:1:400::1"), "Five", &snapshot("Five")).unwrap_err();
    assert!(e.contains("files on this server already"), "{e}");
    // A /56 keeps twice one address's files.
    store.publish_open(ip("2001:db8:2:1::1"), "One", &snapshot("One")).unwrap();
    store.publish_open(ip("2001:db8:2:2::1"), "Two", &snapshot("Two")).unwrap();
    let e = store.publish_open(ip("2001:db8:2:3::1"), "Three", &snapshot("Three")).unwrap_err();
    assert!(e.contains("files on this server already"), "{e}");
    drop(store);
    // Bytes: one address keeps one file's worth, so a /48 keeps 4 files.
    let probe = TempDir::new("block-caps-probe");
    let mut p = Store::open(probe.path(), open(probe.path()).store()).unwrap();
    let size = p.publish_open(ip("203.0.113.1"), "One", &snapshot("One")).unwrap().0.bytes;
    let dir = TempDir::new("block-caps-bytes");
    let mut s = open(dir.path()).store();
    s.hosting.max_files_per_ip = 100;
    s.hosting.max_bytes_per_ip = size;
    let mut store = Store::open(dir.path(), s).unwrap();
    for i in 0..4 {
        store.publish_open(ip(&format!("2001:db8:3:{:x}::1", i << 8)), "One", &snapshot("One")).unwrap();
    }
    let e = store.publish_open(ip("2001:db8:3:400::1"), "One", &snapshot("One")).unwrap_err();
    assert!(e.contains("MiB it may keep"), "{e}");
}

#[test]
fn edits_count_the_wider_ipv6_blocks_too() {
    use figcraft_collab_net::hub::Backend;
    let probe = TempDir::new("block-room-probe");
    let mut p = Store::open(probe.path(), open(probe.path()).store()).unwrap();
    let size = p.publish_open(ip("203.0.113.1"), "One", &snapshot("One")).unwrap().0.bytes;
    let dir = TempDir::new("block-room");
    let mut s = open(dir.path()).store();
    s.hosting.max_files_per_ip = 100;
    // One address keeps 2.25 files' worth, so its /56 keeps 4.5.
    s.hosting.max_bytes_per_ip = size * 2 + size / 4;
    let mut store = Store::open(dir.path(), s).unwrap();
    let mut ids = Vec::new();
    for i in 0..4 {
        ids.push(store.publish_open(ip(&format!("2001:db8:4:{i:x}::1")), "One", &snapshot("One")).unwrap().0.id);
    }
    let server = Backend::open(&mut store, &ids[0]).unwrap();
    // Its own /64 has room for this, but its /56 doesn't.
    let e = Backend::room(&mut store, &ids[0], &server, (size * 3 / 4) as usize).unwrap_err();
    assert!(e.contains("have reached their"), "{e}");
    Backend::room(&mut store, &ids[0], &server, (size / 4) as usize).unwrap();
}

#[test]
fn idle_files_published_without_the_key_expire() {
    let dir = TempDir::new("expiry");
    let mut c = open(dir.path());
    c.idle_days = 2.0;
    let mut store = Store::open(dir.path(), c.store()).unwrap();
    let (public, ..) = store.publish_open(ip("203.0.113.9"), "Public", &snapshot("Public")).unwrap();
    let (fresh, ..) = store.publish_open(ip("203.0.113.9"), "Fresh", &snapshot("Fresh")).unwrap();
    let (keyed, ..) = store.publish("Operator's", &snapshot("Operator's")).unwrap();
    store.age(&public.id, 2 * 86_400 + 1);
    store.age(&keyed.id, 365 * 86_400);
    store.age(&fresh.id, 86_400);
    assert_eq!(store.sweep(), vec![public.id.clone()]);
    let ids: Vec<String> = store.listing(&|_| None).into_iter().map(|r| r.info.id).collect();
    assert!(!ids.contains(&public.id) && ids.contains(&fresh.id) && ids.contains(&keyed.id));
    assert!(!dir.path().join("files").join(&public.id).exists());
    // Opening it counts as using it: idle time starts over.
    let mut server = figcraft_collab_net::hub::Backend::open(&mut store, &fresh.id).unwrap();
    store.age(&fresh.id, 0);
    assert!(store.listing(&|_| None).iter().any(|r| r.info.id == fresh.id && r.expires.unwrap() > crate::store::now() + 86_400));
    // An open file never expires under someone.
    store.age(&fresh.id, 10 * 86_400);
    assert!(store.sweep().is_empty());
    figcraft_collab_net::hub::Backend::idle(&mut store, &fresh.id, &mut server);
    assert!(store.sweep().is_empty(), "everyone just left: the idle time starts now");
    // A startup finds what expired while the server was down.
    store.age(&fresh.id, 3 * 86_400);
    drop(store);
    let store = Store::open(dir.path(), c.store()).unwrap();
    assert!(store.listing(&|_| None).iter().all(|r| r.info.id != fresh.id));
}

#[test]
fn offline_commands_do_not_sweep_idle_files() {
    let dir = TempDir::new("offline-sweep");
    let mut c = open(dir.path());
    c.idle_days = 30.0;
    let mut store = Store::open(dir.path(), c.store()).unwrap();
    let (public, ..) = store.publish_open(ip("203.0.113.9"), "Public", &snapshot("Public")).unwrap();
    store.age(&public.id, 31 * 86_400);
    drop(store);
    let store = Store::try_open(dir.path(), c.store()).unwrap().unwrap();
    drop(store);
    assert!(dir.path().join("files").join(&public.id).exists(), "a command deleted an idle file");
    let _serving = Store::open(dir.path(), c.store()).unwrap();
    assert!(!dir.path().join("files").join(&public.id).exists());
}

#[test]
fn rate_limits_and_blocks_refuse_with_a_reason() {
    let dir = TempDir::new("limits");
    let mut c = open(dir.path());
    c.connects_per_minute = 3;
    let mut server = crate::start(&c).unwrap();
    let addr = server.addr();
    let ask = || {
        let mut raw = Raw::connect(addr);
        raw.send(&ClientMsg::OpenHosting(OpenHosting {
            protocol: PROTOCOL,
            client_version: String::new(),
            file: String::new(),
            owner: String::new(),
        }));
        raw.next()
    };
    for _ in 0..3 {
        assert!(matches!(ask(), ServerMsg::Hosting(_)));
    }
    assert!(matches!(ask(), ServerMsg::Refused { reason } if reason.contains("too many connections from your address")));
    server.stop();
    // Messages per address, and blocking.
    let mut c = open(dir.path());
    c.messages_per_second = 5;
    let mut server = crate::start(&c).unwrap();
    let addr = server.addr();
    let mut raw = Raw::connect(addr);
    raw.send(&ClientMsg::OpenHosting(OpenHosting { protocol: PROTOCOL, client_version: String::new(), file: String::new(), owner: String::new() }));
    assert!(matches!(raw.next(), ServerMsg::Hosting(_)));
    let refused = (0..200).find_map(|_| {
        raw.send(&ClientMsg::Ping);
        match raw.next() {
            ServerMsg::Refused { reason } => Some(reason),
            _ => None,
        }
    });
    assert!(refused.is_some_and(|r| r.contains("too many messages from your address")));
    let mut held = Raw::connect(addr);
    held.send(&ClientMsg::OpenHosting(OpenHosting { protocol: PROTOCOL, client_version: String::new(), file: String::new(), owner: String::new() }));
    assert!(matches!(held.next(), ServerMsg::Hosting(_)));
    assert!(cli(&c, Command::Block { ip: "127.0.0.1".into() }).unwrap().contains("Blocked"));
    assert!(matches!(held.next(), ServerMsg::Refused { reason } if reason.contains("no longer accepts")));
    let mut again = Raw::connect(addr);
    again.send(&ClientMsg::OpenHosting(OpenHosting { protocol: PROTOCOL, client_version: String::new(), file: String::new(), owner: String::new() }));
    assert!(matches!(again.next(), ServerMsg::Refused { .. }));
    assert_eq!(cli(&c, Command::Blocked).unwrap(), "127.0.0.1\n");
    server.stop();
    // Kept across a restart (the commands work on the directory itself then).
    assert_eq!(cli(&c, Command::Blocked).unwrap(), "127.0.0.1\n");
    assert!(cli(&c, Command::Unblock { ip: "127.0.0.1".into() }).unwrap().contains("Unblocked"));
    assert!(cli(&c, Command::Block { ip: "2001:db8:1:2:3:4:5:6".into() }).unwrap().contains("Blocked"));
    assert_eq!(cli(&c, Command::Blocked).unwrap(), "2001:db8:1:2::\n", "an IPv6 address blocks its /64");
    assert!(cli(&c, Command::Block { ip: "not an address".into() }).is_err());
}

#[test]
fn participants_report_files_to_the_operator() {
    let dir = TempDir::new("reports");
    let cfg = open(dir.path());
    let mut server = crate::start(&cfg).unwrap();
    let url = format!("ws://{}/", server.addr());
    let mut a = session("Ada");
    ex(&mut a, "collab.publish", json!({"server": url, "name": "Flyer"}));
    pump(&mut [&mut a], "the publish", |s| live(s[0]) && s[0].live().published.is_some());
    let p = a.live().published.clone().unwrap();
    let mut b = join(&p.view, "Bo");
    pump(&mut [&mut a, &mut b], "Bo in", |s| live(s[1]) && s[1].live().hosting.is_some());
    assert!(b.execute("collab.report", &json!({"reason": "   "})).is_err());
    ex(&mut b, "collab.report", json!({"reason": "Spam\nwith a link"}));
    pump(&mut [&mut b], "the answer", |s| s[0].message.as_deref().is_some_and(|m| m.contains("operator")));
    let reports = cli(&cfg, Command::Reports).unwrap();
    assert!(reports.contains(&p.file) && reports.contains("Spam with a link") && reports.contains("Flyer"), "{reports}");
    assert!(cli(&cfg, Command::Files { sort: Sort::Idle }).unwrap().lines().any(|l| l.contains(&p.file) && l.contains("127.0.0.1")));
    server.stop();
    // Kept on disk.
    assert!(cli(&cfg, Command::Reports).unwrap().contains("Spam with a link"));
}

/// Connect with `X-Forwarded-For: forwarded` and open a hosting connection: what the server said.
pub(crate) fn via_proxy(addr: SocketAddr, forwarded: &str) -> (tungstenite::WebSocket<TcpStream>, ServerMsg) {
    let s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut req = format!("ws://{addr}/").into_client_request().unwrap();
    req.headers_mut().insert("X-Forwarded-For", forwarded.parse().unwrap());
    let (mut ws, _) = tungstenite::client::client(req, s).unwrap();
    let open = ClientMsg::OpenHosting(OpenHosting { protocol: PROTOCOL, client_version: String::new(), file: String::new(), owner: String::new() });
    ws.send(Message::Binary(Bytes::from(message::encode(&open).unwrap()))).unwrap();
    let reply = loop {
        if let Message::Binary(b) = ws.read().unwrap() {
            break message::decode(&b, usize::MAX).unwrap();
        }
    };
    (ws, reply)
}

#[test]
fn forwarded_addresses_count_only_from_the_trusted_proxy() {
    let dir = TempDir::new("proxy");
    let mut c = open(dir.path());
    c.max_connections_per_ip = 1;
    c.trusted_proxy = Some("127.0.0.1".into());
    let mut server = crate::start(&c).unwrap();
    let addr = server.addr();
    // Behind the proxy each client is its own address: the last one the proxy added.
    let (_one, r) = via_proxy(addr, "203.0.113.1");
    assert!(matches!(r, ServerMsg::Hosting(_)));
    let (_two, r) = via_proxy(addr, "10.0.0.1, 203.0.113.2");
    assert!(matches!(r, ServerMsg::Hosting(_)));
    let (_three, r) = via_proxy(addr, "203.0.113.2, 203.0.113.1");
    assert!(matches!(r, ServerMsg::Refused { reason } if reason.contains("too many connections")));
    // Blocking the forwarded address blocks that client only.
    cli(&c, Command::Block { ip: "203.0.113.9".into() }).unwrap();
    assert!(matches!(via_proxy(addr, "203.0.113.9").1, ServerMsg::Refused { .. }));
    assert!(matches!(via_proxy(addr, "203.0.113.10").1, ServerMsg::Hosting(_)));
    server.stop();
    // Without a trusted proxy the header is anyone's to forge: the connection's own address counts.
    c.trusted_proxy = None;
    let mut server = crate::start(&c).unwrap();
    let addr = server.addr();
    let (_one, r) = via_proxy(addr, "203.0.113.1");
    assert!(matches!(r, ServerMsg::Hosting(_)));
    let (_two, r) = via_proxy(addr, "203.0.113.2");
    assert!(matches!(r, ServerMsg::Refused { .. }));
    server.stop();
}

/// Send `request` on a raw socket; what came back.
fn http(addr: SocketAddr, request: &str) -> String {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s.write_all(request.as_bytes()).unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out
}

#[test]
fn uptime_checks_share_the_port() {
    let dir = TempDir::new("healthz");
    let mut server = crate::start(&config(dir.path())).unwrap();
    let addr = server.addr();
    let ok = http(addr, "GET /healthz HTTP/1.1\r\nHost: x\r\n\r\n");
    assert!(ok.starts_with("HTTP/1.1 200 OK") && ok.ends_with("ok\n"), "{ok}");
    // Under a path (behind a proxy's prefix), and with a query.
    assert!(http(addr, "GET /figcraft/healthz?x=1 HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 200"));
    // Anything else is a WebSocket handshake or nothing.
    assert!(!http(addr, "GET /healthzz HTTP/1.1\r\n\r\n").contains("200 OK"));
    assert!(!http(addr, "POST /healthz HTTP/1.1\r\n\r\n").contains("200 OK"));
    // Sessions still work beside it.
    let mut a = session("Ada");
    let p = crate::test_util::publish(&mut a, dir.path(), addr);
    assert!(!p.edit.is_empty());
    server.stop();
}

#[test]
fn older_clients_get_clear_answers() {
    let dir = TempDir::new("old");
    let mut server = crate::start(&open(dir.path())).unwrap();
    let addr = server.addr();
    // An older app tries to publish with a wrong key: it is told the server is open.
    let e =
        manage::request(&Endpoint::plain(addr.to_string()), "wrong", vec![Admin::Files], &NetLimits::default(), Duration::from_secs(5)).unwrap_err();
    assert!(e.contains("open to the public"), "{e}");
    // A hello that doesn't announce `hosting` is never sent what it can't read: after its
    // welcome, the answer to its ping comes before anything about hosting would.
    let mut a = session("Ada");
    let p = crate::test_util::publish(&mut a, dir.path(), addr);
    let edit = Invite::parse(&p.edit).unwrap();
    let mut raw = Raw::connect(addr);
    let mut hello = figcraft_collab::Client::hello(&edit.file, &edit.token, "Old", "");
    if let ClientMsg::Hello(h) = &mut hello {
        h.client_version = "0.3.0 signals".into();
    }
    raw.send(&hello);
    raw.send(&ClientMsg::Ping);
    let seen: Vec<ServerMsg> = std::iter::from_fn(|| Some(raw.next())).take_while(|m| *m != ServerMsg::Pong).collect();
    assert!(seen.iter().any(|m| matches!(m, ServerMsg::Welcome { .. })));
    assert!(!seen.iter().any(|m| matches!(m, ServerMsg::Hosting(_))), "{seen:?}");
    // The operator's key still manages everything.
    let ok =
        manage::request(&Endpoint::plain(addr.to_string()), &key(dir.path()), vec![Admin::Listing], &NetLimits::default(), Duration::from_secs(5))
            .unwrap();
    assert!(matches!(&ok[..], [AdminReply::Listing(rows)] if rows.len() == 1));
    server.stop();
}

#[test]
fn the_address_table_stays_bounded() {
    let mut abuse = Abuse::new(AbuseConfig { open: true, connects_per_minute: 1, publishes_per_hour: 1, messages_per_second: 1 });
    let t = Instant::now();
    for i in 0..(MAX_ADDRESSES as u32 * 3) {
        assert!(abuse.allow(IpAddr::from(i.to_be_bytes()), Action::Connect, t + Duration::from_micros(u64::from(i))));
    }
    assert!(abuse.len() <= MAX_ADDRESSES);
    // A remembered address is limited; a private server limits only reports.
    let me = ip("192.0.2.1");
    assert!(abuse.allow(me, Action::Connect, t) && !abuse.allow(me, Action::Connect, t));
    let mut closed = Abuse::new(AbuseConfig { open: false, connects_per_minute: 1, publishes_per_hour: 1, messages_per_second: 1 });
    assert!((0..100).all(|_| closed.allow(me, Action::Connect, t)));
    assert!((0..10).all(|_| closed.allow(me, Action::Report, t)) && !closed.allow(me, Action::Report, t));
}

#[test]
fn open_mode_has_safe_defaults_and_warns_the_operator() {
    let private = Config::default();
    assert_eq!(private.file_mb(), 64, "a private server behaves as before");
    assert!(private.warnings().is_empty());
    assert_eq!(private.net().max_per_address, 0);
    let open = Config { open_publish: true, public_url: Some("wss://figcraft.example.com/".into()), ..Config::default() };
    assert_eq!(open.file_mb(), 16, "strangers' files stay small");
    assert_eq!(open.store().hosting.idle_secs, 30 * 86_400, "idle files expire");
    assert_eq!(open.net().max_per_address, 32);
    assert_eq!(open.net().request_idle, Duration::from_secs(20), "idle one-shot requests go quickly");
    assert_eq!(open.net().reserve_for_sessions, 512 / 8);
    assert_eq!(open.net().max_client_message, 17 << 20);
    assert_eq!(private.net().max_client_message, 65 << 20, "private: a 64 MiB file and a margin");
    let w = open.warnings().join("\n");
    assert!(w.contains("operator_contact is empty") && w.contains("trusted_proxy"), "{w}");
    let set = Config { operator_contact: "abuse@example.com".into(), trusted_proxy: Some("127.0.0.1".into()), max_file_mb: Some(8), ..open };
    assert!(set.warnings().is_empty(), "{:?}", set.warnings());
    assert_eq!(set.file_mb(), 8);
    assert!(Config { trusted_proxy: Some("localhost".into()), ..set.clone() }.check().is_err());
    assert!(Config { terms_url: "javascript:alert(1)".into(), ..set.clone() }.check().is_err());
    assert!(Config { idle_days: f64::NAN, ..set }.check().is_err());
}

#[test]
fn open_connections_never_reach_what_needs_the_key() {
    let dir = TempDir::new("keyonly");
    let mut server = crate::start(&open(dir.path())).unwrap();
    let addr = server.addr();
    let mut store_side = session("Ada");
    let keyed = crate::test_util::publish(&mut store_side, dir.path(), addr);
    let other = Invite::parse(&keyed.edit).unwrap().file;
    // A file of one's own, with its owner token.
    let (info, replies) = manage::hosting(
        &Endpoint::plain(addr.to_string()),
        "",
        "",
        vec![Admin::Publish { name: "Mine".into(), snapshot: snapshot("Mine") }],
        &NetLimits::default(),
        Duration::from_secs(5),
    )
    .unwrap();
    assert!(info.open);
    let Some(AdminReply::Hosted { file: mine, owner, .. }) = replies.into_iter().next() else { panic!("not hosted") };
    let key_only = vec![
        Admin::Files,
        Admin::Listing,
        Admin::Reports,
        Admin::Blocked,
        Admin::Block { ip: "203.0.113.1".into() },
        Admin::Unblock { ip: "203.0.113.1".into() },
        Admin::Links { file: other.clone() },
        Admin::CreateLink { file: other.clone(), access: figcraft_collab::Access::Edit, label: String::new() },
        Admin::Revoke { file: other.clone(), link: "x".into() },
        Admin::Rename { file: other.clone(), name: "Taken".into() },
        Admin::Remove { file: other.clone() },
    ];
    // Without a token, and with the token of another file: every one is refused.
    for (file, token) in [("", ""), (mine.id.as_str(), owner.as_str())] {
        let (_, replies) =
            manage::hosting(&Endpoint::plain(addr.to_string()), file, token, key_only.clone(), &NetLimits::default(), Duration::from_secs(5))
                .unwrap();
        for (req, r) in key_only.iter().zip(replies) {
            assert!(matches!(r, AdminReply::Failed { .. }), "{req:?} answered {r:?}");
        }
    }
    // The keyed file is untouched (its name, both links), nothing is blocked, and the plugin
    // directory won't take a bundle.
    let k = key(dir.path());
    let check = manage::request(
        &Endpoint::plain(addr.to_string()),
        &k,
        vec![Admin::Listing, Admin::Links { file: other.clone() }],
        &NetLimits::default(),
        Duration::from_secs(5),
    )
    .unwrap();
    assert!(matches!(&check[0], AdminReply::Listing(rows) if rows.iter().any(|r| r.info.id == other && r.info.name == "Team file")));
    assert!(matches!(&check[1], AdminReply::Links(links) if links.len() == 2));
    assert!(std::fs::read_to_string(dir.path().join("blocked.json")).is_err());
    let e = manage::plugins(
        &Endpoint::plain(addr.to_string()),
        "",
        figcraft_collab::message::PluginReq::Publish { bundle: vec![1, 2, 3] },
        &NetLimits::default(),
        Duration::from_secs(5),
    )
    .unwrap();
    assert!(matches!(e, figcraft_collab::message::PluginReply::Failed { .. }), "{e:?}");
    // Open-mode answers never carry the server key.
    assert!(!format!("{info:?}").contains(&k));
    server.stop();
}

#[test]
fn files_above_64_mib_fit_every_limit() {
    // `max_file_mb` may go up to 4096: every cap a whole file's message meets on its way in must
    // follow it, or the upload is dropped by the transport before the server can say why.
    for open_publish in [false, true] {
        let cfg = Config { open_publish, max_file_mb: Some(256), public_url: Some("wss://figcraft.example.com/".into()), ..Config::default() };
        let (net, core, file) = (cfg.net(), cfg.store().core, 256usize << 20);
        assert!(net.max_client_message > file, "open={open_publish}: {}", net.max_client_message);
        assert!(net.max_queued_bytes > file && net.max_reading_block > file && net.max_in_flight > file, "open={open_publish}: {net:?}");
        assert!(net.max_reading_total >= net.max_reading_block && net.max_queued_total >= net.max_queued_block);
        assert!(core.max_message_bytes > file, "open={open_publish}: {}", core.max_message_bytes);
    }
    // A client takes such a file too, but caps what any server may make it buffer at 1 GiB.
    assert_eq!(NetLimits::default().max_server_message, 1 << 30);
}

#[test]
fn the_batches_kept_for_reconnecting_apps_follow_max_log_mb() {
    let log = |cfg: Config| cfg.store().core.max_log_bytes;
    assert_eq!(log(Config::default()), 64 << 20, "a private server's log is bounded too");
    assert_eq!(log(Config { max_log_mb: 8, ..Config::default() }), 8 << 20);
    // Open to the public: also at most a quarter of a file (16 MiB unless set).
    assert_eq!(log(Config { open_publish: true, ..Config::default() }), 4 << 20);
    assert_eq!(log(Config { open_publish: true, max_log_mb: 1, ..Config::default() }), 1 << 20);
    for mb in [0, 4097] {
        assert!(Config { max_log_mb: mb, ..Config::default() }.check().is_err(), "max_log_mb = {mb}");
    }
}

#[test]
fn large_files_warn_the_operator() {
    let public_url = Some("wss://figcraft.example.com/".into());
    let warns = |open_publish, mb| {
        Config { open_publish, max_file_mb: Some(mb), public_url: public_url.clone(), ..Config::default() }.warnings().join(
            "
",
        )
    };
    assert!(!warns(true, 64).contains("max_file_mb"), "{}", warns(true, 64));
    assert!(warns(true, 65).contains("open_publish is on with max_file_mb = 65"), "{}", warns(true, 65));
    assert!(!warns(false, 1023).contains("max_file_mb"), "{}", warns(false, 1023));
    assert!(warns(false, 1024).contains("can't be opened"), "{}", warns(false, 1024));
}
