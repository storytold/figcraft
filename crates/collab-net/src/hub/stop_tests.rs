//! Stopping a host never waits forever on its accept loop, even when the connection meant to wake
//! the loop is taken by another listener on the same port.

use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::sync::mpsc::channel;
use std::time::Duration;

use figcraft_collab::Server;

use super::{Admission, Backend, Closing, Hosting};
use crate::net::NetLimits;

/// A backend that admits no one.
struct Nobody;

impl Backend for Nobody {
    fn admit(&mut self, _file: &str, _token: &str) -> Result<Admission, String> {
        Err("no".into())
    }

    fn open(&mut self, _file: &str) -> Result<Server, String> {
        Err("no".into())
    }

    fn closing(&self) -> Closing {
        Closing::Ended
    }
}

/// A host on every interface is woken at the loopback address on its port. On macOS and the BSDs
/// another socket may bind exactly that address and port while the host listens (another test, or
/// another app), and then it takes the wake-up connection: the host's accept loop stays asleep.
/// `stop` used to join that loop and hung for good (the full test run did, in
/// `code_tests::a_share_on_every_interface_names_an_address_others_reach`).
#[test]
fn stop_returns_when_another_listener_takes_the_wake_up() {
    let host = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
    let port = host.local_addr().unwrap().port();
    // Linux refuses this bind while the host listens on every interface: nothing to test there.
    let Ok(_decoy) = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))) else { return };
    let mut hosting = Hosting::serve(host, Nobody, NetLimits::default()).unwrap();
    let (done, stopped) = channel();
    std::thread::spawn(move || {
        hosting.stop();
        let _ = done.send(());
    });
    assert!(stopped.recv_timeout(Duration::from_secs(10)).is_ok(), "stop hung waiting for the accept loop");
}
