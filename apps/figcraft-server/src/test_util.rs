//! What the tests share: temporary data directories (removed when dropped), settings, and engine
//! sessions linked over loopback.

use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use figcraft_collab::message::{self, ClientMsg, ServerMsg};
use figcraft_collab_net::NetLink;
use figcraft_collab_net::guest::GuestConfig;
use figcraft_engine::Session;
use figcraft_engine::live::{Published, Status};
use serde_json::json;
use tungstenite::{Bytes, Message, WebSocket};

use crate::config::Config;

/// A directory of its own under the system's temporary directory, removed with everything in it
/// when dropped.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join("figcraft-server-tests").join(format!("{tag}-{}-{nanos}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Settings for a server on `dir`, listening on a free loopback port.
pub fn config(dir: &Path) -> Config {
    Config { listen: "127.0.0.1:0".into(), data: dir.to_path_buf(), ..Config::default() }
}

/// Joins quickly, and keeps trying long enough to outlast a server restart.
pub fn quick(mut c: GuestConfig) -> GuestConfig {
    c.backoff = Duration::from_millis(20);
    c.max_backoff = Duration::from_millis(200);
    c.attempts = 100;
    c.limits.hello_timeout = Duration::from_secs(5);
    c
}

/// An engine session with the network link, named `name`.
pub fn session(name: &str) -> Session {
    let mut s = Session::new();
    s.live.install(Box::new(NetLink::new(Arc::new(|| {})).with_guest_config(quick)));
    s.execute("pref.userName", &json!({"name": name})).unwrap();
    s
}

/// Pump every session (as their windows would each frame) until `f` holds.
pub fn pump(sessions: &mut [&mut Session], what: &str, f: impl Fn(&[&mut Session]) -> bool) {
    let start = Instant::now();
    loop {
        for s in sessions.iter_mut() {
            s.live_poll();
            s.live_sync();
        }
        if f(sessions) {
            return;
        }
        let states: Vec<String> = sessions.iter().map(|s| s.live_summary().to_string()).collect();
        assert!(start.elapsed() < Duration::from_secs(20), "timed out waiting for {what}: {states:#?}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The synced properties of two documents agree.
pub fn same(a: &figcraft_doc::Document, b: &figcraft_doc::Document) -> bool {
    figcraft_collab::diff(&figcraft_collab::Replica::new(a.clone()), b).is_ok_and(|d| d.changes.is_empty())
}

pub fn key(dir: &Path) -> String {
    std::fs::read_to_string(dir.join(crate::store::KEY)).unwrap().trim().to_string()
}

pub fn live(s: &Session) -> bool {
    s.live().status == Status::Live && s.live().syncing()
}

/// `a` publishes its open file to the server at `addr`; the links.
pub fn publish(a: &mut Session, dir: &Path, addr: SocketAddr) -> Published {
    let r = a.execute("collab.publish", &json!({"server": format!("ws://{addr}/?key={}", key(dir)), "name": "Team file"})).unwrap();
    assert_eq!(r["status"], "connecting");
    assert_eq!(r["readOnly"], true, "read-only while it is on its way");
    pump(&mut [a], "the publish", |s| live(s[0]) && s[0].live().published.is_some());
    a.live().published.clone().unwrap()
}

pub fn join(link: &str, name: &str) -> Session {
    let mut s = session(name);
    s.execute("collab.join", &json!({"link": link})).unwrap();
    s
}

/// A bare client: hello, then what it's told.
pub struct Raw(WebSocket<TcpStream>);

impl Raw {
    pub fn connect(addr: SocketAddr) -> Raw {
        let s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        Raw(tungstenite::client::client(format!("ws://{addr}/"), s).unwrap().0)
    }

    pub fn send(&mut self, m: &ClientMsg) {
        self.0.send(Message::Binary(Bytes::from(message::encode(m).unwrap()))).unwrap();
    }

    /// Write just the header of a masked binary frame declaring `len` payload bytes: the server
    /// refuses an oversized message from its header, before the client would push the payload
    /// into a socket the server has already closed (a broken pipe on Linux).
    pub fn send_header(&mut self, len: u64) {
        let mut h = vec![0x82u8, 0x80 | 127];
        h.extend_from_slice(&len.to_be_bytes());
        h.extend_from_slice(&[1, 2, 3, 4]);
        self.raw_write(&h);
    }

    /// Write raw bytes on the socket (a hand-made, partial frame).
    pub fn raw_write(&mut self, bytes: &[u8]) {
        use std::io::Write as _;
        self.0.get_mut().write_all(bytes).unwrap();
    }

    /// The server closed the connection within `limit` (anything it sends meanwhile is skipped).
    pub fn closed_within(&mut self, limit: Duration) -> bool {
        use std::io::Read as _;
        let s = self.0.get_mut();
        let end = Instant::now() + limit;
        let mut buf = [0u8; 4096];
        let _ = s.set_read_timeout(Some(Duration::from_millis(200)));
        while Instant::now() < end {
            match s.read(&mut buf) {
                Ok(0) => return true,
                Ok(_) => {}
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(_) => return true,
            }
        }
        false
    }

    /// Send without waiting to be read (`Err` once the server has hung up).
    pub fn try_send(&mut self, m: &ClientMsg) -> tungstenite::Result<()> {
        self.0.send(Message::Binary(Bytes::from(message::encode(m).unwrap())))
    }

    pub fn next(&mut self) -> ServerMsg {
        loop {
            if let Message::Binary(b) = self.0.read().unwrap() {
                return message::decode(&b, usize::MAX).unwrap();
            }
        }
    }
}

/// A one-page file with one rectangle, named `name`, as a snapshot to publish.
pub fn snapshot(name: &str) -> Vec<figcraft_collab::Change> {
    let mut d = figcraft_doc::Document::new(name);
    let page = d.first_page().unwrap();
    d.create(page, figcraft_doc::NodeKind::Rectangle, "Card", 0.0, 0.0, 100.0, 60.0).unwrap();
    d.repair();
    figcraft_collab::Replica::new(d).snapshot().unwrap()
}

/// An address from text.
pub fn ip(s: &str) -> std::net::IpAddr {
    s.parse().unwrap()
}
