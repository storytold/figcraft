//! Font downloads over HTTPS against a local server with a test certificate: a 200 is the file, a
//! 404 is none, other answers, cut-off and oversized bodies are errors, and only `https://`
//! addresses are asked.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

use super::*;

/// A server on `localhost` that answers one request with `answer` (written as is) and keeps the
/// request it read.
fn serve(answer: Vec<u8>) -> (String, Arc<Mutex<String>>) {
    let cert = CertificateDer::from(&include_bytes!("testdata/localhost.der")[..]).into_owned();
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(include_bytes!("testdata/localhost-key.der").to_vec()));
    let cfg = ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .unwrap();
    let listener = TcpListener::bind("localhost:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(String::new()));
    let keep = seen.clone();
    std::thread::spawn(move || {
        let Ok((s, _)) = listener.accept() else { return };
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut tls = StreamOwned::new(ServerConnection::new(Arc::new(cfg)).unwrap(), s);
        let mut req = Vec::new();
        let mut buf = [0u8; 1024];
        while !req.ends_with(b"\r\n\r\n") {
            match tls.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => req.extend_from_slice(&buf[..n]),
            }
        }
        *keep.lock().unwrap() = String::from_utf8_lossy(&req).into_owned();
        let _ = tls.write_all(&answer);
        tls.conn.send_close_notify();
        let _ = tls.flush();
    });
    (format!("https://localhost:{port}"), seen)
}

fn fetch(url: &str, max: usize) -> Result<Option<Vec<u8>>, String> {
    get(url, max, &NetLimits::default(), Duration::from_secs(5))
}

#[test]
fn a_200_is_the_file_and_the_request_is_a_plain_get() {
    let (base, seen) = serve(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nfonts".to_vec());
    assert_eq!(fetch(&format!("{base}/ofl/a%5Bwght%5D.ttf"), 1024), Ok(Some(b"fonts".to_vec())));
    let req = seen.lock().unwrap().clone();
    assert!(req.starts_with("GET /ofl/a%5Bwght%5D.ttf HTTP/1.0\r\n"), "{req}");
    assert!(req.contains("\r\nHost: localhost\r\n"), "{req}");
}

#[test]
fn a_404_is_no_file_and_other_answers_are_errors() {
    let (base, _) = serve(b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nnot found".to_vec());
    assert_eq!(fetch(&format!("{base}/x"), 1024), Ok(None));
    let (base, _) = serve(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n".to_vec());
    assert_eq!(fetch(&format!("{base}/x"), 1024), Err("the server answered 500".into()));
    let (base, _) = serve(b"HTTP/1.1 301 Moved\r\nLocation: https://elsewhere/\r\nContent-Length: 0\r\n\r\n".to_vec());
    assert_eq!(fetch(&format!("{base}/x"), 1024), Err("the server answered 301".into()), "redirects aren't followed");
}

#[test]
fn cut_off_and_oversized_bodies_are_errors() {
    let (base, _) = serve(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nshort".to_vec());
    assert_eq!(fetch(&format!("{base}/x"), 1024), Err("the download ended early".into()));
    let (base, _) = serve(b"HTTP/1.1 200 OK\r\nContent-Length: 2000\r\n\r\n".to_vec());
    assert!(fetch(&format!("{base}/x"), 1024).unwrap_err().contains("larger than"));
    let mut big = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
    big.extend(std::iter::repeat_n(b'x', 4096));
    let (base, _) = serve(big);
    assert!(fetch(&format!("{base}/x"), 1024).unwrap_err().contains("larger than"), "no length given");
    let (base, _) = serve(b"garbage".to_vec());
    assert!(fetch(&format!("{base}/x"), 1024).is_err());
}

#[test]
fn only_https_addresses_are_asked() {
    for url in ["http://localhost/x", "ftp://localhost/x", "https://", "https://user@host/x", "https://host:99999/x", "https://host/a b"] {
        assert!(parse(url).is_err(), "{url}");
    }
    assert_eq!(
        parse("https://raw.githubusercontent.com/google/fonts/main/ofl/x/METADATA.pb").unwrap(),
        ("raw.githubusercontent.com:443".into(), "raw.githubusercontent.com".into(), "/google/fonts/main/ofl/x/METADATA.pb".into())
    );
    assert_eq!(parse("https://[::1]:8443").unwrap(), ("[::1]:8443".into(), "::1".into(), "/".into()));
    assert_eq!(parse("https://[::1]/f").unwrap(), ("[::1]:443".into(), "::1".into(), "/f".into()));
}
