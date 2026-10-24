//! Where the network threads leave what happened for the UI thread, which takes it with
//! [`Inbox::drain`] when the waker gets it to draw a frame. Presence is latest-wins per peer, so
//! an app that polls rarely (a headless agent) never piles up cursor moves.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use figcraft_engine::live::{Event, Presence};

/// Wakes the UI (the desktop app passes egui's `request_repaint`).
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// Most discrete events kept between two drains (joins, leaves, status); the oldest go first.
/// Document sync events don't count and are never dropped: the sync client treats a missing
/// batch as a fatal sequence gap, so losing one would end the session.
const MAX_EVENTS: usize = 10_000;

#[derive(Default)]
struct State {
    /// Arrival number of the next event, so [`Inbox::drain`] can merge the two queues in order.
    next: u64,
    /// [`Event::Sync`] events, never dropped.
    syncs: VecDeque<(u64, Event)>,
    /// Every other event, capped at [`MAX_EVENTS`].
    others: VecDeque<(u64, Event)>,
    presence: BTreeMap<u32, Presence>,
}

/// One session's mailbox (a new session gets a new one, so a stopped session's threads can't
/// reach the next).
#[derive(Clone)]
pub struct Inbox {
    state: Arc<Mutex<State>>,
    wake: Waker,
}

impl Inbox {
    pub fn new(wake: Waker) -> Self {
        Self { state: Arc::default(), wake }
    }

    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let r = f(&mut self.state.lock().unwrap_or_else(PoisonError::into_inner));
        (self.wake)();
        r
    }

    pub fn push(&self, e: Event) {
        self.with(|s| {
            if let Event::PeerLeft(id) = &e {
                s.presence.remove(id);
            }
            let n = s.next;
            s.next += 1;
            if matches!(e, Event::Sync(_)) {
                s.syncs.push_back((n, e));
                return;
            }
            if s.others.len() >= MAX_EVENTS {
                log::warn!("live session: {MAX_EVENTS} events not taken; dropping the oldest");
                s.others.pop_front();
            }
            s.others.push_back((n, e));
        });
    }

    /// Peer `id`'s latest presence (replaces one not taken yet).
    pub fn presence(&self, id: u32, p: Presence) {
        self.with(|s| {
            s.presence.insert(id, p);
        });
    }

    /// Everything since the last drain: the events in order, then the latest presence of each
    /// peer.
    pub fn drain(&self) -> Vec<figcraft_engine::live::Event> {
        let mut s = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut syncs = std::mem::take(&mut s.syncs).into_iter().peekable();
        let mut others = std::mem::take(&mut s.others).into_iter().peekable();
        let mut out = Vec::with_capacity(syncs.len() + others.len() + s.presence.len());
        loop {
            let sync_first = match (syncs.peek(), others.peek()) {
                (Some((a, _)), Some((b, _))) => a < b,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            let next = if sync_first { syncs.next() } else { others.next() };
            out.extend(next.map(|(_, e)| e));
        }
        out.extend(std::mem::take(&mut s.presence).into_iter().map(|(id, p)| Event::Presence(id, p)));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use figcraft_collab::message::ServerMsg;

    fn reject(batch_id: u64) -> Event {
        Event::Sync(ServerMsg::Reject { batch_id, reason: String::new(), fixes: Vec::new() })
    }

    fn sync_ids(events: &[Event]) -> Vec<u64> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Sync(ServerMsg::Reject { batch_id, .. }) => Some(*batch_id),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_full_inbox_never_drops_sync_events() {
        let inbox = Inbox::new(Arc::new(|| {}));
        inbox.push(reject(1));
        for _ in 0..MAX_EVENTS {
            inbox.push(Event::PeerLeft(7));
        }
        inbox.push(reject(2));
        for _ in 0..MAX_EVENTS {
            inbox.push(Event::PeerLeft(7));
        }
        let out = inbox.drain();
        assert_eq!(sync_ids(&out), vec![1, 2]);
        assert!(out.len() <= MAX_EVENTS + 2, "{} events kept", out.len());
    }

    #[test]
    fn sync_events_past_the_cap_are_all_kept_in_order() {
        let inbox = Inbox::new(Arc::new(|| {}));
        let n = MAX_EVENTS as u64 + 5;
        for id in 0..n {
            inbox.push(reject(id));
        }
        inbox.push(Event::PeerLeft(7));
        let out = inbox.drain();
        let ids = sync_ids(&out);
        assert_eq!(ids.len() as u64, n, "Sync events dropped");
        assert!(ids.iter().copied().eq(0..n), "Sync events out of order");
    }
}
