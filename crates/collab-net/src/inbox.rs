//! Where the network threads leave what happened for the UI thread, which takes it with
//! [`Inbox::drain`] when the waker gets it to draw a frame. Presence is latest-wins per peer, so
//! an app that polls rarely (a headless agent) never piles up cursor moves.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use figcraft_engine::live::{Event, Presence};

/// Wakes the UI (the desktop app passes egui's `request_repaint`).
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// Most discrete events kept between two drains (joins, leaves, status); the oldest go first.
const MAX_EVENTS: usize = 10_000;

#[derive(Default)]
struct State {
    events: VecDeque<Event>,
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
            if matches!(e, Event::AssetsChanged) && s.events.iter().any(|old| matches!(old, Event::AssetsChanged)) {
                return;
            }
            if let Event::PeerLeft(id) = &e {
                s.presence.remove(id);
            }
            if s.events.len() >= MAX_EVENTS {
                log::warn!("live session: {MAX_EVENTS} events not taken; dropping the oldest");
                s.events.pop_front();
            }
            s.events.push_back(e);
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
        let mut out: Vec<Event> = s.events.drain(..).collect();
        out.extend(std::mem::take(&mut s.presence).into_iter().map(|(id, p)| Event::Presence(id, p)));
        out
    }
}
