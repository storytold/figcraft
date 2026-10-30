//! Frame statistics for spotting wasted redraws: how many frames the UI drew, what asked for
//! them (the `file:line` of each repaint request) and how long building them took, read through
//! `ui.inspect` (`framesDrawn`, `repaintCauses`, `frameTimes`) and reset with
//! `ui.set {resetFrameStats: true}`. An idle window should draw no frames at all.

use std::collections::BTreeMap;

use serde_json::{Value, json};

/// How many of the most frequent repaint causes `ui.inspect` lists.
const TOP_CAUSES: usize = 12;

#[derive(Default)]
pub struct FrameStats {
    /// Frames drawn since the app started (or the last reset).
    pub frames: u64,
    /// Repaint requests by call site, counted once per frame they asked for.
    causes: BTreeMap<(&'static str, u32), u64>,
    /// The frames timed ([`Self::timed`]), their milliseconds together and the longest.
    timed: u64,
    total_ms: f64,
    max_ms: f64,
}

impl FrameStats {
    /// Count this frame and what asked for it (call once a frame).
    pub fn record(&mut self, ctx: &egui::Context) {
        self.frames += 1;
        for c in ctx.repaint_causes() {
            *self.causes.entry((c.file, c.line)).or_default() += 1;
        }
    }

    /// A frame took `ms` to build (the UI pass, before it is painted).
    pub fn timed(&mut self, ms: f64) {
        self.timed += 1;
        self.total_ms += ms;
        self.max_ms = self.max_ms.max(ms);
    }

    /// The frames timed since the last reset: `{frames, meanMs, maxMs}`.
    pub fn times(&self) -> Value {
        let mean = if self.timed == 0 { 0.0 } else { self.total_ms / self.timed as f64 };
        json!({"frames": self.timed, "meanMs": mean, "maxMs": self.max_ms})
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The most frequent repaint causes, most frequent first: `[{at: "file:line", count}]`.
    pub fn causes(&self) -> Value {
        let mut top: Vec<(&(&'static str, u32), &u64)> = self.causes.iter().collect();
        top.sort_by(|a, b| b.1.cmp(a.1));
        top.into_iter().take(TOP_CAUSES).map(|((file, line), n)| json!({"at": format!("{file}:{line}"), "count": n})).collect()
    }
}
