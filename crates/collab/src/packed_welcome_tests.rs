//! The document in a welcome goes compressed to clients that read that ([`message::PACKED`],
//! #1665): it shrinks several times over, comes back unchanged, and a hostile packed document
//! can't make the client allocate past the size an unpacked welcome could have had.

use std::io::Write;

use figcraft_doc::{Document, NodeKind};

use crate::message::{self, CatchUp, ClientMsg, ServerMsg};
use crate::{Client, Error, Limits, NoRecompute, Outgoing, Replica, Server};

/// A design file of 40 screens with 100 cards each, as a server's first welcome.
fn welcome() -> ServerMsg {
    let mut doc = Document::new("Synthetic");
    let page = doc.first_page().unwrap();
    for f in 0..40 {
        let screen =
            doc.create(page, NodeKind::Frame, &format!("Screen {f}"), f64::from(f % 8) * 1500.0, f64::from(f / 8) * 1000.0, 1440.0, 900.0).unwrap();
        for i in 0..100 {
            let (x, y) = (f64::from(i % 10) * 140.0, f64::from(i / 10) * 88.0);
            doc.create(screen, NodeKind::Rectangle, &format!("Card {i} of screen {f}"), x, y, 128.0, 80.0).unwrap();
        }
    }
    doc.repair();
    let mut s = Server::new(doc, "tok", Limits::default());
    let ClientMsg::Hello(h) = Client::hello("f", "tok", "Ada", "#FF8800") else { panic!("not a hello") };
    let (_, out) = s.join(h).unwrap();
    out.into_iter().find_map(|o| if let Outgoing::To(_, m @ ServerMsg::Welcome { .. }) = o { Some(m) } else { None }).unwrap()
}

fn with_catch_up(mut msg: ServerMsg, c: CatchUp) -> ServerMsg {
    if let ServerMsg::Welcome { catch_up, .. } = &mut msg {
        *catch_up = c;
    }
    msg
}

fn deflated(bytes: &[u8]) -> Vec<u8> {
    let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
    z.write_all(bytes).unwrap();
    z.finish().unwrap()
}

#[test]
fn a_packed_welcome_is_much_smaller_and_reads_back_the_same() {
    let plain = welcome();
    let mut packed = plain.clone();
    message::pack(&mut packed).unwrap();
    assert!(matches!(packed, ServerMsg::Welcome { catch_up: CatchUp::Packed(_), .. }));
    let (before, after) = (message::encode(&plain).unwrap().len(), message::encode(&packed).unwrap().len());
    println!("welcome of 4,040 layers: {before} bytes, packed {after} bytes");
    assert!(after * 5 < before, "{before} → {after}");
    let mut back: ServerMsg = message::decode(&message::encode(&packed).unwrap(), after).unwrap();
    message::unpack(&mut back, before).unwrap();
    assert_eq!(back, plain);
    // The client opens the same document from it.
    let ServerMsg::Welcome { catch_up: CatchUp::Snapshot(changes), .. } = &plain else { panic!("not a snapshot") };
    assert_eq!(Client::join(back).unwrap().document(), Replica::from_snapshot(changes).unwrap().document());
}

#[test]
fn only_a_welcomes_whole_document_is_packed() {
    for msg in [with_catch_up(welcome(), CatchUp::Delta(Vec::new())), ServerMsg::Pong, ServerMsg::PeerLeft { client_id: 3 }] {
        let mut m = msg.clone();
        message::pack(&mut m).unwrap();
        assert_eq!(m, msg, "left as it is");
        message::unpack(&mut m, 0).unwrap();
        assert_eq!(m, msg, "nothing to unpack");
    }
}

#[test]
fn packed_documents_are_untrusted() {
    let mut packed = welcome();
    message::pack(&mut packed).unwrap();
    let ServerMsg::Welcome { catch_up: CatchUp::Snapshot(changes), .. } = welcome() else { panic!("not a snapshot") };
    let size = postcard::to_allocvec(&changes).unwrap().len();
    // Unpacked, it may be no larger than an unpacked welcome could have been.
    assert!(matches!(message::unpack(&mut packed.clone(), size - 1), Err(Error::TooLarge(n)) if n == size - 1));
    message::unpack(&mut packed, size).unwrap();
    // A bomb stops at the limit instead of filling memory.
    let mut bomb = with_catch_up(welcome(), CatchUp::Packed(deflated(&vec![0; 64 << 20])));
    assert!(matches!(message::unpack(&mut bomb, 1 << 20), Err(Error::TooLarge(_))));
    // Not deflate, not changes, or changes with bytes after them.
    let mut good = postcard::to_allocvec(&changes).unwrap();
    for bytes in [
        vec![0xFF; 64],
        deflated(b"not a document"),
        deflated(&{
            good.push(0);
            good
        }),
    ] {
        let mut m = with_catch_up(welcome(), CatchUp::Packed(bytes));
        assert!(matches!(message::unpack(&mut m, usize::MAX), Err(Error::Decode(_))), "{m:?}");
    }
}

#[test]
fn clients_announce_that_they_read_packed_welcomes() {
    assert!(message::reads_packed(&message::client_version()));
    assert!(!message::reads_packed("0.12.1 signals hosting comments"), "a client from before");
}

/// The sync client reads only what the transport unpacked: a packed document reaching it is a
/// mistake it refuses, on the first welcome and on one after a dropped connection.
#[test]
fn the_sync_client_wants_the_document_unpacked() {
    let packed = with_catch_up(welcome(), CatchUp::Packed(Vec::new()));
    assert!(Client::join(packed.clone()).is_err());
    let mut c = Client::join(welcome()).unwrap();
    c.disconnect();
    assert!(matches!(c.receive(packed, &NoRecompute), Err(Error::Protocol(_))));
}
