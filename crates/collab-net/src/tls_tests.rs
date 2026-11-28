//! Joining over `wss://`: a guest and a managing connection reach a host through a TLS terminator
//! (like the reverse proxy in front of a server) whose certificate comes from a test authority
//! only test builds trust; a certificate for another name is refused with a clear reason. An app
//! hosting with its own certificate is joined through the pin in its invite, and only through it:
//! a plain client, or a host presenting another certificate (someone in between), gets nothing.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use figcraft_collab::Limits;
use figcraft_engine::live::{Event, Status};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

use crate::guest::{Guest, GuestConfig};
use crate::invite::{Endpoint, Invite};
use crate::loopback_tests::{Probe, fast_limits, host_on_loopback, quick, start};
use crate::manage;

/// Serve `localhost`'s test certificate on a port of `localhost` and relay each connection to
/// `upstream`.
fn terminator(upstream: SocketAddr) -> SocketAddr {
    let cert = CertificateDer::from(&include_bytes!("testdata/localhost.der")[..]).into_owned();
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(include_bytes!("testdata/localhost-key.der").to_vec()));
    let cfg = ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .unwrap();
    let cfg = Arc::new(cfg);
    // The name's first address, which a client tries first (Windows takes seconds to give up on
    // a refused one).
    let listener = TcpListener::bind("localhost:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            let cfg = cfg.clone();
            std::thread::spawn(move || relay(s, upstream, cfg));
        }
    });
    addr
}

/// Copy bytes both ways until either side ends (polling with short timeouts: test code).
fn relay(s: TcpStream, upstream: SocketAddr, cfg: Arc<ServerConfig>) {
    let tick = Some(Duration::from_millis(2));
    let Ok(mut up) = TcpStream::connect(upstream) else { return };
    up.set_read_timeout(tick).unwrap();
    s.set_read_timeout(tick).unwrap();
    let mut tls = StreamOwned::new(ServerConnection::new(cfg).unwrap(), s);
    let mut buf = [0u8; 16 * 1024];
    let pass = |r: io::Result<usize>, to: &mut dyn Write, buf: &[u8]| match r {
        Ok(0) => false,
        Ok(n) => to.write_all(&buf[..n]).and_then(|()| to.flush()).is_ok(),
        Err(e) => matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut),
    };
    loop {
        let r = tls.read(&mut buf);
        if !pass(r, &mut up, &buf) {
            return;
        }
        let r = up.read(&mut buf);
        if !pass(r, &mut tls, &buf) {
            return;
        }
    }
}

fn secure(host: String, invite: &Invite) -> Invite {
    Invite { server: Endpoint { host, path: "/".into(), secure: true, cert: None }, ..invite.clone() }
}

#[test]
fn guests_and_managing_connections_join_over_wss() {
    let (mut host, _hp, invite) = start();
    let front = terminator(host.addr());
    let wss = secure(format!("localhost:{}", front.port()), &invite);
    assert!(wss.url().starts_with("wss://localhost:"));
    let mut probe = Probe::new();
    let mut g = Guest::start(quick(GuestConfig::new(wss.clone(), "Ada".into())), probe.inbox.clone()).unwrap();
    probe.wait("the welcome over TLS", |e| e.iter().any(|e| matches!(e, Event::Welcome { .. })));
    // The managing path speaks TLS too: an app's host answers, and refuses to manage files.
    let err = manage::request(&wss.server, "key", Vec::new(), &fast_limits(), Duration::from_secs(5)).unwrap_err();
    assert!(err.contains("hosted by an app"), "{err}");
    g.leave();
    host.stop();
}

#[test]
fn a_certificate_for_another_name_is_refused() {
    let (mut host, _hp, invite) = start();
    let front = terminator(host.addr());
    let mut probe = Probe::new();
    let _g = Guest::start(quick(GuestConfig::new(secure(front.to_string(), &invite), "Ada".into())), probe.inbox.clone()).unwrap();
    probe.wait("the refusal", |e| {
        e.iter()
            .any(|e| matches!(e, Event::Status(Status::Failed { reason }) if reason.contains(&format!("certificate of {front} is for another name"))))
    });
    host.stop();
}

#[test]
fn reader_and_writer_share_one_session_without_blocking_each_other() {
    // An echo server behind the terminator: more than any socket buffers hold, written in one go,
    // comes back while it is still being sent (were the reader to wait for the writer, both would
    // stall once the buffers fill).
    let echo = TcpListener::bind("127.0.0.1:0").unwrap();
    let upstream = echo.local_addr().unwrap();
    std::thread::spawn(move || {
        let (mut s, _) = echo.accept().unwrap();
        let _ = io::copy(&mut s.try_clone().unwrap(), &mut s);
    });
    let front = terminator(upstream);
    let server = Endpoint { host: format!("localhost:{}", front.port()), path: "/".into(), secure: true, cert: None };
    let (mut rd, mut wr, _sock) = crate::net::open(&server, Duration::from_secs(5), &fast_limits()).unwrap();
    rd.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let sent: Vec<u8> = (0..8 << 20).map(|i: u32| (i % 251) as u8).collect();
    let expect = sent.clone();
    let writer = std::thread::spawn(move || wr.write_all(&sent).unwrap());
    let mut got = vec![0u8; expect.len()];
    rd.read_exact(&mut got).unwrap();
    writer.join().unwrap();
    assert!(got == expect);
}

/// The loopback hosts' token.
const TOKEN_OF_TESTS: &str = "AbCdEfGhIjKlMnOpQrStUv";

fn tls_host() -> (crate::host::Host, Probe, Invite) {
    host_on_loopback(fast_limits(), Limits::default(), true)
}

fn failed(e: &[Event]) -> Option<&str> {
    e.iter().find_map(|e| match e {
        Event::Status(Status::Failed { reason }) => Some(reason.as_str()),
        _ => None,
    })
}

#[test]
fn a_host_with_its_own_certificate_is_joined_through_the_pin() {
    let (mut host, mut hp, invite) = tls_host();
    let pin = host.cert_sha256().unwrap();
    assert_eq!((invite.server.secure, invite.server.cert), (true, Some(pin)));
    assert!(invite.link().contains("cert=") && invite.url().starts_with("wss://"), "{}", invite.link());
    // Joined from the link, as someone pasting it would; the document comes over TLS.
    let mut probe = Probe::new();
    let mut g = Guest::start(quick(GuestConfig::new(Invite::parse(&invite.link()).unwrap(), "Ada".into())), probe.inbox.clone()).unwrap();
    probe.wait("the welcome over pinned TLS", |e| e.iter().any(|e| matches!(e, Event::Welcome { .. })));
    hp.wait("the guest in the host's session", |e| e.iter().any(|e| matches!(e, Event::PeerJoined(_))));
    g.leave();
    host.stop();
}

/// Someone in between (here: another host with the same file and token, and a certificate of its
/// own) is refused before the guest says anything: the token never reaches it.
#[test]
fn a_host_presenting_another_certificate_is_refused_before_the_hello() {
    let (mut real, _rp, invite) = tls_host();
    let (mut mitm, mut mp, posing) = tls_host();
    assert_ne!(real.cert_sha256(), mitm.cert_sha256());
    let pinned_to_real = Invite { server: Endpoint { host: posing.server.host.clone(), ..invite.server.clone() }, ..invite };
    let mut probe = Probe::new();
    let _g = Guest::start(quick(GuestConfig::new(pinned_to_real, "Ada".into())), probe.inbox.clone()).unwrap();
    probe.wait("the refusal", |e| failed(e).is_some());
    let reason = failed(&probe.events).unwrap();
    assert!(reason.contains("isn't the one the invite names"), "{reason}");
    assert!(!reason.contains(TOKEN_OF_TESTS) && !reason.contains("cert="), "{reason}");
    std::thread::sleep(Duration::from_millis(200));
    mp.events.extend(mp.inbox.drain());
    assert!(mp.joined().is_empty(), "the impostor admitted the guest: {:#?}", mp.events);
    assert!(!probe.events.iter().any(|e| matches!(e, Event::Welcome { .. })));
    mitm.stop();
    real.stop();
}

/// A plain `ws://` client gets no session from a TLS host, and soon knows it.
#[test]
fn a_plain_client_cant_join_a_tls_host() {
    let (mut host, mut hp, invite) = tls_host();
    let plain = Invite::new(invite.server.host.clone(), invite.file.clone(), invite.token.clone());
    let mut probe = Probe::new();
    let started = Instant::now();
    let _g = Guest::start(quick(GuestConfig::new(plain, "Ada".into())), probe.inbox.clone()).unwrap();
    probe.wait("the failure", |e| failed(e).is_some());
    assert!(started.elapsed() < fast_limits().hello_timeout, "{:?}", started.elapsed());
    hp.events.extend(hp.inbox.drain());
    assert!(hp.joined().is_empty());
    host.stop();
}

/// The WebSocket request to a pinned host names neither the token nor the pin.
#[test]
fn the_request_to_a_pinned_host_carries_no_secret() {
    let cert = crate::cert::generate().unwrap();
    let cfg = crate::cert::server_config(&cert).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (s, _) = listener.accept().unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut tls = StreamOwned::new(ServerConnection::new(cfg).unwrap(), s);
        let mut req = Vec::new();
        let mut b = [0u8; 1];
        while !req.ends_with(b"\r\n\r\n") && tls.read_exact(&mut b).is_ok() {
            req.push(b[0]);
        }
        let _ = tx.send(String::from_utf8_lossy(&req).into_owned());
    });
    let invite = Invite { server: Endpoint::pinned(addr.to_string(), cert.sha256), file: "0123456789abcdef".into(), token: TOKEN_OF_TESTS.into() };
    let _g = Guest::start(quick(GuestConfig::new(invite, "Ada".into())), Probe::new().inbox.clone()).unwrap();
    let req = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let line = req.lines().next().unwrap_or_default();
    assert!(line.starts_with("GET / "), "{req}");
    assert!(!req.contains(TOKEN_OF_TESTS) && !req.contains("cert"), "{req}");
}
