//! The browser shell: web `Services` and the eframe web runner.

use figcraft_engine::Session;
use figcraft_ui_egui::{ContentsFuture, FigApp, Services};
use wasm_bindgen::JsCast as _;

const CANVAS_ID: &str = "figcraft_canvas";
const LOADING_ID: &str = "figcraft_loading";
/// What the page shows once it stopped on an internal error (styled in `index.html`).
const STOPPED_ID: &str = "figcraft_stopped";
/// The local storage key your name is kept under ([`crate::kept`]).
const NAME_KEY: &str = "figcraft.userName";
/// The local storage key the recovery copies are kept under ([`crate::recovery`]): a JSON list of
/// base64 `.figcraft` files.
const RECOVERY_KEY: &str = "figcraft.recovery";

thread_local! {
    /// The files with unsaved changes as of the last frame, for the panic hook ([`crate::recovery`]).
    static UNSAVED: std::cell::RefCell<crate::recovery::Unsaved> = std::cell::RefCell::default();
}

pub fn start() {
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    // Before the runner's own hook, which logs the panic and then calls this one.
    keep_unsaved_on_panic();
    wasm_bindgen_futures::spawn_local(async {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            log::error!("no document");
            return;
        };
        let Some(canvas) = document.get_element_by_id(CANVAS_ID).and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok()) else {
            log::error!("missing <canvas id=\"{CANVAS_ID}\">");
            return;
        };
        let mut options = eframe::WebOptions::default();
        // `?webgl`: force the WebGL2 backend (troubleshooting, and headless browsers whose WebGPU
        // adapter request returns nothing).
        if query().contains("webgl")
            && let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup
        {
            create.instance_descriptor.backends = eframe::wgpu::Backends::GL;
        }
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                options,
                Box::new(|cc| {
                    if let Some(rs) = &cc.wgpu_render_state {
                        log::info!("figcraft-web: wgpu backend {:?}", rs.adapter.get_info().backend);
                    }
                    let reduce_motion = web_sys::window().and_then(|w| w.match_media("(prefers-reduced-motion: reduce)").ok().flatten());
                    let mut session = Session::new();
                    crate::kept::restore_name(&mut session, local_storage().and_then(|s| s.get_item(NAME_KEY).ok().flatten()));
                    let recovered = restore_unsaved(&mut session);
                    // Live sessions: the browser joins sessions and publishes files to servers;
                    // whatever arrives on the socket wakes the page for a frame.
                    let ctx = cc.egui_ctx.clone();
                    session.live.install(Box::new(figcraft_collab_net::NetLink::new(std::sync::Arc::new(move || ctx.request_repaint()))));
                    let mut fig = FigApp::new(session, Box::new(WebServices { reduce_motion, ctx: cc.egui_ctx.clone() }));
                    fig.note_recovered(recovered);
                    // `#join=<link>`: join that session, as opening an invite link does on the desktop.
                    let hash = web_sys::window().and_then(|w| w.location().hash().ok()).unwrap_or_default();
                    if let Some(link) = crate::query::join_link(&hash) {
                        let link = js_sys::decode_uri_component(link).ok().and_then(|s| s.as_string()).unwrap_or_else(|| link.to_string());
                        figcraft_ui_egui::live::open_invite(&mut fig, &link);
                    }
                    // `?control`: `window.figcraft`, the page's control channel (crate::api), opt-in.
                    if crate::query::control(&query()) {
                        crate::api::connect(cc.egui_ctx.clone());
                        if let Err(e) = crate::api::install() {
                            log::error!("couldn't install window.figcraft: {e:?}");
                        }
                    }
                    Ok(Box::new(WebShell { fig }))
                }),
            )
            .await;
        if let Some(el) = document.get_element_by_id(LOADING_ID) {
            match result {
                Ok(()) => el.remove(),
                Err(e) => el.set_inner_html(&format!("<p>FigCraft failed to start: {e:?}</p><p>A browser with WebGPU or WebGL2 is required.</p>")),
            }
        }
    });
}

/// The browser can't catch a panic (wasm32 aborts), so the page ends with it: keep the files with
/// unsaved changes in the page's storage for the next page, and say so on the page.
fn keep_unsaved_on_panic() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        // Callbacks that run after the page stopped can panic again: the files are kept once.
        ONCE.call_once(|| {
            let (unsaved, files) = UNSAVED.with(|u| u.try_borrow().map(|u| (u.count(), u.files())).unwrap_or_default());
            let kept = local_storage().map_or(0, |s| crate::recovery::store(files, |v| s.set_item(RECOVERY_KEY, v).is_ok()));
            show_stopped(&figcraft_ui_egui::browser_stopped(unsaved, kept));
        });
    }));
}

/// Open the files the last page kept when it stopped; how many came back.
fn restore_unsaved(session: &mut Session) -> usize {
    let Some(storage) = local_storage() else { return 0 };
    let kept = storage.get_item(RECOVERY_KEY).ok().flatten();
    // Gone before they open: a copy that stops the page again must not come back on every reload
    // (once a frame ran, the files it opened are followed as any unsaved file).
    let _ = storage.remove_item(RECOVERY_KEY);
    crate::recovery::restore(session, kept)
}

/// Say on the page that it stopped (the canvas stays as it was).
fn show_stopped(text: &str) {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else { return };
    let (Ok(el), Some(body)) = (document.create_element("div"), document.body()) else { return };
    el.set_id(STOPPED_ID);
    el.set_text_content(Some(text));
    let _ = body.append_child(&el);
}

/// The page's local storage (`None` where the browser keeps none, e.g. storage turned off).
fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

fn query() -> String {
    web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default()
}

struct WebShell {
    fig: FigApp,
}

impl eframe::App for WebShell {
    // eframe runs `logic` for visible tabs and, with only `run_logic`, for hidden ones: keep
    // autosave/live/control deadlines moving without a paint. `FigApp::ui` does not repeat it.
    /// Run the browser shell's eframe logic callback: guard `background_tick` so hidden tabs
    /// still advance autosave, live and control deadlines without painting. `FigApp::ui` does
    /// not repeat it.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Every frame, hidden or not, starts here: what the last one left is what a panic keeps.
        UNSAVED.with(|u| u.try_borrow_mut().map(|mut u| u.follow(&self.fig.session))).ok();
        figcraft_ui_egui::windows::guard(&mut self.fig, ctx, |fig| fig.background_tick(ctx));
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        // Pointer and key events sent over the control channel (`ui.input`), for the frame about
        // to draw: the batch is applied by this frame and acknowledged once it is presented, as on
        // the desktop (`apps/figcraft/src/main.rs`). Without this hook the batch sat in its queue
        // and its promise never settled.
        let events = figcraft_ui_egui::windows::take_input(&mut self.fig, raw_input.viewport_id);
        raw_input.events.extend(events);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.fig.control_queue.extend(crate::api::take_requests());
        self.fig.ui(ui);
        crate::api::poll_replies();
        // The fonts on this computer that text asked for this frame are read; once one is in,
        // auto-sizing text measured in a fallback fits it.
        crate::local_fonts::load_wanted(ui.ctx());
        if crate::local_fonts::take_loaded() {
            self.fig.run("text.refit", serde_json::json!({}));
        }
    }
}

/// Browser services. Files are opened and imported through the browser's file picker, which hands
/// over their contents (pickers that need a path return nothing, the trait's defaults); files are
/// saved as downloads. Copies go to the system clipboard through egui's output; pastes arrive as
/// paste events with their text (a page can't read the clipboard when it likes).
struct WebServices {
    /// `prefers-reduced-motion: reduce` (it follows the setting while the page is open).
    reduce_motion: Option<web_sys::MediaQueryList>,
    ctx: egui::Context,
}

impl Services for WebServices {
    /// An `<input type="file">` taking `exts`, clicked; its file is read once picked.
    fn pick_open_contents(&mut self, _title: &str, exts: &[&str]) -> Option<ContentsFuture> {
        let accept = exts.iter().map(|e| format!(".{e}")).collect::<Vec<_>>().join(",");
        Some(Box::pin(async move {
            pick_file(&accept).await.unwrap_or_else(|e| {
                log::error!("couldn't open a file: {e:?}");
                None
            })
        }))
    }
    fn downloads(&self) -> bool {
        true
    }
    /// A Blob behind a temporary `<a download>` link, clicked and removed.
    fn download(&mut self, name: &str, bytes: &[u8]) -> bool {
        if crate::api::capture(name, bytes) {
            return true;
        }
        match download(name, bytes) {
            Ok(()) => true,
            Err(e) => {
                log::error!("couldn't download {name}: {e:?}");
                false
            }
        }
    }
    fn clipboard_set_text(&mut self, s: &str) {
        figcraft_ui_egui::egui_copy_text(&self.ctx, s);
    }
    fn clipboard_set_png(&mut self, w: u32, h: u32, rgba: &[u8]) {
        figcraft_ui_egui::egui_copy_png(&self.ctx, w, h, rgba);
    }
    fn open_url(&mut self, url: &str) {
        let Some(window) = web_sys::window() else { return };
        if let Err(e) = window.open_with_url_and_target_and_features(url, "_blank", "noopener,noreferrer") {
            log::error!("couldn't open {url}: {e:?}");
        }
    }
    fn reduce_motion(&self) -> bool {
        self.reduce_motion.as_ref().is_some_and(web_sys::MediaQueryList::matches)
    }
    /// Only your name is kept ([`crate::kept`]): the rest starts afresh with each page.
    fn save_prefs(&mut self, prefs: &figcraft_engine::Prefs) {
        if let Some(Err(e)) = local_storage().map(|s| s.set_item(NAME_KEY, &prefs.user_name)) {
            log::warn!("couldn't keep your name: {e:?}");
        }
    }
    /// The browser's language (`navigator.language`).
    fn system_language(&self) -> Option<String> {
        web_sys::window().and_then(|w| w.navigator().language())
    }
    /// The Gamepad API's controllers with the standard mapping (its button order is the layout's;
    /// others are skipped). Browsers list controllers once one of their buttons is pressed.
    fn gamepad_held(&mut self) -> u32 {
        let Some(pads) = web_sys::window().and_then(|w| w.navigator().get_gamepads().ok()) else { return 0 };
        let mut held = 0;
        for pad in pads.iter().filter_map(|p| p.dyn_into::<web_sys::Gamepad>().ok()) {
            if pad.mapping() != web_sys::GamepadMappingType::Standard {
                continue;
            }
            for (i, b) in pad.buttons().iter().take(32).enumerate() {
                if b.dyn_into::<web_sys::GamepadButton>().is_ok_and(|b| b.pressed()) {
                    held |= 1 << i;
                }
            }
        }
        held
    }
    /// Local Font Access (`window.queryLocalFonts`, Chromium), not asked yet.
    fn can_ask_for_local_fonts(&self) -> bool {
        crate::local_fonts::can_ask()
    }
    fn ask_for_local_fonts(&mut self) {
        crate::local_fonts::ask(self.ctx.clone());
    }
}

/// The file picked in the browser's file picker (`accept`: the extensions it offers), with its
/// name; `None` when the picker is cancelled, `Some(Err)` with what to tell the user when the file
/// can't be read.
async fn pick_file(accept: &str) -> Result<Option<Result<(String, Vec<u8>), String>>, wasm_bindgen::JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let body = document.body().ok_or("no body")?;
    let input: web_sys::HtmlInputElement = document.create_element("input")?.dyn_into()?;
    input.set_type("file");
    input.set_accept(accept);
    input.set_hidden(true);
    // Some browsers (older Safari) open the picker only for an input in the page.
    body.append_child(&input)?;
    let (closed, picker) = PickerClosed::new(&window, input.clone());
    input.click();
    let result = wasm_bindgen_futures::JsFuture::from(closed).await;
    let file = input.files().and_then(|files| files.get(0));
    drop(picker);
    result?;
    let Some(file) = file else { return Ok(None) };
    let name = file.name();
    if let Err(why) = figcraft_ui_egui::browser_can_read(&name, file.size()) {
        return Ok(Some(Err(why)));
    }
    Ok(Some(match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
        Ok(buffer) => Ok((name, js_sys::Uint8Array::new(&buffer).to_vec())),
        Err(e) => {
            log::error!("couldn't read {name}: {e:?}");
            Err(figcraft_ui_egui::unreadable(&name))
        }
    }))
}

/// How long after the page gets focus back the file picker counts as closed (milliseconds): a
/// picked file's `change` can come just after the focus, and must win. A `change` later than this
/// (a slow cloud-backed file in a browser that never sends `cancel`) counts as a cancel.
const PICKER_FOCUS_SETTLE_MS: i32 = 1_000;

/// Settles a file picker's promise once: on `change` (a file was picked), on `cancel` (closed
/// without one) or shortly after the page gets focus back, for browsers that never send `cancel`
/// (Safari before 16.4), where the pick would otherwise stay pending and block every later one.
/// Dropping it (also when the pick is abandoned) removes the input, the focus listener and its timer.
struct PickerClosed {
    window: web_sys::Window,
    input: web_sys::HtmlInputElement,
    _settle: wasm_bindgen::closure::Closure<dyn FnMut()>,
    on_focus: wasm_bindgen::closure::Closure<dyn FnMut()>,
    timer: std::rc::Rc<std::cell::Cell<Option<i32>>>,
}

impl PickerClosed {
    fn new(window: &web_sys::Window, input: web_sys::HtmlInputElement) -> (js_sys::Promise, Self) {
        let resolve = std::rc::Rc::new(std::cell::RefCell::new(None::<js_sys::Function>));
        let slot = resolve.clone();
        let closed = js_sys::Promise::new(&mut |res, _rej| *slot.borrow_mut() = Some(res));
        let settle = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
            if let Some(res) = resolve.borrow_mut().take() {
                let _ = res.call0(&wasm_bindgen::JsValue::NULL);
            }
        });
        let _ = input.add_event_listener_with_callback("change", settle.as_ref().unchecked_ref());
        let _ = input.add_event_listener_with_callback("cancel", settle.as_ref().unchecked_ref());
        let timer = std::rc::Rc::new(std::cell::Cell::new(None));
        let (w, t, f) = (window.clone(), timer.clone(), settle.as_ref().unchecked_ref::<js_sys::Function>().clone());
        let on_focus = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
            if t.get().is_none() {
                t.set(w.set_timeout_with_callback_and_timeout_and_arguments_0(&f, PICKER_FOCUS_SETTLE_MS).ok());
            }
        });
        let _ = window.add_event_listener_with_callback("focus", on_focus.as_ref().unchecked_ref());
        (closed, Self { window: window.clone(), input, _settle: settle, on_focus, timer })
    }
}

impl Drop for PickerClosed {
    fn drop(&mut self) {
        if let Some(t) = self.timer.take() {
            self.window.clear_timeout_with_handle(t);
        }
        let _ = self.window.remove_event_listener_with_callback("focus", self.on_focus.as_ref().unchecked_ref());
        self.input.remove();
    }
}

fn download(name: &str, bytes: &[u8]) -> Result<(), wasm_bindgen::JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let body = document.body().ok_or("no body")?;
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &{
        let o = web_sys::BlobPropertyBag::new();
        o.set_type("application/octet-stream");
        o
    })?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)?;
    let a: web_sys::HtmlAnchorElement = document.create_element("a")?.dyn_into()?;
    a.set_href(&url);
    a.set_download(name);
    body.append_child(&a)?;
    a.click();
    a.remove();
    // Revoked later, not at once: some browsers cancel a download whose URL goes before it starts.
    let revoke = wasm_bindgen::closure::Closure::once_into_js(move || {
        let _ = web_sys::Url::revoke_object_url(&url);
    });
    // Without the timer the URL just stays until the page closes.
    let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), REVOKE_DOWNLOAD_MS);
    Ok(())
}

/// How long a download's object URL is kept (milliseconds).
const REVOKE_DOWNLOAD_MS: i32 = 40_000;
