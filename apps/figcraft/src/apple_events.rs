//! macOS open-documents and quit Apple events (#289).
//!
//! Finder double-clicks, Open With, drops on the Dock icon and `open -a FigCraft file.figcraft`
//! don't pass paths on the command line: LaunchServices sends the running (or just-launched) app a
//! `kAEOpenDocuments` ('odoc') Apple event. winit 0.30 doesn't handle it and owns the
//! `NSApplicationDelegate`, so AppKit answered "FigCraft cannot open files in the FigCraft Document
//! format". Handling it ourselves needs Objective-C class declarations, i.e. `unsafe`, which this
//! workspace forbids. The `fmv-macos-events` crate (also used by GridCraft, EffectCraft and
//! PhotoCraft) wraps exactly that, an `NSAppleEventManager` handler registered before Finder's
//! launch event that leaves winit's delegate alone, behind a safe main-thread API.

use std::path::PathBuf;

use figcraft_ui_egui::FigApp;
use fmv_macos_events::{Event, Inbox, Registration};
use serde_json::json;

/// Keeps the Apple-event handlers registered; hold it until the event loop returns.
pub struct AppleEvents {
    _registration: Registration,
    inbox: Inbox,
}

impl AppleEvents {
    /// Register the handlers. Call on the main thread before the event loop starts, so the event
    /// that launched the app (a Finder double-click) is caught too.
    pub fn install() -> Self {
        let (registration, inbox) = Registration::install();
        Self { _registration: registration, inbox }
    }

    /// The queue the app drains every frame ([`poll`]); events arriving later wake `ctx`.
    pub fn connect(&self, ctx: &egui::Context) -> Inbox {
        let ctx = ctx.clone();
        self.inbox.set_wake(move || ctx.request_repaint());
        self.inbox.clone()
    }
}

/// Open the documents and act on the quit requests that arrived since the last frame.
pub fn poll(inbox: &Inbox, fig: &mut FigApp, ctx: &egui::Context) {
    for e in inbox.drain() {
        match e {
            Event::Open(paths) => open_documents(fig, paths),
            Event::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }
}

/// Open `paths` as File › Open does; a file that fails shows its error and the others still open.
pub fn open_documents(fig: &mut FigApp, paths: Vec<PathBuf>) {
    for p in paths {
        if fig.run_user("file.open", json!({"path": p.to_string_lossy()})).is_some() {
            fig.ui.home = false;
        }
    }
}
