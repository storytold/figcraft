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
