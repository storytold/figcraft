//! A hostile server can't grow a client's list of people without bound (#1121): past
//! [`MAX_PEERS`], someone new is left out (their presence and chat with them), in a welcome or one
//! join at a time; someone leaving makes room again.

use serde_json::json;

use crate::live::{Event, MAX_PEERS, Peer, Presence, Role};
use crate::live_tests::{info, peer, push, session};

fn many(from: u32, n: usize) -> Vec<Peer> {
    (from..).take(n).map(|id| peer(id, "Someone")).collect()
}

#[test]
fn a_flood_of_joins_keeps_at_most_the_cap() {
    let (mut s, script) = session();
    s.execute("collab.join", &json!({"link": "figcraft://join?host=a:1&file=f&token=t"})).unwrap();
    push(&script, vec![Event::Welcome { info: info(Role::Guest, 1), peers: many(2, MAX_PEERS + 100) }]);
    s.live_poll();
    assert_eq!(s.live().peers.len(), MAX_PEERS, "a welcome listing too many");
    let first_left_out = 2 + MAX_PEERS as u32;
    let joins = many(first_left_out, 1_000).into_iter().map(Event::PeerJoined).collect();
    push(&script, joins);
    s.live_poll();
    assert_eq!(s.live().peers.len(), MAX_PEERS, "joins past the cap");
    // Presence of someone left out isn't kept either.
    push(&script, vec![Event::Presence(first_left_out, Presence::default())]);
    s.live_poll();
    assert!(!s.live().presence.contains_key(&first_left_out));
    // Someone already here may still be renamed; someone leaving makes room.
    push(&script, vec![Event::PeerJoined(peer(2, "Renamed")), Event::PeerLeft(3), Event::PeerJoined(peer(first_left_out, "Late"))]);
    s.live_poll();
    assert_eq!(s.live().peers.len(), MAX_PEERS);
    assert_eq!(s.live().peers[&2].name, "Renamed");
    assert!(s.live().peers.contains_key(&first_left_out) && !s.live().peers.contains_key(&3));
}
