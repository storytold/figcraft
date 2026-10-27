//! Host modes: a page that runs FigCraft (as an `<iframe>`, or in its own document) opens
//! documents in it and stores what it saves instead of the browser downloading. The same
//! messages as the other Crafting Apps' bridges.
//!
//! ## `?host=parent` — an `<iframe>` (such as a Nextcloud app)
//!
//! The framing page and FigCraft talk over `postMessage` between same-origin windows. Messages
//! are plain objects with a `type`:
//!
//! - `figcraft:ready` (to the parent): `{ version }`, sent once the app is running and
//!   listening; send a file now.
//! - `figcraft:open` (from the parent): `{ name, bytes }`, bytes an `ArrayBuffer` or
//!   `Uint8Array`. It opens exactly as a file picked with File › Open or dropped on the window
//!   would (the app's own open queue, with its "Opening as a new file" card). A payload over the
//!   browser's own open limit ([`figcraft_ui_egui::browser_can_read`]'s 512 MB) is refused before
//!   its bytes are copied, and anything malformed is ignored.
//! - `figcraft:save` (to the parent): `{ name, saveAs, bytes }` (a transferred `ArrayBuffer`) for
//!   every file the app hands its download service — in host mode nothing reaches the browser's
//!   downloads. The parent stores it and reports failures itself. `saveAs` says whether the file
//!   becomes the document's file ([`crate::hostmode::save_as`]):
//!   - A Save of the file the host opened, under its own name: `false`.
//!   - A document file (`.figcraft` or `.json`, the formats `file.save` and `file.saveAs` write)
//!     under a name the host didn't open — Save As, or a Save of a document the host never sent
//!     (an untitled one): `true`. It becomes the document's file; the next Save sends its name
//!     with `false`. Names compare without case.
//!   - Everything else is a copy or export that never becomes the document: `false` — rendered
//!     images, SVG and PDF, `.fig` copies, and the variables exports (a `.variables.json`
//!     sidecar's `.json` extension doesn't make it a document).
//! - `figcraft:dirty` (to the parent): `{ dirty }` whenever unsaved changes appear or go away.
//! - `figcraft:failed` (to the parent): `{ error }` when the app couldn't start.
//!
//! Only the parent window on the page's own origin is heard or answered.
//!
//! ## `data-host="page"` — the page's own document
//!
//! A page that loads the module into its own document (its own `<canvas id="figcraft_canvas"
//! data-host="page">`, no iframe) gets the same behaviour over DOM `CustomEvent`s on the canvas
//! instead of `postMessage`: the same four events outward (`ready`, `save`, `dirty`, `failed`,
//! with the message fields as the event's `detail`), and the page dispatches `figcraft:open` on
//! the canvas with `detail: { name, bytes }` to open a document. Events don't bubble, so two
//! editors on one page can't hear each other, and the canvas is the only target FigCraft
//! listens on.
//!
//! In host mode the app starts with no document (the home screen) and the host sends the file to
//! edit; `?author=` names the user for comments and board notes.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use figcraft_ui_egui::FigApp;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;

/// The longest user name taken from `?author=`.
const MAX_AUTHOR_CHARS: usize = 200;

/// Files the host sent, on their way to the app's own open queue ([`figcraft_ui_egui::pending_open`]):
/// `(name, bytes)`.
pub type OpenQueue = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// How a host page is reached. The modes behave the same; they differ only in how an event goes
/// out ([`Transport::send`]) and how an open request comes in ([`Bridge::listen`]).
#[derive(Clone)]
enum Transport {
    /// `?host=parent`: `postMessage` between same-origin windows.
    Parent { window: web_sys::Window, parent: web_sys::Window, origin: String },
    /// `data-host="page"`: DOM `CustomEvent`s on the editor's own canvas.
    Page { canvas: web_sys::HtmlCanvasElement },
}

impl Transport {
    /// Hand an event to the host. In parent mode this is a `postMessage` whose object carries a
    /// `type` field; in page mode a `CustomEvent` of that name on the canvas, whose `detail`
    /// holds the fields and which never bubbles (two editors on one page must not hear each
    /// other). `transfer` hands an `ArrayBuffer` to the parent instead of copying it (parent
    /// mode only; the page's document already shares the buffer's bytes).
    fn send(&self, event: &str, fields: &[(&str, JsValue)], transfer: Option<&js_sys::ArrayBuffer>) -> Result<(), String> {
        match self {
            Transport::Parent { parent, origin, .. } => {
                let mut message_fields = Vec::with_capacity(fields.len() + 1);
                message_fields.push(("type", JsValue::from_str(event)));
                message_fields.extend_from_slice(fields);
                let message = message(&message_fields).ok_or_else(|| "couldn't build the message".to_string())?;
                let sent = match transfer {
                    Some(buffer) => parent.post_message_with_transfer(&message, origin, &js_sys::Array::of1(buffer)),
                    None => parent.post_message(&message, origin),
                };
                sent.map_err(|e| js_error(&e, "couldn't post the message"))
            }
            Transport::Page { canvas } => {
                let detail = message(fields).ok_or_else(|| "couldn't build the event detail".to_string())?;
                let init = web_sys::CustomEventInit::new();
                // The builder methods (`bubbles(..)`, `detail(..)`) are deprecated in web-sys
                // 0.3.106 and fail clippy -D warnings; use the setters.
                init.set_bubbles(false);
                init.set_detail(&detail);
                let custom = web_sys::CustomEvent::new_with_event_init_dict(event, &init).map_err(|e| js_error(&e, "couldn't create the event"))?;
                // A listener calling `preventDefault` would be the bool; not a send failure.
                canvas.dispatch_event(&custom).map_err(|e| js_error(&e, "couldn't dispatch the event"))?;
                Ok(())
            }
        }
    }
}

#[derive(Clone)]
pub struct Bridge {
    transport: Transport,
    /// The file the host opened, and after a `saveAs` hand-over the document's file: the name a
    /// Save under (`saveAs: false`) is decided against.
    host_file: Rc<RefCell<Option<String>>>,
    /// The unsaved-changes state last reported to the host.
    reported: Rc<Cell<Option<bool>>>,
}

impl Bridge {
    /// The parent bridge for `?host=parent`. `None` when the page isn't framed (there is no
    /// parent to talk to), so opening that URL directly stays the normal app with downloads, or
    /// when the parent is on another origin: messages only go to this origin, so the browser
    /// would drop every save without an error while the document showed as saved.
    pub fn new() -> Option<Self> {
        let window = web_sys::window()?;
        let parent = window.parent().ok().flatten()?;
        if js_sys::Object::is(&parent, &window) {
            return None;
        }
        let origin = window.location().origin().ok()?;
        // An opaque origin (a `file://` page in some browsers) can't be a `postMessage` target.
        if origin == "null" {
            return None;
        }
        // Reading another origin's location throws, which is `None` too.
        if parent.location().origin().ok()? != origin {
            return None;
        }
        Some(Self { transport: Transport::Parent { window, parent, origin }, host_file: Rc::default(), reported: Rc::default() })
    }

    /// The page bridge for `data-host="page"`: the host page runs FigCraft in its own document
    /// and hears the editor's DOM `CustomEvent`s on this canvas.
    pub fn for_page(canvas: &web_sys::HtmlCanvasElement) -> Self {
        Self { transport: Transport::Page { canvas: canvas.clone() }, host_file: Rc::default(), reported: Rc::default() }
    }

    /// Hear the files the host sends, and queue each one on `opens` for the app's own open path
    /// (the shell drains it into [`figcraft_ui_egui::pending_open`] every frame, so a host file
    /// opens like a dropped one).
    pub fn listen(&self, opens: OpenQueue, ctx: egui::Context) {
        let host_file = self.host_file.clone();
        match &self.transport {
            Transport::Parent { window, parent, origin } => {
                let parent = parent.clone();
                let origin = origin.clone();
                let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
                    if event.origin() != origin || !event.source().is_some_and(|s| js_sys::Object::is(&s, &parent)) {
                        return;
                    }
                    let data = event.data();
                    if string(&data, "type").as_deref() != Some("figcraft:open") {
                        return;
                    }
                    deliver_open(&opens, &ctx, &host_file, &data);
                });
                if window.add_event_listener_with_callback("message", on_message.as_ref().unchecked_ref()).is_ok() {
                    // The listener lives as long as the page.
                    on_message.forget();
                }
            }
            Transport::Page { canvas } => {
                let on_open = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
                    // A plain `Event` of the same name carries no detail; only a `CustomEvent`.
                    let Some(open) = event.dyn_ref::<web_sys::CustomEvent>() else { return };
                    deliver_open(&opens, &ctx, &host_file, &open.detail());
                });
                if canvas.add_event_listener_with_callback("figcraft:open", on_open.as_ref().unchecked_ref()).is_ok() {
                    // The listener lives as long as the page.
                    on_open.forget();
                }
            }
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
        let _ = self.transport.send("figcraft:ready", &[("version", env!("CARGO_PKG_VERSION").into())], None);
    }

    /// The app couldn't start (no WebGPU or WebGL2, for example): let the host say so.
    pub fn post_failed(&self, error: &str) {
        let _ = self.transport.send("figcraft:failed", &[("error", error.into())], None);
    }

    /// Hand a file the app would otherwise download to the host; `false` when nothing was
    /// delivered. The `saveAs` rule is this module's documentation's.
    pub fn post_save(&self, path: &str, bytes: &[u8]) -> bool {
        let name = file_name(path);
        let host_file = self.host_file.borrow().clone();
        let save_as = crate::hostmode::save_as(&name, host_file.as_deref());
        // `figcraft:save` carries the bytes as an `ArrayBuffer`; the parent bridge transfers it.
        let buffer = js_sys::Uint8Array::from(bytes).buffer();
        let fields: [(&str, JsValue); 3] = [("name", name.as_str().into()), ("saveAs", save_as.into()), ("bytes", buffer.clone().into())];
        let sent = self.transport.send("figcraft:save", &fields, Some(&buffer)).is_ok();
        // After a Save As hand-over the host has the new file: the next Save of it sends its name
        // with `saveAs: false`. A failed send changes nothing, so a retry is still a Save As.
        if sent && save_as {
            *self.host_file.borrow_mut() = Some(name);
        }
        sent
    }

    /// Report unsaved changes when they appear or go away, not every frame.
    pub fn set_dirty(&self, dirty: bool) {
        if self.reported.get() != Some(dirty) && self.transport.send("figcraft:dirty", &[("dirty", dirty.into())], None).is_ok() {
            self.reported.set(Some(dirty));
        }
    }
}

/// The file an open message or event carries, queued for the app's own open path, or nothing when
/// the payload isn't one (no bytes, or over the open limit).
fn deliver_open(opens: &OpenQueue, ctx: &egui::Context, host_file: &RefCell<Option<String>>, data: &JsValue) {
    let name = crate::hostmode::open_name(string(data, "name").as_deref());
    let Some(bytes) = payload_bytes(data, &name) else { return };
    // The document's file is now the one the host sent: a Save of it under this name is
    // `saveAs: false`.
    *host_file.borrow_mut() = Some(name.clone());
    opens.borrow_mut().push((name, bytes));
    ctx.request_repaint();
}

/// The `bytes` of an open message or event as owned bytes, refused over the browser's own open
/// limit ([`figcraft_ui_egui::browser_can_read`]) before they are copied. `bytes` is an
/// `ArrayBuffer` or `Uint8Array`.
fn payload_bytes(data: &JsValue, name: &str) -> Option<Vec<u8>> {
    let bytes = js_sys::Reflect::get(data, &JsValue::from_str("bytes")).ok()?;
    let len = if let Some(array) = bytes.dyn_ref::<js_sys::Uint8Array>() {
        array.length() as f64
    } else {
        bytes.dyn_ref::<js_sys::ArrayBuffer>()?.byte_length() as f64
    };
    if figcraft_ui_egui::browser_can_read(name, len).is_err() {
        log::warn!("host file {name} is too large to open in the browser; not opened");
        return None;
    }
    Some(if let Some(array) = bytes.dyn_ref::<js_sys::Uint8Array>() { array.to_vec() } else { js_sys::Uint8Array::new(&bytes).to_vec() })
}

/// The last component of `path`, or the whole of it when there is none.
fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string())
}

/// A message object or `CustomEvent` detail with the given fields, or `None` if one couldn't be
/// built (a field that isn't a valid property name, say).
fn message(fields: &[(&str, JsValue)]) -> Option<js_sys::Object> {
    let m = js_sys::Object::new();
    for (key, value) in fields {
        js_sys::Reflect::set(&m, &JsValue::from_str(key), value).ok()?;
    }
    Some(m)
}

fn string(data: &JsValue, key: &str) -> Option<String> {
    js_sys::Reflect::get(data, &JsValue::from_str(key)).ok()?.as_string()
}

/// A `JsValue` error as a string, after `what` went wrong.
fn js_error(e: &JsValue, what: &str) -> String {
    format!("{what}: {}", e.as_string().unwrap_or_else(|| format!("{e:?}")))
}
