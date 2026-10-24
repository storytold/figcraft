//! A connection whose hello the backend hasn't admitted yet is held to the hello's size limit: it
//! can't have a message of the full size read while its token is unchecked.

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use figcraft_collab::message::{self, ClientMsg, ServerMsg, Signal};
use figcraft_collab::{Client, Server};
use tungstenite::{Bytes, Message};

use super::{Admission, Backend, Closing, Hosting};
use crate::net::NetLimits;

/// A backend that refuses every hello, once it is told to.
struct Slow(Receiver<()>);

impl Backend for Slow {
    fn admit(&mut self, _file: &str, _token: &str) -> Result<Admission, String> {
        let _ = self.0.recv();
        Err("wrong token".into())
    }

    fn open(&mut self, _file: &str) -> Result<Server, String> {
        Err("no".into())
    }

    fn closing(&self) -> Closing {
        Closing::Ended
    }
}

fn connect(addr: SocketAddr) -> tungstenite::WebSocket<TcpStream> {
    let s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    tungstenite::client::client(format!("ws://{addr}/"), s).unwrap().0
}

#[test]
fn an_unadmitted_connection_cant_send_a_large_message() {
    let limits = NetLimits { hello_timeout: Duration::from_secs(5), ..NetLimits::default() };
    let (release, wait) = channel();
    let mut hosting = Hosting::serve(TcpListener::bind("127.0.0.1:0").unwrap(), Slow(wait), limits).unwrap();
    let addr = hosting.addr();
    let sender = std::thread::spawn(move || {
        let mut ws = connect(addr);
        let hello = message::encode(&Client::hello("0123456789abcdef", "bad", "x", "")).unwrap();
        ws.send(Message::Binary(Bytes::from(hello))).unwrap();
        let big = ClientMsg::Signal(Signal { kind: Signal::CHAT.into(), text: "x".repeat(1 << 20) });
        let _ = ws.send(Message::Binary(Bytes::from(message::encode(&big).unwrap())));
        let mut refused = None;
        while let Ok(m) = ws.read() {
            if let Message::Binary(b) = m
                && let Ok(ServerMsg::Refused { reason }) = message::decode(&b, 64 << 20)
            {
                refused = Some(reason);
            }
        }
        refused
    });
    let mut peak = 0;
    let until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < until {
        peak = peak.max(hosting.in_flight_bytes());
        std::thread::sleep(Duration::from_millis(2));
    }
    release.send(()).unwrap();
    let refused = sender.join().unwrap();
    hosting.stop();
    assert!(peak <= limits.max_hello, "a message of {peak} bytes was read before the token was checked");
    assert_eq!(refused.as_deref(), Some("wrong token"));
}
