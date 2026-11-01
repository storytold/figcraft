//! A hostile server can't grow a guest's memory without bound (#1121): the presence waiting for
//! the UI and the people a connection has seen hold at most [`MAX_PEERS`] others; past it,
//! someone new is left out, and someone leaving (or the UI taking the presence) makes room.

use std::sync::{Arc, Mutex};

use figcraft_collab::message::{Peer, Presence, ServerMsg};
use figcraft_engine::live::{Event, MAX_PEERS};

use crate::guest::GuestConfig;
use crate::inbox::Inbox;
use crate::invite::Invite;
use crate::join::{Seen, take};

fn inbox() -> Inbox {
    Inbox::new(Arc::new(|| {}))
}

#[test]
fn presence_from_more_people_than_the_cap_is_left_out() {
    let inbox = inbox();
    let over = MAX_PEERS as u32 + 500;
    for id in 0..over {
        inbox.presence(id, Presence::default());
    }
    // Someone already waiting is still updated.
    let moved = Presence { selection: Some(Vec::new()), ..Presence::default() };
    inbox.presence(3, moved.clone());
    let out = inbox.drain();
    assert_eq!(out.len(), MAX_PEERS);
    assert!(out.iter().all(|e| matches!(e, Event::Presence(id, _) if (*id as usize) < MAX_PEERS)));
    assert!(out.iter().any(|e| matches!(e, Event::Presence(3, p) if *p == moved)));
    // Taken by the UI: room again.
    inbox.presence(over, Presence::default());
    assert!(matches!(inbox.drain().as_slice(), [Event::Presence(id, _)] if *id == over));
}

#[test]
fn joins_past_the_cap_are_left_out_until_someone_leaves() {
    let inbox = inbox();
    let cfg = GuestConfig::new(Invite::new("127.0.0.1:47900", "0123456789abcdef", "AbCdEfGhIjKlMnOpQrStUv"), "Ada".into());
    let resume = Mutex::new(None);
    let mut seen = Seen::default();
    let mut send = |m: ServerMsg| {
        take(m, &cfg, &inbox, &mut seen, &resume, &|| {});
    };
    let joined = |id: u32| ServerMsg::PeerJoined(Peer { client_id: id, name: format!("P{id}"), color: String::new() });
    let over = MAX_PEERS as u32 + 500;
    for id in 0..over {
        send(joined(id));
    }
    let count = |events: &[Event]| events.iter().filter(|e| matches!(e, Event::PeerJoined(_))).count();
    assert_eq!(count(&inbox.drain()), MAX_PEERS, "joins past the cap reach the UI");
    // Someone already here may be renamed; someone left out stays out until someone leaves.
    send(joined(5));
    send(joined(over));
    assert!(matches!(inbox.drain().as_slice(), [Event::PeerJoined(p)] if p.client_id == 5));
    send(ServerMsg::PeerLeft { client_id: 7 });
    send(joined(over));
    assert!(matches!(inbox.drain().as_slice(), [Event::PeerLeft(7), Event::PeerJoined(p)] if p.client_id == over));
}
