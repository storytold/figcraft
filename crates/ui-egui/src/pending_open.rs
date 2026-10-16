//! Files dropped on the home screen or the canvas, opened via File ▸ Open, or opened directly
//! from recents or command line arguments, open behind a card with an animated progress bar and
//! spinner. Heavy parsing and kiwi decoding of large `.fig` files runs off the UI thread so
//! the application remains completely responsive.

#[cfg(not(target_arch = "wasm32"))]
use base64::Engine as _;
use figcraft_doc::NodeId;
use serde_json::Value;

use crate::FigApp;
use crate::i18n::{tr, trf, trn};
use crate::theme::{Tokens, semibold};

/// A dropped or opened file waiting to be opened (`file.open`, a document of its own) or imported
/// (`file.import`, into the current document).
pub struct PendingOpen {
    /// What the card shows for it (the file name).
    pub label: String,
    pub params: Value,
    pub command: &'static str,
    /// Canvas imports stay with the document and page that received the drop.
    pub target: Option<ImportTarget>,
    #[cfg(not(target_arch = "wasm32"))]
    pub worker: Option<std::thread::JoinHandle<std::result::Result<(figcraft_format::Loaded, bool), String>>>,
}

impl PendingOpen {
    pub fn new(label: String, params: Value, command: &'static str, target: Option<ImportTarget>) -> Self {
        Self {
            label,
            params,
            command,
            target,
            #[cfg(not(target_arch = "wasm32"))]
            worker: None,
        }
    }
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

/// Draw the modal progress card with animated progress bar and spinner.
fn draw_dialog(app: &FigApp, ctx: &egui::Context) {
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
    crate::dialogs::window("", ctx).id(egui::Id::new("pending_open")).title_bar(false).movable(false).interactable(false).fade_in(false).show(
        ctx,
        |ui| {
            ui.set_width(480.0);
            ui.label(egui::RichText::new(title).font(semibold(13.0)));
            ui.add_space(12.0);
            for (i, p) in pending.iter().enumerate() {
                ui.horizontal(|ui| {
                    if i == 0 {
                        ui.add(egui::Spinner::new().size(16.0));
                    }
                    ui.label(p.label.as_str());
                });
            }
            ui.add_space(16.0);
            let time = ui.input(|i| i.time);
            let progress = ((time * 0.4).fract() as f32).clamp(0.05, 0.95);
            ui.add(egui::ProgressBar::new(progress).animate(true).fill(t.accent));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(12.0));
                ui.label(egui::RichText::new(tr("Opening the file…")).color(t.text_secondary));
            });
        },
    );
}

/// The card while files wait, then (once it has been on screen) open them. Called before the import
/// report, so a report for these files shows after them.
pub fn pending_opens(app: &mut FigApp, ctx: &egui::Context) {
    if app.ui.pending_opens.is_empty() {
        app.ui.pending_open_frames = 0;
        return;
    }
    ctx.request_repaint();

    if app.ui.pending_open_frames < FRAMES_BEFORE_OPENING {
        app.ui.pending_open_frames += 1;
        draw_dialog(app, ctx);
        return;
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(pending) = app.ui.pending_opens.first_mut()
            && pending.command == "file.open"
        {
            let path_opt = pending.params.get("path").and_then(Value::as_str);
            // Already open: switch to it immediately
            if let Some(path) = path_opt
                && let Some(i) = app.session.docs.iter().position(|d| d.path.as_deref() == Some(path))
            {
                app.session.active = i;
                figcraft_engine::cmd::file::remember(&mut app.session, path);
                app.ui.pending_opens.remove(0);
                app.ui.home = false;
                if !app.ui.pending_opens.is_empty() {
                    draw_dialog(app, ctx);
                }
                return;
            }

            if pending.worker.is_none() {
                let path = path_opt.map(ToString::to_string);
                let data = pending.params.get("data").and_then(Value::as_str).map(ToString::to_string);
                let name = pending.label.clone();
                let variables = pending.params.get("variables").cloned();

                let spawned = std::thread::Builder::new().name("figcraft-file-open".into()).spawn(move || {
                    if let Some(p) = path.as_deref() {
                        figcraft_engine::cmd::file::load_path(p, variables.as_ref()).map_err(|e| e.to_string())
                    } else if let Some(d) = data {
                        let d = d.split_once(',').map_or(d.as_str(), |(_, b)| b);
                        let bytes = base64::engine::general_purpose::STANDARD.decode(d.trim()).map_err(|e| e.to_string())?;
                        figcraft_engine::cmd::file::load_bytes_with(&bytes, &name, variables.as_ref()).map_err(|e| e.to_string())
                    } else {
                        Err("No file path or data provided".to_string())
                    }
                });

                match spawned {
                    Ok(handle) => {
                        pending.worker = Some(handle);
                    }
                    Err(_) => {
                        let item = app.ui.pending_opens.remove(0);
                        let command = item.command;
                        if run_pending(app, item).is_some() && command == "file.open" {
                            app.ui.home = false;
                        }
                        return;
                    }
                }
            }

            #[cfg(not(test))]
            let is_finished = pending.worker.as_ref().is_some_and(|h| h.is_finished());

            #[cfg(test)]
            let is_finished = true;

            if !is_finished {
                draw_dialog(app, ctx);
                return;
            }

            let mut item = app.ui.pending_opens.remove(0);
            let result = match item.worker.take() {
                Some(h) => match h.join() {
                    Ok(res) => res,
                    Err(_) => Err("Failed to open file".to_string()),
                },
                None => Err("Failed to open file".to_string()),
            };

            match result {
                Ok((loaded, rest)) => {
                    let path = item.params.get("path").and_then(Value::as_str);
                    figcraft_engine::cmd::file::finish_open(&mut app.session, path, &item.label, loaded, rest);
                    app.ui.home = false;
                }
                Err(err) => {
                    cancel(app, err);
                }
            }

            if !app.ui.pending_opens.is_empty() {
                draw_dialog(app, ctx);
            }
            return;
        }
    }

    let pending = app.ui.pending_opens.remove(0);
    let command = pending.command;
    if run_pending(app, pending).is_some() && command == "file.open" {
        app.ui.home = false;
    }
    if !app.ui.pending_opens.is_empty() {
        draw_dialog(app, ctx);
    }
}
