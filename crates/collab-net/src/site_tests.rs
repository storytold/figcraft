//! The page a session host serves to browsers ([`crate::site`]): which requests it answers, what
//! it sends, and, end to end over loopback, that the page and the WebSocket share the port and the
//! view invite in the viewer's address only lets people view.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use figcraft_collab::Limits;
use figcraft_collab::message::Access;
use figcraft_doc::Document;
use figcraft_engine::live::{Event, Status};

use crate::guest::{Guest, GuestConfig};
use crate::host::{Host, HostConfig};
use crate::invite::Invite;
use crate::loopback_tests::{Probe, fast_limits, quick};
use crate::net::NetLimits;
use crate::site::{MAX_PAGES_PER_ADDRESS, PageRequest, Site, SiteFile, content_type, parse_head, respond, viewer_url};

const TOKEN: &str = "AbCdEfGhIjKlMnOpQrStUv";
const VIEW: &str = "ViewViewViewViewView00";
const FILE: &str = "0123456789abcdef";

static PAGE: &[u8] = b"\x1f\x8bpage";
static WASM: &[u8] = b"\x1f\x8bwasm";

fn site() -> Site {
    Site::new(vec![
        SiteFile { path: "index.html", content_type: content_type("index.html"), gzip: PAGE },
        SiteFile { path: "figcraft-web-0f1e_bg.wasm", content_type: content_type("x.wasm"), gzip: WASM },
    ])
}

fn request(path: &str, gzip: bool) -> PageRequest {
    PageRequest { path: path.into(), upgrade: false, gzip }
}

#[test]
fn reads_what_a_page_request_asks() {
    let head = b"GET /?webgl HTTP/1.1\r\nHost: x\r\nAccept-Encoding: gzip, deflate";
    assert_eq!(parse_head(head), Some(request("/", true)));
    let ws = b"GET / HTTP/1.1\r\nHost: x\r\nConnection: Upgrade\r\nUpgrade: WebSocket\r\nSec-WebSocket-Key: k";
    assert!(parse_head(ws).is_some_and(|r| r.upgrade), "a WebSocket handshake is left to the session");
    let q = b"GET /a.js HTTP/1.1\r\naccept-encoding: br;q=1.0, GZIP;q=0.5";
    assert_eq!(parse_head(q), Some(request("/a.js", true)), "header names and values are read without regard to case");
    assert_eq!(parse_head(b"GET /a.js HTTP/1.1\r\nAccept-Encoding: br"), Some(request("/a.js", false)));
    assert_eq!(parse_head(b"POST / HTTP/1.1\r\n"), None);
    assert_eq!(parse_head(b"GET"), None);
    assert_eq!(parse_head(&[0xff, 0xfe, b' ']), None);
}

#[test]
fn serves_only_its_own_files_gzipped() {
    let site = site();
    let (head, body) = respond(&site, &request("/", true));
    let head = String::from_utf8(head).unwrap();
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
    assert!(head.contains("Content-Type: text/html; charset=utf-8\r\n"));
    assert!(head.contains("Content-Encoding: gzip\r\n"));
    assert!(head.contains(&format!("Content-Length: {}\r\n", PAGE.len())));
    assert!(head.contains("Cache-Control: no-cache\r\n"), "the page itself is checked again each time");
    assert!(head.contains("Referrer-Policy: no-referrer\r\n"), "the page tells nobody where it came from");
    assert_eq!(body, PAGE);
    let (head, body) = respond(&site, &request("/figcraft-web-0f1e_bg.wasm", true));
    let head = String::from_utf8(head).unwrap();
    assert!(head.contains("Content-Type: application/wasm\r\n"), "browsers compile wasm only with its own type");
    assert!(head.contains("immutable"), "hashed files are cached");
    assert_eq!(body, WASM);
    for missing in ["/nope.js", "/../Cargo.toml", "/index.html/..", "//etc/passwd"] {
        let (head, body) = respond(&site, &request(missing, true));
        assert!(String::from_utf8(head).unwrap().starts_with("HTTP/1.1 404"), "{missing}");
        assert!(body.is_empty());
    }
    let (head, _) = respond(&site, &request("/", false));
    assert!(String::from_utf8(head).unwrap().starts_with("HTTP/1.1 406"));
}

#[test]
fn content_types_follow_the_extension() {
    assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
    assert_eq!(content_type("a/b.JS"), "text/javascript; charset=utf-8");
    assert_eq!(content_type("x_bg.wasm"), "application/wasm");
    assert_eq!(content_type("README"), "application/octet-stream");
}

#[test]
fn a_site_without_a_page_has_nothing_to_open() {
    assert!(!Site::default().has_page());
    assert!(site().has_page());
    assert!(site().file("/").is_some_and(|f| f.path == "index.html"));
    assert!(site().file("/index.html").is_some());
}

#[test]
fn the_viewer_address_opens_the_page_with_the_link_last() {
    let addr: SocketAddr = ([192, 168, 1, 20], 47900).into();
    let link = Invite::new(addr.to_string(), FILE, VIEW).link();
    assert_eq!(
        viewer_url(addr, &link),
        format!("http://192.168.1.20:47900/#view&join=figcraft://join?host=192.168.1.20:47900&file={FILE}&token={VIEW}")
    );
    let v6: SocketAddr = "[fe80::1]:47900".parse().unwrap();
    assert!(viewer_url(v6, "l").starts_with("http://[fe80::1]:47900/#"));
}

fn host(site: Option<Site>, view_token: Option<&str>) -> (Host, Probe) {
    host_with(site, view_token, fast_limits())
}

fn host_with(site: Option<Site>, view_token: Option<&str>, limits: NetLimits) -> (Host, Probe) {
    let probe = Probe::new();
    let mut doc = Document::new("Shared file");
    doc.repair();
    let cfg = HostConfig {
        addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        any_port_if_taken: false,
        file: FILE.into(),
        token: TOKEN.into(),
        view_token: view_token.map(str::to_string),
        site: site.map(Arc::new),
        doc,
        name: "Hana".into(),
        limits,
        core: Limits::default(),
    };
    let host = Host::start(cfg, probe.inbox.clone()).unwrap();
    (host, probe)
}

/// Send a raw HTTP request to `addr`; the whole response.
fn get(addr: SocketAddr, req: &str) -> Vec<u8> {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s.write_all(req.as_bytes()).unwrap();
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    out
}

fn join(link: &str) -> (Guest, Probe) {
    let probe = Probe::new();
    let invite = Invite::parse(link).unwrap();
    let g = Guest::start(quick(GuestConfig::new(invite, "Phone".into())), probe.inbox.clone()).unwrap();
    (g, probe)
}

fn access(p: &mut Probe) -> Access {
    p.wait("the welcome", |e| e.iter().any(|e| matches!(e, Event::Welcome { .. })));
    p.events.iter().find_map(|e| if let Event::Welcome { info, .. } = e { Some(info.access) } else { None }).unwrap()
}

#[test]
fn the_page_and_the_session_share_the_port() {
    let (mut host, _hp) = host(Some(site()), Some(VIEW));
    let addr = host.addr();
    let viewer = host.viewer().unwrap().to_string();
    assert!(viewer.starts_with(&format!("http://{addr}/#view&join=figcraft://join?")), "{viewer}");

    // (The browser leaves the fragment out: it asks for `/`.)
    let page = get(addr, "GET / HTTP/1.1\r\nHost: x\r\nAccept-Encoding: gzip\r\n\r\n");
    assert!(page.starts_with(b"HTTP/1.1 200 OK\r\n"), "{}", String::from_utf8_lossy(&page));
    assert!(page.ends_with(PAGE));
    let missing = get(addr, "GET /secret.txt HTTP/1.1\r\nHost: x\r\nAccept-Encoding: gzip\r\n\r\n");
    assert!(missing.starts_with(b"HTTP/1.1 404"));
    let health = get(addr, "GET /healthz HTTP/1.1\r\nHost: x\r\n\r\n");
    assert!(health.ends_with(b"ok\n"), "uptime checks are answered as before");

    // The address's link only lets people view; the session's own link still edits.
    let (_phone, mut pp) = join(viewer.split_once("join=").unwrap().1);
    assert_eq!(access(&mut pp), Access::View);
    let (_guest, mut gp) = join(&Invite::new(addr.to_string(), FILE, TOKEN).link());
    assert_eq!(access(&mut gp), Access::Edit);
    host.stop();
}

#[test]
fn without_the_page_or_a_view_token_there_is_no_viewer() {
    let (mut plain, _) = host(None, None);
    assert_eq!(plain.viewer(), None);
    let page = get(plain.addr(), "GET / HTTP/1.1\r\nHost: x\r\nAccept-Encoding: gzip\r\n\r\n");
    assert!(!page.starts_with(b"HTTP/1.1 200"), "a host without the page serves none");
    plain.stop();

    let (mut no_page, _) = host(Some(Site::default()), Some(VIEW));
    assert_eq!(no_page.viewer(), None, "a site without index.html has nothing to open");
    no_page.stop();

    // Without a view token the view secret is refused like any wrong one.
    let (mut edit_only, _) = host(Some(site()), None);
    assert_eq!(edit_only.viewer(), None);
    let (_g, mut gp) = join(&Invite::new(edit_only.addr().to_string(), FILE, VIEW).link());
    gp.wait("a refusal", |e| e.iter().any(|e| matches!(e, Event::Status(Status::Failed { reason }) if reason.contains("token"))));
    edit_only.stop();
}

#[test]
fn slow_downloads_from_one_address_take_only_a_few_connections() {
    // A file big enough that a reader who doesn't read keeps its download open.
    let big: &'static [u8] = Box::leak(vec![0u8; 32 << 20].into_boxed_slice());
    let site = Site::new(vec![
        SiteFile { path: "index.html", content_type: content_type("index.html"), gzip: PAGE },
        SiteFile { path: "big.wasm", content_type: content_type("big.wasm"), gzip: big },
    ]);
    let (mut host, _) = host(Some(site), Some(VIEW));
    let addr = host.addr();
    let req = b"GET /big.wasm HTTP/1.1\r\nHost: x\r\nAccept-Encoding: gzip\r\n\r\n";
    let mut slow = Vec::new();
    for _ in 0..MAX_PAGES_PER_ADDRESS {
        let mut s = TcpStream::connect(addr).unwrap();
        s.write_all(req).unwrap();
        slow.push(s);
    }
    // Wait until they all are being answered (their responses have started).
    for s in &mut slow {
        let mut first = [0u8; 12];
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        s.read_exact(&mut first).unwrap();
        assert_eq!(&first, b"HTTP/1.1 200");
    }
    let refused = get(addr, "GET /big.wasm HTTP/1.1\r\nHost: x\r\nAccept-Encoding: gzip\r\n\r\n");
    assert!(refused.starts_with(b"HTTP/1.1 503"), "{}", String::from_utf8_lossy(&refused[..refused.len().min(80)]));
    // The session itself still lets people in.
    let (_phone, mut pp) = join(&Invite::new(addr.to_string(), FILE, VIEW).link());
    assert_eq!(access(&mut pp), Access::View);
    // Once they go, downloads are taken again.
    drop(slow);
    let start = std::time::Instant::now();
    loop {
        let page = get(addr, "GET / HTTP/1.1\r\nHost: x\r\nAccept-Encoding: gzip\r\n\r\n");
        if page.starts_with(b"HTTP/1.1 200") {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "downloads are taken again");
        std::thread::sleep(Duration::from_millis(20));
    }
    host.stop();
}

/// Open a connection to `addr` that starts a page request and never finishes its head.
fn unfinished(addr: SocketAddr) -> TcpStream {
    let mut s = TcpStream::connect(addr).unwrap();
    s.write_all(b"GET /index.html HTTP/1.1\r\nHost: x\r\nAccept-Encoding: gzip\r\n").unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s
}

/// Whether the host closed `s` without answering it.
fn closed_unanswered(s: &mut TcpStream) -> bool {
    let mut buf = [0u8; 64];
    matches!(s.read(&mut buf), Ok(0)) || matches!(s.read(&mut buf), Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset)
}

#[test]
fn unfinished_page_requests_wait_only_until_the_hello_deadline() {
    // Requests that never finish their head (a slow or hostile client) each hold a connection
    // thread, as a WebSocket handshake that never finishes does: until the hello deadline, within
    // the connections the session takes in all.
    let deadline = Duration::from_millis(800);
    let limits = NetLimits { hello_timeout: deadline, max_connections: 12, ..NetLimits::default() };
    let (mut host, _) = host_with(Some(site()), Some(VIEW), limits);
    let addr = host.addr();
    let link = Invite::new(addr.to_string(), FILE, VIEW).link();

    // Below the cap the session still lets people in while they wait.
    let start = std::time::Instant::now();
    let mut slow: Vec<TcpStream> = (0..8).map(|_| unfinished(addr)).collect();
    let (_phone, mut pp) = join(&link);
    assert_eq!(access(&mut pp), Access::View);
    // Each is closed unanswered at the deadline, not before and not much later.
    for s in &mut slow {
        assert!(closed_unanswered(s), "an unfinished request gets no page");
    }
    let took = start.elapsed();
    assert!(took >= deadline.mul_f32(0.9) && took < deadline * 4, "closed at the hello deadline ({took:?})");

    // At the cap (these and the phone) new connections are turned away (sessions too, as with
    // unfinished handshakes), and taken again once the deadline frees them.
    let mut full: Vec<TcpStream> = (0..limits.max_connections - 1).map(|_| unfinished(addr)).collect();
    std::thread::sleep(Duration::from_millis(100));
    let refused = get(addr, "GET / HTTP/1.1\r\nHost: x\r\nAccept-Encoding: gzip\r\n\r\n");
    assert!(refused.is_empty(), "over the cap: closed at once ({} bytes)", refused.len());
    for s in &mut full {
        assert!(closed_unanswered(s));
    }
    let page = get(addr, "GET / HTTP/1.1\r\nHost: x\r\nAccept-Encoding: gzip\r\n\r\n");
    assert!(page.starts_with(b"HTTP/1.1 200 OK\r\n"), "the page again once they are gone");
    let (_phone, mut pp) = join(&link);
    assert_eq!(access(&mut pp), Access::View, "and the session");
    host.stop();
}
