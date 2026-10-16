//! Files dropped on the home screen or the canvas open a frame later, behind a card that says what
//! is happening: opening a large `.fig` blocks the UI thread for a while, and the card is on screen
//! before it does (rather than a window that freezes without a word).

use figcraft_doc::NodeId;
use serde_json::Value;

use crate::FigApp;
use crate::i18n::{tr, trf, trn};
use crate::theme::{Tokens, semibold};

/// A dropped file waiting to be opened (`file.open`, a document of its own) or imported
/// (`file.import`, into the current document).
pub struct PendingOpen {
    /// What the card shows for it (the file name).
    pub label: String,
    pub params: Value,
    pub command: &'static str,
    /// Canvas imports stay with the document and page that received the drop.
    pub target: Option<ImportTarget>,
}

pub struct ImportTarget {
    pub uid: u64,
    pub page: NodeId,
    pub name: String,
}

fn run_pending(app: &mut FigApp, pending: PendingOpen) -> Option<Value> {
    let target = match pending.target {
        Some(target) => target,
        None if pending.command != "file.import" => return app.run(pending.command, pending.params),
        None => {
            cancel(app, tr("The destination document is unavailable. Nothing was imported.").to_string());
            return None;
        }
    };
    let Some(index) = app.session.docs.iter().position(|doc| doc.uid == target.uid) else {
        cancel(app, tr("The destination document was closed. Nothing was imported.").to_string());
        return None;
    };
    if !app.session.docs[index].doc.pages.contains(&target.page) {
        cancel(app, tr("The destination page was removed. Nothing was imported.").to_string());
        return None;
    }

    let previous = app.session.ds().ok().map(|doc| doc.uid);
    let previous_page = app.session.docs[index].page;
    let previous_selection = app.session.docs[index].selection.clone();
    app.session.active = index;
    app.session.docs[index].page = target.page;
    let result = app.run(pending.command, pending.params);
    if previous_page != target.page
        && let Some(doc) = app.session.docs.iter_mut().find(|doc| doc.uid == target.uid)
        && doc.doc.pages.contains(&previous_page)
    {
        doc.page = previous_page;
        doc.selection = previous_selection;
        doc.clean_selection();
    }
    if let Some(index) = previous.and_then(|uid| app.session.docs.iter().position(|doc| doc.uid == uid)) {
        app.session.active = index;
    }
    result
}

/// An import that has no destination left: nothing is imported, and the user is told why.
fn cancel(app: &mut FigApp, message: String) {
    app.ui.last_error = Some(message.clone());
    app.toast(message);
}

/// Frames the card is drawn before the files open: egui lays a new card out invisibly on its first
/// frame, so the second one is the first the window shows.
const FRAMES_BEFORE_OPENING: u8 = 2;

/// The card while files wait, then (once it has been on screen) open them. Called before the import
/// report, so a report for these files shows after them.
pub fn pending_opens(app: &mut FigApp, ctx: &egui::Context) {
    if app.ui.pending_opens.is_empty() {
        app.ui.pending_open_frames = 0;
        return;
    }
    ctx.request_repaint();
    if app.ui.pending_open_frames >= FRAMES_BEFORE_OPENING {
        app.ui.pending_open_frames = 0;
        for pending in std::mem::take(&mut app.ui.pending_opens) {
            let command = pending.command;
            if run_pending(app, pending).is_some() && command == "file.open" {
                app.ui.home = false;
            }
        }
        return;
    }
    app.ui.pending_open_frames += 1;
    let title = match app.ui.pending_opens.first().map(|p| p.command) {
        Some("file.import") => {
            let name = app
                .ui
                .pending_opens
                .first()
                .and_then(|pending| pending.target.as_ref().map(|target| target.name.clone()))
                .or_else(|| app.session.doc().ok().map(|d| d.name.clone()))
                .unwrap_or_default();
            trf("Adding to {name}", &[("name", &name)])
        }
        _ => trn("Opening {n} files as new files", app.ui.pending_opens.len(), &[]),
    };
    let t = Tokens::get(ctx);
    let pending = &app.ui.pending_opens;
    // Shown at once: the frame after this one may block the window while the file opens.
    crate::dialogs::window("", ctx).id(egui::Id::new("pending_open")).title_bar(false).movable(false).interactable(false).fade_in(false).show(
        ctx,
        |ui| {
            ui.set_width(480.0);
            ui.label(egui::RichText::new(title).font(semibold(13.0)));
            ui.add_space(12.0);
            for p in pending {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(16.0));
                    ui.label(p.label.as_str());
                });
            }
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(12.0));
                ui.label(egui::RichText::new(tr("Opening the file…")).color(t.text_secondary));
            });
        },
    );
}
