//! Files dropped on the home screen or the canvas open a frame later, behind a card that says what
//! is happening: opening a large `.fig` blocks the UI thread for a while, and the card is on screen
//! before it does (rather than a window that freezes without a word). A file with no path to read
//! (every drop in the browser) waits behind the card until its contents arrive.

use std::sync::Arc;
use std::task::{Context as TaskContext, Poll, Waker};

use figcraft_doc::NodeId;
use serde_json::{Value, json};

use crate::i18n::{tr, trf, trn};
use crate::theme::{Tokens, semibold};
use crate::{FigApp, ReadFuture, RepaintWaker};

/// A dropped file waiting to be opened (`file.open`, a document of its own) or imported
/// (`file.import`, into the current document).
pub struct PendingOpen {
    /// What the card shows for it (the file name).
    pub label: String,
    pub params: Value,
    pub command: &'static str,
    /// Canvas imports stay with the document and page that received the drop.
    pub target: Option<ImportTarget>,
    /// The file's contents still on their way; they go into `params` as `data` once read.
    pub read: Option<ReadFuture>,
}

pub struct ImportTarget {
    pub uid: u64,
    pub page: NodeId,
    pub name: String,
}

/// Queues dropped file `f` to open as a document of its own: by path, or by its contents when there
/// is no file on disk to read. Callers checked its extension, so there is a name to tell the format by.
pub(crate) fn open_dropped(app: &mut FigApp, f: &egui::DroppedFileHandle) {
    let path = f.path();
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let (params, read) =
        if path.exists() { (json!({"path": path.to_string_lossy()}), None) } else { (json!({"name": name}), Some(crate::dropped_bytes(f, &name))) };
    app.ui.pending_opens.push(PendingOpen { label: name, params, command: "file.open", target: None, read });
}

fn run_pending(app: &mut FigApp, pending: PendingOpen) -> Option<Value> {
    let target = match pending.target {
        Some(target) => target,
        None if pending.command != "file.import" => return app.run_user(pending.command, pending.params),
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
    let result = app.run_user(pending.command, pending.params);
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
    app.show_error(message);
}

/// Space between files dropped together on the canvas.
const IMPORT_GAP: f64 = 20.0;

/// Frames the card is drawn before the files open: egui lays a new card out invisibly on its first
/// frame, so the second one is the first the window shows.
const FRAMES_BEFORE_OPENING: u8 = 2;

/// Polls the contents still being read: those that arrived go into their file's parameters, files
/// that couldn't be read are dropped with a message. Whether any are still on their way.
fn read_contents(app: &mut FigApp, ctx: &egui::Context) -> bool {
    let waker = Waker::from(Arc::new(RepaintWaker(ctx.clone())));
    let mut cx = TaskContext::from_waker(&waker);
    let mut reading = false;
    let mut unread = Vec::new();
    app.ui.pending_opens.retain_mut(|pending| {
        let Some(read) = pending.read.as_mut() else { return true };
        match read.as_mut().poll(&mut cx) {
            Poll::Pending => {
                reading = true;
                true
            }
            Poll::Ready(Ok(bytes)) => {
                use base64::Engine as _;
                pending.params["data"] = json!(base64::engine::general_purpose::STANDARD.encode(&bytes));
                pending.read = None;
                true
            }
            Poll::Ready(Err(why)) => {
                unread.push(why);
                false
            }
        }
    });
    for why in unread {
        app.show_error(why);
    }
    reading
}

/// The card while files wait, then (once it has been on screen and their contents are read) open
/// them. Called before the import report, so a report for these files shows after them.
pub fn pending_opens(app: &mut FigApp, ctx: &egui::Context) {
    let reading = read_contents(app, ctx);
    if app.ui.pending_opens.is_empty() {
        app.ui.pending_open_frames = 0;
        return;
    }
    ctx.request_repaint();
    if app.ui.pending_open_frames >= FRAMES_BEFORE_OPENING && !reading {
        app.ui.pending_open_frames = 0;
        // Files dropped together land side by side: each starts where the previous one ends.
        let mut next_x: Option<f64> = None;
        for mut pending in std::mem::take(&mut app.ui.pending_opens) {
            let command = pending.command;
            let canvas = command == "file.import" && pending.params.get("x").is_some();
            if canvas && let Some(x) = next_x {
                pending.params["x"] = json!(x);
            }
            let page = pending.target.as_ref().map(|t| (t.uid, t.page));
            let result = run_pending(app, pending);
            if result.is_some() && command == "file.open" {
                app.ui.home = false;
            }
            if canvas {
                let right = result.as_ref().and_then(|r| r.get("id")?.as_u64()).and_then(|id| {
                    let doc = &app.session.docs.iter().find(|d| Some(d.uid) == page.map(|p| p.0))?.doc;
                    doc.world_bounds(NodeId(id)).map(|b| b.x1)
                });
                next_x = right.map(|r| r + IMPORT_GAP).or(next_x);
            }
        }
        return;
    }
    app.ui.pending_open_frames = app.ui.pending_open_frames.saturating_add(1);
    let title = match app.ui.pending_opens.first().map(|p| p.command) {
        Some("file.import" | "image.replace") => {
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
    crate::dialogs::window("", ctx).id(egui::Id::unique("pending_open")).title_bar(false).movable(false).interactable(false).fade_in(false).show(
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
