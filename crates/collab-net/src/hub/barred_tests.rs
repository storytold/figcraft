//! Addresses removed from a file are kept only while its session lasts, so a long-running server
//! doesn't collect them for every file it ever had open.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};

use figcraft_collab::{Limits, Server};
use figcraft_doc::Document;

use super::{Admission, Backend, Closing, Hub, InFlight, Room};

/// A backend that closes a file as soon as everyone left it.
struct Closes;

impl Backend for Closes {
    fn admit(&mut self, _file: &str, _token: &str) -> Result<Admission, String> {
        Err("no".into())
    }

    fn open(&mut self, _file: &str) -> Result<Server, String> {
        Err("no".into())
    }

    fn idle(&mut self, _file: &str, _server: &mut Server) -> bool {
        true
    }

    fn closing(&self) -> Closing {
        Closing::Restarting
    }
}

#[test]
fn a_closed_file_forgets_whom_it_barred() {
    let mut doc = Document::new("File");
    doc.repair();
    let mut hub = Hub {
        in_flight: InFlight(Default::default()),
        backend: Closes,
        rooms: HashMap::from([("f".to_string(), Room::new(Server::new(doc, "", Limits::default())))]),
        conns: HashMap::new(),
        local: None,
        outbox: Vec::new(),
        barred: HashMap::new(),
    };
    let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7));
    hub.barred.entry("f".into()).or_default().insert(ip);
    hub.barred.entry("other".into()).or_default().insert(ip);
    hub.close_if_empty("f");
    assert!(!hub.rooms.contains_key("f"), "everyone left: the file closed");
    assert!(!hub.barred.contains_key("f"), "and its barred addresses went with it");
    assert!(hub.barred.contains_key("other"), "another file's stay");
}

#[test]
fn a_kicked_connection_with_a_full_queue_is_shut() {
    use std::io::Read;
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;

    use super::{Conn, Kind, Line, Out};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (server_side, _) = listener.accept().unwrap();
    client.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    // Stands in for the writer thread's handle: it keeps the socket open after the Conn is gone.
    let _writer = server_side.try_clone().unwrap();
    let (tx, _rx) = sync_channel(2);
    while tx.try_send(Out::Bytes(Default::default(), None)).is_ok() {}
    let out = Line {
        assets: false,
        previews: false,
        tx,
        queued: Arc::default(),
        block: Arc::new(AtomicUsize::new(0)),
        all: Arc::new(AtomicUsize::new(0)),
        max: 1 << 20,
        max_block: usize::MAX,
        max_all: usize::MAX,
    };
    let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let mut hub = Hub {
        in_flight: InFlight(Default::default()),
        backend: Closes,
        rooms: HashMap::new(),
        conns: HashMap::from([(1, Conn { out, sock: server_side, kind: Kind::Managing { comments: true }, ip })]),
        local: None,
        outbox: Vec::new(),
        barred: HashMap::new(),
    };
    hub.kick(&[1], "kicked");
    assert!(hub.conns.is_empty());
    let mut buf = [0u8; 1];
    assert_eq!(client.read(&mut buf).ok(), Some(0), "the kicked socket must be shut (client sees EOF)");
}

#[test]
fn review_followup_catalog_caps_current_new_and_historical_references() {
    use figcraft_assets::AssetRef;
    use figcraft_doc::ImageData;
    use std::sync::Arc;
    let reference = |n: u64| AssetRef { scope: "b".repeat(64), sha256: format!("{n:064x}"), len: 1 };
    let mut doc = Document::new("Synthetic catalog budget");
    for n in 0..20_001u64 {
        doc.images.insert(format!("{n:08}"), ImageData { asset: Some(reference(n)), bytes: Arc::new(Vec::new()), width: 1, height: 1 });
    }
    assert_eq!(super::image_catalog(&doc, [], []).len(), 20_000);
    let catalog = super::image_catalog(&doc, (30_000..70_001).map(reference), [reference(90_000)]);
    assert_eq!(catalog.len(), 40_000);
    assert!(catalog.iter().all(|asset| asset.scope.is_empty()));
    let mut current = reference(0);
    current.scope.clear();
    let mut recent = reference(30_000);
    recent.scope.clear();
    let mut old = reference(90_000);
    old.scope.clear();
    assert!(catalog.contains(&current));
    assert!(catalog.contains(&recent));
    assert!(!catalog.contains(&old));
}
