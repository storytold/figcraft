/// How often open documents get an autosave checkpoint (seconds).
pub const AUTOSAVE_SECS: f64 = 30.0 * 60.0;

/// Most changed layers whose names are checked for CJK scripts one by one (beyond: all of them).
pub(crate) const CJK_CHANGES: usize = 10_000;

/// With auto-save on: how long after a file last changed it is saved, and how long a failed save
/// waits before trying again (seconds).
pub const AUTO_SAVE_SECS: f64 = 3.0;

pub(crate) const AUTO_SAVE_RETRY_SECS: f64 = 60.0;

/// How long a toast shows (seconds).
pub(crate) const TOAST_SECS: f64 = 3.0;

/// How long a toast has been showing ([`UiState::toast`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ToastClock {
    /// Raised, not drawn yet.
    Raised,
    /// Drawn once; the next frame starts the clock.
    Drawn,
    /// Showing since this time (UI clock, seconds).
    Since(f64),
}

/// Draw another frame `secs` from now (nothing else needs drawing until then: an idle window draws
/// no frames).
#[track_caller]
pub(crate) fn repaint_in(ctx: &egui::Context, secs: f64) {
    if secs.is_finite() {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(secs.max(0.0)));
    }
}

/// A monotonic clock for background work (autosave, recovery, capture deadlines).
///
/// `Context::input(|i| i.time)` is the UI pass's time and is stale in `run_logic`; wall-clock time
/// can jump. `web_time::Instant` is monotonic on native and in the browser. Tests drive deadlines
/// directly with [`FrameClock::set_test_now`].
pub struct FrameClock {
    origin: web_time::Instant,
    test_now: Option<f64>,
}

impl Default for FrameClock {
    /// Start the frame clock at the current web time with no injected test time.
    fn default() -> Self {
        Self { origin: web_time::Instant::now(), test_now: None }
    }
}

impl FrameClock {
    /// Seconds since the app started (or the injected test time).
    pub fn now(&self) -> f64 {
        self.test_now.unwrap_or_else(|| self.origin.elapsed().as_secs_f64())
    }

    /// Set a fake monotonic time (tests); `None` returns to the real clock.
    pub fn set_test_now(&mut self, secs: Option<f64>) {
        self.test_now = secs;
    }
}
