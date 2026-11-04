//! Latest-wins throttling: at most one value per interval goes out; a value offered sooner waits
//! (replacing any older waiting value) until the interval has passed, so the last one is never
//! lost. Used for presence on both ends (about 30 updates a second).

use std::collections::VecDeque;
use std::time::Duration;

// `std::time::Instant` on native builds; the browser's clock in the web build.
use web_time::Instant;

/// How often presence goes out at most.
pub const PRESENCE_INTERVAL: Duration = Duration::from_millis(33);

/// How many messages other than presence go out at once, and per second after that: with presence
/// and pings that stays under what a server takes from a connection before it drops it.
const PACE_BURST: f64 = 200.0;
const PACE_RATE: f64 = 60.0;

/// Messages that must all go out, in order, at a pace (everything queued while offline leaves at
/// once when the connection comes back). The pace is counted when they go, so messages held up by
/// a stalled connection don't leave together afterwards.
#[derive(Debug)]
pub struct Paced<T> {
    tokens: f64,
    last: Instant,
    queue: VecDeque<T>,
}

impl<T> Paced<T> {
    pub fn new(now: Instant) -> Self {
        Self { tokens: PACE_BURST, last: now, queue: VecDeque::new() }
    }

    pub fn push(&mut self, v: T) {
        self.queue.push_back(v);
    }

    /// The next message, if its turn has come at `now`.
    pub fn take_due(&mut self, now: Instant) -> Option<T> {
        if self.queue.is_empty() {
            return None;
        }
        self.tokens = (self.tokens + now.saturating_duration_since(self.last).as_secs_f64() * PACE_RATE).min(PACE_BURST);
        self.last = now;
        // Within rounding of a whole turn counts as one: the time `due` gave has come.
        if self.tokens < 1.0 - 1e-6 {
            return None;
        }
        self.tokens -= 1.0;
        self.queue.pop_front()
    }

    /// When the next message's turn comes (`None`: there's none).
    pub fn due(&self) -> Option<Instant> {
        self.queue.front()?;
        // Rounded up to whole microseconds, so a timer set for this always finds the turn come.
        let wait = ((1.0 - self.tokens).max(0.0) / PACE_RATE * 1e6).ceil() as u64;
        Some(self.last + Duration::from_micros(wait))
    }
}

#[derive(Debug)]
pub struct Throttle<T> {
    interval: Duration,
    last: Option<Instant>,
    pending: Option<T>,
}

impl<T> Throttle<T> {
    pub fn new(interval: Duration) -> Self {
        Self { interval, last: None, pending: None }
    }

    /// Offer `v` at `now`: returns it when it may go out now, else keeps it for [`take_due`](Self::take_due).
    pub fn offer(&mut self, v: T, now: Instant) -> Option<T> {
        if self.last.is_some_and(|l| now.saturating_duration_since(l) < self.interval) {
            self.pending = Some(v);
            return None;
        }
        self.last = Some(now);
        self.pending = None;
        Some(v)
    }

    /// When the waiting value may go out (none waiting: `None`).
    pub fn due(&self) -> Option<Instant> {
        self.pending.as_ref()?;
        Some(self.last.map_or_else(Instant::now, |l| l + self.interval))
    }

    /// The waiting value, once its time has come.
    pub fn take_due(&mut self, now: Instant) -> Option<T> {
        if self.due().is_some_and(|d| d <= now) {
            self.last = Some(now);
            return self.pending.take();
        }
        None
    }
}

/// The earliest of some deadlines.
pub fn earliest(times: impl IntoIterator<Item = Option<Instant>>) -> Option<Instant> {
    times.into_iter().flatten().min()
}
