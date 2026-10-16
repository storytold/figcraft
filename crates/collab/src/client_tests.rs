//! A client's handling of the host's messages: sequence numbers and batch ids come from the host,
//! so they are treated as hostile input (they must never overflow).

use figcraft_doc::NodeKind;

use crate::message::{Accepted, ClientMsg, Hello, ServerMsg};
use crate::sim_tests::base;
use crate::{Client, Limits, NoRecompute, Outgoing, Server};

fn hello() -> Hello {
    let ClientMsg::Hello(h) = Client::hello("f", "tok", "Ada", "#FF8800") else { panic!("not a hello") };
    h
}

/// A welcome whose sequence numbers are what a lying host would send.
fn hostile_welcome(seq: u64, last_batch: u64) -> ServerMsg {
    let mut s = Server::new(base(), "tok", Limits::default());
    let (_, out) = s.join(hello()).unwrap();
    let mut welcome = out
        .into_iter()
        .find_map(|o| match o {
            Outgoing::To(_, m) => Some(m),
            _ => None,
        })
        .unwrap();
    let ServerMsg::Welcome { seq: seq0, processed, .. } = &mut welcome else { panic!("no welcome") };
    *seq0 = seq;
    processed.last_batch = last_batch;
    welcome
}

#[test]
fn hostile_sequence_numbers_do_not_overflow_the_client() {
    let mut c = Client::join(hostile_welcome(u64::MAX, u64::MAX)).unwrap();
    // This client's own first batch must not overflow its batch counter.
    let mut d = c.document().clone();
    let page = d.first_page().unwrap();
    d.create(page, NodeKind::Rectangle, "R", 0.0, 0.0, 10.0, 10.0).unwrap();
    assert!(c.edit(&d).unwrap().is_some(), "the edit is batched");
    // A batch the host claims follows `u64::MAX` can't exist: refuse it, don't overflow.
    let acc = Accepted { seq: 0, author: c.id().wrapping_add(1), batch_id: 0, changes: Vec::new() };
    assert!(c.receive(ServerMsg::Changes(acc), &NoRecompute).is_err());
}
