//! Watching on another device: the browser build opened from the address Share ▸ View on another
//! device gives (`http://<host>:<port>/#view&join=<view link>`, served by the desktop app that
//! hosts the session) is only the prototype player, full-window, sized for a phone. It joins the
//! session with the view link, presents the file's prototype as soon as the file arrives and
//! follows every edit made on the desktop (the player copies the synced document each frame).
//! There is no editor to go back to: the player's bar has no Close and Escape does nothing; if the
//! presented frame goes away, the prototype starts again. Until there is something to present the
//! page says what it waits for.

use egui::{Color32, Ui};
use figcraft_engine::live::Status;
use serde_json::json;

use crate::FigApp;
use crate::i18n::tr;
use crate::present::{PresentScale, PresentView};
use crate::theme::regular;

/// The device viewer's state (the window is a viewer while `FigApp::ui.viewer` is set).
#[derive(Debug, Default)]
pub struct Viewer {
    /// The document revision presenting last failed on (nothing to present): tried again only
    /// when the file changes.
    failed: Option<u64>,
    /// The session was live once: when it is over, the file shown is no longer the shared one.
    joined: bool,
    /// Why the address couldn't be joined at all (no link in it, or one that can't be used).
    refused: Option<String>,
}

/// Make the window a device viewer and join `link` (the view link from the page's address).
pub fn open(app: &mut FigApp, link: Option<&str>) {
    app.ui.home = false;
    let refused = match link.map(str::trim).filter(|l| !l.is_empty()) {
        None => Some(tr("This address doesn't name a shared file. Open the address Share ▸ View on another device shows.").to_string()),
        Some(link) => app.session.execute("collab.join", &json!({"link": link})).err().map(|e| e.to_string()),
    };
    app.handle_ui_requests();
    app.ui.viewer = Some(Viewer { refused, ..Viewer::default() });
}

/// How the player shows a prototype on a phone: the width of the screen, and no device drawn
/// around it (the phone is the device). A tap where nothing reacts flashes the hotspots, as on the
/// desktop.
pub fn present_view() -> PresentView {
    PresentView { scale: PresentScale::FitWidth, device_frame: false, ..PresentView::default() }
}

/// What the viewer is doing, from the session's state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Joining: waiting for the file.
    Opening,
    /// The connection dropped; trying again.
    Reconnecting,
    /// The file is here: present it.
    Present,
    /// It couldn't join, or the session ended.
    Failed(String),
    Ended,
}

/// The stage for session status `status` (`syncing`: the file arrived; `joined`: it was live).
pub fn stage(status: &Status, syncing: bool, joined: bool) -> Stage {
    match status {
        Status::Live if syncing => Stage::Present,
        Status::Reconnecting { .. } => Stage::Reconnecting,
        Status::Failed { reason } => Stage::Failed(reason.clone()),
        Status::Off if joined => Stage::Ended,
        Status::Off | Status::Connecting | Status::Live => Stage::Opening,
    }
}

/// One frame of the viewer.
pub fn show(app: &mut FigApp, ui: &mut Ui) {
    let live = app.session.live();
    let (status, syncing) = (live.status.clone(), live.syncing());
    let viewer = app.ui.viewer.get_or_insert_default();
    viewer.joined |= status == Status::Live;
    let stage = match viewer.refused.clone() {
        Some(why) => Stage::Failed(why),
        None => stage(&status, syncing, viewer.joined),
    };
    match stage {
        Stage::Present if app.ui.present.is_none() => start(app),
        // What is open isn't the shared file any more.
        Stage::Ended | Stage::Failed(_) => app.ui.present = None,
        _ => {}
    }
    if app.ui.present.is_some() {
        crate::present::show(app, ui);
        return;
    }
    let note = match stage {
        Stage::Opening => tr("Opening the shared file…").to_string(),
        Stage::Reconnecting => tr("The connection dropped. Trying again…").to_string(),
        Stage::Present => tr("Nothing to present yet: add a frame to the file on the computer sharing it.").to_string(),
        Stage::Failed(reason) => format!("{}\n\n{reason}", tr("Couldn't open the shared file.")),
        Stage::Ended => tr("The session ended. Share ▸ View on another device on the computer gives a new address.").to_string(),
    };
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(BACKGROUND)).show(ui, |ui| {
        let rect = ui.max_rect();
        let galley = ui.painter().layout(note, regular(14.0), TEXT, (rect.width() - 48.0).max(120.0));
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, TEXT);
    });
}

/// The page behind the notes (the full-window presenter's dark bar colour), and their text.
const BACKGROUND: Color32 = Color32::from_rgb(0x1E, 0x1E, 0x1E);
const TEXT: Color32 = Color32::from_gray(200);

/// Present the session's file (its first flow, else its first frame), unless that failed on this
/// revision already.
fn start(app: &mut FigApp) {
    let Ok(rev) = app.session.ds().map(|d| d.revision) else { return };
    if app.ui.viewer.as_ref().is_some_and(|v| v.failed == Some(rev)) {
        return;
    }
    app.ui.present_view = present_view();
    let ok = app.session.execute("proto.present", &json!({})).is_ok();
    app.handle_ui_requests();
    let presenting = ok && app.ui.present.is_some();
    if let Some(v) = app.ui.viewer.as_mut() {
        v.failed = (!presenting).then_some(rev);
    }
}
