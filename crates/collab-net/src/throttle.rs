//! Latest-wins throttling: at most one value per interval goes out; a value offered sooner waits
//! (replacing any older waiting value) until the interval has passed, so the last one is never
//! lost. Used for presence on both ends (about 30 updates a second).

use std::time::Duration;

// `std::time::Instant` on native builds; the browser's clock in the web build.
use web_time::Instant;

/// How often presence goes out at most.
pub const PRESENCE_INTERVAL: Duration = Duration::from_millis(33);

/// How many messages other than presence go out at once, and per second after that: with presence
/// and pings that stays under what a server takes from a connection before it drops it.
const PACE_BURST: f64 = 200.0;
const PACE_RATE: f64 = 60.0;

/// Paces messages that must all go out (everything queued while offline leaves at once when the
/// connection comes back): `wait` says how long until the next may.
#[derive(Debug)]
pub struct Pace {
    tokens: f64,
    last: Instant,
}

impl Pace {
    pub fn new(now: Instant) -> Self {
        Self { tokens: PACE_BURST, last: now }
    }

    /// Take one message's turn at `now`: how long to wait before sending it (zero: at once).
    pub fn wait(&mut self, now: Instant) -> Duration {
        let passed = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + passed * PACE_RATE).min(PACE_BURST) - 1.0;
        if self.tokens >= 0.0 { Duration::ZERO } else { Duration::from_secs_f64(-self.tokens / PACE_RATE) }
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
