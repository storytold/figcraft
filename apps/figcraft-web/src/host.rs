//! Host modes: a page that runs FigCraft (as an `<iframe>`, or in its own document) opens
//! documents in it and stores what it saves instead of the browser downloading. The messages
//! and the two transports are `craft-web-host`'s (storytold/craft-libs): the same contract as
//! the other Craft apps' bridges. This module is FigCraft's part of it.
//!
//! - `?host=parent` (an `<iframe>`, such as a Nextcloud app) talks to the same-origin parent over
//!   `postMessage`; `data-host="page"` on the canvas (the page's own document) uses DOM
//!   `CustomEvent`s on the canvas instead. `?host=parent` wins when both are present.
//! - `figcraft:ready` (to the host) once the app is running and listening; send a file now.
//! - `figcraft:open` (from the host): `{ name, bytes }`. It opens exactly as a file picked with
//!   File › Open or dropped on the window would (the app's own open queue, with its "Opening as
//!   a new file" card). A payload over the browser's own open limit
//!   ([`figcraft_ui_egui::browser_can_read`]'s 512 MB) is refused before its bytes are copied,
//!   and anything malformed is ignored. FigCraft reads whole files, so it says `file: false` in
//!   `ready` and a host sends `bytes`.
//! - `figcraft:save` (to the host): `{ name, saveAs, bytes }` for every file the app hands its
//!   download service; in host mode nothing reaches the browser's downloads. `saveAs` says whether
//!   the file becomes the document's file ([`crate::hostmode::save_as`]):
//!   - A Save of the file the host opened, under its own name: `false`.
//!   - A document file (`.figcraft` or `.json`, the formats `file.save` and `file.saveAs` write)
//!     under a name the host didn't open — Save As, or a Save of a document the host never sent
//!     (an untitled one): `true`. It becomes the document's file; the next Save sends its name
//!     with `false`. Names compare without case.
//!   - Everything else is a copy or export that never becomes the document: `false` — rendered
//!     images, SVG and PDF, `.fig` copies, and the variables exports (a `.variables.json`
//!     sidecar's `.json` extension doesn't make it a document).
//! - Confirmed saves, opt-in: with `?save=managed` in the URL, or `data-save="managed"` on the
//!   canvas ([`crate::query::confirms_saves`]), each Save and Save As of the document carries an
//!   `id` (`save-1`, `save-2`, …) and the document keeps its unsaved changes until the host answers
//!   `figcraft:saved` with that `id` ([`crate::host_saves`]). Then it is saved as of the file the
//!   host stored, so edits made meanwhile stay unsaved, and a Save As becomes the document's file
//!   only then. Answers for any other `id` are ignored; a newer save replaces the one waiting; a
//!   file the host opens forgets it (the user is told). Without an answer within 30 seconds the save counts as not
//!   stored: the user is told and the host gets `figcraft:failed` with the `id`. Copies and exports
//!   carry no `id` and `saveAs: false`. A host that doesn't opt in sees no change: every file
//!   counts as saved once it is posted. `craft-web-host` has no message yet for a host to refuse a
//!   save, so a refusal shows as the timeout.
//! - `figcraft:dirty` whenever unsaved changes appear or go away, `figcraft:failed` when the app
//!   couldn't start.
//!
//! In host mode the app starts with no document (the home screen) and the host sends the file to
//! edit; `?author=` names the user for comments and board notes.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use craft_web_host::web::{FileData, Host, Message};
use figcraft_engine::cmd::file::FileSnapshot;
use figcraft_ui_egui::{Delivery, FigApp};

use crate::host_saves::HostSaves;

/// The app id in the host messages (`figcraft:ready`).
const APP: &str = "figcraft";

/// The longest user name taken from `?author=`.
const MAX_AUTHOR_CHARS: usize = 200;

/// What the host sent, in the order it came, for the shell's next frame ([`Bridge::frame`]).
enum FromHost {
    /// A file to open: `(name, bytes)`.
    Open(String, Vec<u8>),
    /// The save with this `id` is stored.
    Saved(String),
}

#[derive(Clone)]
pub struct Bridge {
    host: Rc<Host>,
    /// The host confirms saves (`?save=managed`): document saves wait for its `saved`.
    confirms_saves: bool,
    /// The document's file and the save waiting for the host ([`crate::host_saves`]).
    saves: Rc<RefCell<HostSaves<FileSnapshot>>>,
    /// Messages heard and not yet handled. The listener only queues them: a page host can answer
    /// while a post is still running, so nothing else is borrowed there.
    inbox: Rc<RefCell<VecDeque<FromHost>>>,
}

impl Bridge {
    /// The bridge for this page, or `None` when FigCraft isn't hosted: no `?host=parent` and no
    /// `data-host="page"` on the canvas, an unframed `?host=parent` page (it stays the normal app
    /// with downloads), or a parent on another origin. `confirms_saves`: the host opted in to
    /// confirmed saves.
    pub fn connect(canvas: &web_sys::HtmlCanvasElement, confirms_saves: bool) -> Option<Self> {
        Some(Self { host: Rc::new(Host::connect(APP, canvas)?), confirms_saves, saves: Rc::default(), inbox: Rc::default() })
    }

    /// Hear the host's messages. They are handled in the shell's next frame ([`Bridge::frame`]),
    /// in the order they came.
    pub fn listen(&self, ctx: egui::Context) {
        let inbox = self.inbox.clone();
        self.host.listen(move |message| {
            let heard = match message {
                Message::Open { name, data } => open_bytes(&name, &data),
                Message::Saved { id } => Some(FromHost::Saved(id)),
                _ => None,
            };
            if let Some(heard) = heard {
                inbox.borrow_mut().push_back(heard);
                ctx.request_repaint();
            }
        });
    }

    /// Every frame: handle what the host sent, in order (a file opens on the app's own open path,
    /// as a dropped one does; a `saved` marks its document saved), and end a save that waited too
    /// long. `now_ms`: the app's clock, which also wakes the app when the waiting save times out.
    pub fn frame(&self, fig: &mut FigApp, ctx: &egui::Context, now_ms: u64) {
        let heard = std::mem::take(&mut *self.inbox.borrow_mut());
        for message in heard {
            match message {
                FromHost::Open(name, bytes) => {
                    let forgotten = self.saves.borrow_mut().opened(&name);
                    if let Some(save) = forgotten {
                        fig.document_not_stored(&save.name, &figcraft_ui_egui::save_forgotten_on_open());
                    }
                    queue_open(fig, name, bytes);
                }
                FromHost::Saved(id) => {
                    let confirmed = self.saves.borrow_mut().saved(&id);
                    match confirmed {
                        Some(save) => fig.document_stored(&save.name, save.file),
                        None => log::info!("host confirmed save {id:?}, which isn't the one waiting; ignored"),
                    }
                }
            }
        }
        let expired = self.saves.borrow_mut().expire(now_ms);
        if let Some(save) = expired {
            let secs = self.saves.borrow().timeout_ms() / 1000;
            let _ = self.host.post_failed(&format!("save {} was not confirmed within {secs} seconds", save.id), Some(&save.id));
            fig.document_not_stored(&save.name, &figcraft_ui_egui::save_unconfirmed(usize::try_from(secs).unwrap_or(usize::MAX)));
        }
        let wait = self.saves.borrow().wait_ms(now_ms);
        if let Some(wait) = wait {
            ctx.request_repaint_after(std::time::Duration::from_millis(wait));
        }
    }

    /// `?author=`: the host's name for the user signs comments and board notes.
    pub fn set_author(app: &mut FigApp, author: &str) {
        let author: String = author.trim().chars().take(MAX_AUTHOR_CHARS).collect();
        if !author.is_empty() {
            app.session.prefs.user_name = author;
        }
    }

    pub fn post_ready(&self) {
        let _ = self.host.post_ready(env!("CARGO_PKG_VERSION"), false);
    }

    /// The app couldn't start (no WebGPU or WebGL2, for example): let the host say so.
    pub fn post_failed(&self, error: &str) {
        let _ = self.host.post_failed(error, None);
    }

    /// The document file the host opened or the last Save As made, if any.
    pub fn host_file(&self) -> Option<String> {
        self.saves.borrow().host_file().map(str::to_string)
    }

    /// Hand a file the app would otherwise download to the host, without an `id`; `false` when
    /// nothing was delivered. To a host that confirms saves only copies and exports come this way
    /// (the document's own file goes through [`post_document`](Self::post_document)), so they are
    /// `saveAs: false`; to one that doesn't, the `saveAs` rule is this module's documentation's.
    pub fn post_save(&self, path: &str, bytes: &[u8]) -> bool {
        let name = file_name(path);
        let save_as = !self.confirms_saves && self.saves.borrow().save_as(&name);
        let sent = self.host.post_save(&name, bytes, None, save_as).is_ok();
        // After a Save As hand-over the host has the new file: the next Save of it sends its name
        // with `saveAs: false`. A failed send changes nothing, so a retry is still a Save As.
        if sent && save_as {
            self.saves.borrow_mut().adopt(&name);
        }
        sent
    }

    /// Hand the host the document's own file (Save, Save As) as `file`. To a host that confirms
    /// saves it goes with an `id` and stays unsaved until the host answers ([`Delivery::Pending`]);
    /// otherwise as [`post_save`](Self::post_save).
    pub fn post_document(&self, path: &str, bytes: &[u8], file: FileSnapshot, now_ms: u64, ctx: &egui::Context) -> Delivery {
        if !self.confirms_saves {
            return if self.post_save(path, bytes) { Delivery::Saved } else { Delivery::Refused };
        }
        let name = file_name(path);
        // Recorded before the post: a page host can answer while it is still dispatching.
        let (started, timeout_ms) = {
            let mut saves = self.saves.borrow_mut();
            (saves.start(&name, file, now_ms), saves.timeout_ms())
        };
        match self.host.post_save(&name, bytes, Some(&started.id), started.save_as) {
            Ok(()) => {
                // Schedule the timeout wakeup immediately so an idle tab still times out if the
                // host never answers after this frame.
                ctx.request_repaint_after(std::time::Duration::from_millis(timeout_ms));
                Delivery::Pending
            }
            Err(e) => {
                log::error!("couldn't hand {name} to the host: {e}");
                self.saves.borrow_mut().failed(&started.id);
                Delivery::Refused
            }
        }
    }

    /// Report unsaved changes when they appear or go away, not every frame (the crate does not
    /// repeat a state it already reported).
    pub fn set_dirty(&self, dirty: bool) {
        self.host.post_dirty(dirty);
    }
}

/// The app's own open queue gets a file the host sent: the same "Opening as a new file" card as a
/// dropped file, and `file.open` with the contents once read.
fn queue_open(fig: &mut FigApp, name: String, bytes: Vec<u8>) {
    let read: figcraft_ui_egui::ReadFuture = Box::pin(std::future::ready(Ok(bytes)));
    fig.ui.pending_opens.push(figcraft_ui_egui::pending_open::PendingOpen {
        label: name.clone(),
        params: serde_json::json!({ "name": name }),
        command: "file.open",
        target: None,
        read: Some(read),
    });
}

/// The file an open message carries, copied out for the next frame, or nothing when the payload
/// isn't one (a `File` the app doesn't read, or over the open limit). Once handled it is the
/// document's file: a Save of it under this name is `saveAs: false`.
fn open_bytes(name: &str, data: &FileData) -> Option<FromHost> {
    let name = crate::hostmode::open_name(name);
    let FileData::Bytes(array) = data else {
        log::warn!("host file {name} came as a File; FigCraft reads bytes only; not opened");
        return None;
    };
    if figcraft_ui_egui::browser_can_read(&name, data.size()).is_err() {
        log::warn!("host file {name} is too large to open in the browser; not opened");
        return None;
    }
    Some(FromHost::Open(name, array.to_vec()))
}

/// The last component of `path`, or the whole of it when there is none.
fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string())
}
