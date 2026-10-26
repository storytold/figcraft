//! The control channel's UI-thread side. The app crate accepts JSON lines on a localhost socket
//! and queues [`ControlRequest`]s. [`background`] runs the phase-safe part from every logic
//! callback; [`drain`] admits and answers the UI-only work inside a real frame so replies reflect
//! the drawn UI. Methods:
//!
//! - `engine.execute {command, params}` — run any command (no dialogs open)
//! - `engine.commands {}` — every command with its params doc
//! - `document.inspect {depth?, allPages?}` / `node.get {id, depth?}`
//! - `ui.inspect {}` — panel/tab/tool state, canvas rect, timings
//! - `ui.pointer {kind: down|move|up|click|double|hover|drag, x, y (page) | sx, sy (screen), to?, shift?, alt?, cmd?}`
//! - `ui.key {key, shift?, cmd?, alt?}` — a shortcut as if typed on the canvas
//! - `ui.type {text}` — type into the text being edited
//! - `ui.input {events: [{move|down|up: [x, y], button?} | {key, shift?, alt?, cmd?} | {text} | {leave}
//!   | {hold: {alt?, shift?, cmd?}} | {gamepad: "A" | "Gamepad A" | 0}]}` — raw pointer and key events
//!   for the next frame, as if from the window (panels, popovers); `hold` keeps modifier keys down
//!   for the frames that follow (all false: released); `gamepad` presses a controller button in the
//!   prototype playing (presenter or preview pane)
//! - `ui.menu.invoke {id}` — a menu item exactly like clicking it (may open a dialog)
//! - `ui.set {rightTab?, leftTab?, theme?, home?, homeSection?, homeSearch?, homeFolder?, homeProjectsKind?, homeProjectsFilter?, homeProjectsView?, homeRecentsView?, homeRecentsSort?, homeRecentsOrder?, palette?, focusRegion?, preview?, presentScroll?, …}`
//! - `ui.screenshot {path?}` — PNG of the window, written to `path` or returned as base64 `data`;
//!   a hidden window is asked to paint (`RequestPaintWhileHidden`) and the request expires once at
//!   its deadline (native; the shared web runner does not implement hidden paint yet)
//! - `ui.render {path?, scale?, ids?}` — PNG of the document (no UI chrome; works without a
//!   window), written to `path` or returned as base64 `data`
//! - `window.new {path?, index?}` → `{window}` / `window.list {}` / `window.focus {id}` — several
//!   windows (desktop)
//! - `app.quit {}`
//!
//! Any method takes `window` (an id from `window.list`) to run in that window, with its tabs and
//! panels; without it, requests go to the window last in front.

use std::sync::{Arc, Mutex};

use egui::Context;
use figcraft_engine::cmd::comment::{Filter, Sort};
use figcraft_engine::{PointerEvent, PointerKind};
use serde_json::{Value, json};

use crate::FigApp;

pub struct ControlReply {
    pub id: Value,
    pub result: Result<Value, String>,
}

/// Which side has finished a control request.
///
/// The transport and the UI race to settle one request: while it is `Open`, either may cancel it
/// (the transport's timeout) or commit it (the UI's final reply). Once settled, neither side may
/// change it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RequestState {
    #[default]
    Open,
    /// The transport stopped waiting: no file may be written and no reply is wanted.
    Cancelled,
    /// The UI wrote the file (if any) and queued the reply before releasing the gate.
    Replied,
}

/// The gate the transport's timeout and the UI's final reply share for one request.
///
/// Encoding happens outside it, so a slow PNG never blocks the timeout. The final file write and
/// reply enqueue happen under it, after rechecking cancellation and the deadline, so a timeout
/// cannot overtake a committed file and a cancellation cannot slip between the check and the
/// write. It is never held across socket I/O.
#[derive(Clone, Default)]
pub struct RequestGate {
    state: Arc<Mutex<RequestState>>,
}

impl RequestGate {
    /// Create a request gate in its default `Open` state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return a copy of the gate's current request state.
    pub fn state(&self) -> RequestState {
        *self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Whether the gate is `Cancelled`.
    pub fn is_cancelled(&self) -> bool {
        self.state() == RequestState::Cancelled
    }

    /// Whether the gate is `Replied`.
    pub fn is_replied(&self) -> bool {
        self.state() == RequestState::Replied
    }

    /// Cancel an open request, as the transport's timeout does. Returns the resulting state so a
    /// timeout that lost the race knows to consume the already-queued reply.
    pub fn cancel_if_open(&self) -> RequestState {
        let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if *state == RequestState::Open {
            *state = RequestState::Cancelled;
        }
        *state
    }

    /// Run the final commit (file write and reply enqueue) only while the request is still open,
    /// then mark it replied before releasing the gate. `None` means the transport cancelled it or
    /// it was already committed; the closure must not have run then.
    pub fn commit<T>(&self, finish: impl FnOnce() -> T) -> Option<T> {
        let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if *state != RequestState::Open {
            return None;
        }
        let value = finish();
        *state = RequestState::Replied;
        Some(value)
    }
}

pub struct ControlRequest {
    pub id: Value,
    pub method: String,
    pub params: Value,
    pub reply: std::sync::mpsc::Sender<ControlReply>,
    /// The transport stops waiting after its timeout; discard requests still queued then. The UI
    /// passes the same gate to its screenshot's final commit.
    pub state: RequestGate,
}

impl ControlRequest {
    /// Whether this request's gate is `Cancelled`.
    pub fn is_cancelled(&self) -> bool {
        self.state.is_cancelled()
    }
}

/// One screenshot's completion slot.
///
/// The renderer's one-shot callback only stores the raw image here and wakes the captured
/// viewport; it never encodes or writes a file, and it holds only a [`std::sync::Weak`] reference, so a late
/// capture after the request (or the app) is gone is a no-op. The live request owns the only
/// strong reference, together with the viewport `Context` clone used for the wakeup.
pub struct CaptureSlot {
    inner: Mutex<SlotInner>,
    ctx: Context,
    viewport: egui::ViewportId,
}

#[derive(Default)]
struct SlotInner {
    image: Option<Arc<egui::ColorImage>>,
    closed: bool,
    /// The one image was already handed to a frame (a second callback cannot store another).
    taken: bool,
}

impl CaptureSlot {
    /// Create an empty capture slot for `viewport`, holding a clone of the UI context.
    fn new(ctx: &Context, viewport: egui::ViewportId) -> Arc<Self> {
        Arc::new(Self { inner: Mutex::new(SlotInner::default()), ctx: ctx.clone(), viewport })
    }

    /// The one-shot callback for `ViewportCommand::Screenshot`.
    fn callback(slot: &Arc<Self>) -> egui::ScreenshotCallback {
        let weak = Arc::downgrade(slot);
        egui::ScreenshotCallback::new(move |image| {
            if let Some(slot) = weak.upgrade() {
                slot.complete(image);
            }
        })
    }

    /// Store the first delivered image only while the slot is open and untaken, ignoring later
    /// or closed deliveries, then wake the viewport after releasing the lock.
    pub(crate) fn complete(&self, image: Arc<egui::ColorImage>) {
        {
            let mut inner = self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if inner.closed || inner.image.is_some() || inner.taken {
                return;
            }
            inner.image = Some(image);
        }
        // The lock is released before waking: the wakeup may draw the frame that drains this slot.
        self.ctx.request_repaint_of(self.viewport);
    }

    /// The captured image, once: a second callback or drain finds the slot taken.
    fn take(&self) -> Option<Arc<egui::ColorImage>> {
        let mut inner = self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if inner.taken {
            return None;
        }
        let image = inner.image.take();
        inner.taken = image.is_some();
        image
    }

    /// Whether the renderer delivered the image. A hidden window's frame must be asked to paint
    /// again to drain it: the capture command's own paint was the one that produced it.
    fn has_result(&self) -> bool {
        self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner).image.is_some()
    }

    /// Forget a cancelled, timed-out or superseded request: a late callback finds it closed.
    fn close(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.closed = true;
        inner.image = None;
    }
}

/// A screenshot's capture-time geometry (the viewport surface and the scale it was admitted at).
/// Pixels are derived from these dimensions, never from a stale `ViewportInfo::inner_rect` or a
/// previous pass's viewport rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptureSpec {
    pub viewport: egui::ViewportId,
    /// Expected readback pixels.
    pub size_px: [usize; 2],
    pub pixels_per_point: f32,
    pub zoom: f32,
}

/// Slack allowed for PNG headers and chunk framing past the raw pixel bytes.
const SHOT_PNG_OVERHEAD: usize = 64 * 1024;

impl CaptureSpec {
    /// Return the checked pixel byte count (`width × height × 4`), or `None` on overflow.
    fn bytes(&self) -> Option<usize> {
        self.size_px[0].checked_mul(self.size_px[1])?.checked_mul(4)
    }

    /// The bytes this capture may hold at its peak.
    ///
    /// `shot_reply` first holds the readback image, the straight-RGBA copy handed to the encoder
    /// and the PNG being built (two raw buffers plus the PNG), then drops the copy and builds the
    /// base64 reply from the PNG (readback plus PNG plus four thirds of the PNG). The budget is the
    /// larger of those phases, never a sum of all four buffers at once. The reservation is a bound,
    /// not an estimate of the encoded size; `shot_reply` enforces the reply side of it and must
    /// drop the straight-RGBA copy before encoding base64.
    fn cost(&self) -> Option<usize> {
        let raw = self.bytes()?;
        let png = raw.checked_add(SHOT_PNG_OVERHEAD)?;
        let encode_phase = raw.checked_mul(2)?.checked_add(png)?;
        let reply = png.checked_div(3)?.checked_add(1)?.checked_mul(4)?;
        let reply_phase = raw.checked_add(png)?.checked_add(reply)?;
        Some(encode_phase.max(reply_phase))
    }
}

/// One admitted screenshot: its deadline, capture identity and completion slot.
pub struct ShotRequest {
    pub id: Value,
    /// Where to write the PNG; `None`: return it as data.
    pub path: Option<String>,
    pub reply: std::sync::mpsc::Sender<ControlReply>,
    /// The transport's timeout and this request's final write share this gate.
    pub state: RequestGate,
    /// Monotonic seconds ([`crate::FrameClock`]); covers admission, settling, retries and arrival.
    pub deadline: f64,
    pub viewport: egui::ViewportId,
    /// The window whose document generation the capture belongs to.
    pub window: u64,
    /// The document UID shown when the request was admitted.
    pub document: u64,
    /// That presentation's monotonic generation (the UID alone cannot tell A→B→A apart).
    pub presentation: u64,
    pub spec: Option<CaptureSpec>,
    /// The geometry epoch at issuance; a resize/DPI/zoom change rejects the pixels.
    pub epoch: u64,
    /// Completed, presented UI frames since admission (hidden logic ticks never count; a
    /// discarded sizing pass inside a frame does not count separately from that frame's final
    /// presented pass).
    pub settle: u8,
    /// The last completed-frame number counted into `settle`; `None` until the first observed
    /// pass anchors the count. A frame is counted only once it has completed, so its first pass
    /// is never treated as settled before the frame has presented a layout.
    pub settle_frame: Option<u64>,
    pub issued: bool,
    pub slot: Option<Arc<CaptureSlot>>,
    /// Image bytes reserved for this request (in flight or completed but undrained).
    pub reserved: usize,
    pub retries: u8,
}

impl ShotRequest {
    /// Close and drop the capture slot, if any, and zero the reserved byte count.
    fn release(&mut self) {
        if let Some(slot) = self.slot.take() {
            slot.close();
        }
        self.reserved = 0;
    }
}

/// Screenshot state owned by the app: pending requests and the geometry epoch they were admitted
/// against. Pending and completed-but-undrained images both count toward the budget.
#[derive(Default)]
pub struct CaptureState {
    pub(crate) shots: Vec<ShotRequest>,
    /// The last UI geometry of each viewport and the epoch it defines (a resize, DPI change or
    /// zoom change — even back — bumps it). Per viewport: one window's frames must not invalidate
    /// another's captures.
    pub(crate) geometry: std::collections::HashMap<egui::ViewportId, (CaptureSpec, u64)>,
    pub(crate) epoch: u64,
}

/// How long a screenshot waits for the window to deliver it (the control server gives up then too).
pub(crate) const SHOT_TIMEOUT: f64 = 60.0;

/// Completed real UI frames a capture waits for before it is issued, so fonts, layout and the
/// renderer have settled (hidden logic ticks do not count; a frame counts once, after presenting
/// its final pass).
const SHOT_SETTLE_PASSES: u8 = 3;

/// The suite's capture budget, counted over in-flight and completed-but-undrained images.
pub(crate) const MAX_SHOTS: usize = 4;
const MAX_SHOT_IMAGE_BYTES: usize = 128 * 1024 * 1024;
const MAX_SHOT_TOTAL_BYTES: usize = 256 * 1024 * 1024;
/// A PNG larger than this is refused before its base64 reply is built (a PNG cannot normally grow
/// past its raw pixels, so only pathological content reaches it).
const MAX_SHOT_REPLY_BYTES: usize = 192 * 1024 * 1024;

/// The capture geometry of the UI pass drawing now, if the surface size is known. The screenshot
/// is the whole viewport surface (a notch's area is still read back), and pixel bounds are rounded
/// up: a fractional DPI must not turn a valid surface into a one-pixel mismatch.
fn current_spec(ctx: &Context, viewport: egui::ViewportId) -> Option<CaptureSpec> {
    let surface = ctx.viewport_rect();
    let rect = if surface.width() >= 1.0 && surface.height() >= 1.0 { surface } else { ctx.content_rect() };
    let ppp = ctx.pixels_per_point();
    let zoom = ctx.zoom_factor();
    if !(rect.width().is_finite() && rect.height().is_finite() && ppp.is_finite() && zoom.is_finite()) {
        return None;
    }
    let w = (rect.width() * ppp).ceil();
    let h = (rect.height() * ppp).ceil();
    if w < 1.0 || h < 1.0 || w > 32768.0 || h > 32768.0 {
        return None;
    }
    Some(CaptureSpec { viewport, size_px: [w as usize, h as usize], pixels_per_point: ppp, zoom })
}

/// Send one reply carrying the request id, ignoring a disconnected receiver.
fn reply(r: &mut ShotRequest, result: Result<Value, String>) {
    let _ = r.reply.send(ControlReply { id: r.id.clone(), result });
}

/// Admit a screenshot request. Geometry is filled in on the next UI pass; until then the request
/// is only counted, so a hidden window's capture can wait for its forced paint without inventing
/// geometry from a stale pass.
fn admit(app: &mut FigApp, r: ControlRequest, window: u64, now: f64) {
    let canceled = r.is_cancelled();
    let ControlRequest { id, method: _, params, reply: tx, state } = r;
    if app.capture.shots.len() >= MAX_SHOTS {
        if !canceled {
            let _ = tx.send(ControlReply { id, result: Err(format!("too many screenshots are pending (at most {MAX_SHOTS})")) });
        }
        return;
    }
    let path = params.get("path").and_then(Value::as_str).map(str::to_string);
    let (document, presentation) = app.windows.presentation(&app.session, window);
    app.capture.shots.push(ShotRequest {
        id,
        path,
        reply: tx,
        state,
        deadline: now + SHOT_TIMEOUT,
        viewport: crate::windows::viewport(window),
        window,
        document,
        presentation,
        spec: None,
        epoch: app.capture.epoch,
        settle: 0,
        settle_frame: None,
        issued: false,
        slot: None,
        reserved: 0,
        retries: 0,
    });
}

/// Expire requests whose whole lifetime (admission, settling, retries, arrival) is over, exactly
/// once, and release their image reservation.
fn expire(app: &mut FigApp, ctx: &Context, now: f64) {
    let mut i = 0;
    while i < app.capture.shots.len() {
        let (deadline, cancelled, window) = {
            let r = &app.capture.shots[i];
            (r.deadline, r.state.is_cancelled(), r.window)
        };
        let closed = !app.windows.exists(window);
        if now >= deadline || cancelled || closed {
            let mut r = app.capture.shots.remove(i);
            r.release();
            if !cancelled {
                let why = if closed {
                    "the window closed before the screenshot was captured"
                } else {
                    "the window didn't deliver a screenshot (is it visible?)"
                };
                reply(&mut r, Err(why.into()));
            }
            ctx.request_repaint();
        } else {
            i += 1;
        }
    }
}

/// Whether the platform reports window `id` as visible; `None` means it has not said (a
/// minimised window can report no `visible` flag at all).
fn visibility(ctx: &Context, id: u64) -> Option<bool> {
    ctx.input(|i| {
        let info = i.raw.viewports.get(&crate::windows::viewport(id))?;
        // Some platforms report `visible: true` for a minimised window; `minimized` is the more
        // specific signal there. No information at all stays `None` (unknown), never visible.
        if info.minimized == Some(true) {
            return Some(false);
        }
        info.visible()
    })
}

/// Whether window `id` draws no frames now: minimised, or covered by other windows (on macOS also
/// on another Space).
fn hidden(ctx: &Context, id: u64) -> bool {
    visibility(ctx, id) == Some(false)
}

/// Send `RequestPaintWhileHidden` for work that needs a real UI pass (a queued capture or
/// synthetic input), and schedule a wakeup for the next capture deadline. Native eframe honours
/// this command; the shared web runner does not implement it yet, so a hidden browser tab remains
/// an explicit, recorded qualification gap (see `docs/egui-037-local-followups.md`).
fn request_paint(app: &FigApp, ctx: &Context) {
    for r in &app.capture.shots {
        if !app.windows.exists(r.window) {
            continue;
        }
        // A capture that is still settling needs the real UI pass its geometry comes from; one
        // whose pixels arrived while the window was hidden needs the pass that drains the slot.
        // The capture command's own paint was drained when it produced the image.
        if !r.issued || r.slot.as_ref().is_some_and(|s| s.has_result()) {
            ctx.send_viewport_cmd_to(r.viewport, egui::ViewportCommand::RequestPaintWhileHidden);
        }
    }
    for r in &app.control_queue {
        if r.method == "ui.input" {
            let target = r.params.get("window").and_then(Value::as_u64).unwrap_or(app.windows.focused);
            if hidden(ctx, target) {
                ctx.send_viewport_cmd_to(crate::windows::viewport(target), egui::ViewportCommand::RequestPaintWhileHidden);
            }
        }
    }
    // A batch queued for a frame that has not drawn it yet also keeps asking. During a child
    // window's pass `app.control_input` is that window's live pane; from the main window's logic
    // it is the root's, so the viewport drawing now is the one to wake.
    if !app.control_input.is_empty() {
        ctx.send_viewport_cmd_to(ctx.viewport_id(), egui::ViewportCommand::RequestPaintWhileHidden);
    }
    for viewport in app.windows.pending_input_viewports() {
        ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::RequestPaintWhileHidden);
    }
    // A batch already handed to a hidden window's raw input (a logic-only pass takes it, egui
    // saves it for the next pass) still needs the real pass that applies and acknowledges it.
    if let Some(viewport) = app.applied_frame {
        ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::RequestPaintWhileHidden);
    }
    if let Some(d) = app.capture.shots.iter().map(|r| r.deadline).reduce(f64::min) {
        crate::repaint_in(ctx, (d - app.clock.now()).clamp(0.0, SHOT_TIMEOUT));
    }
}

/// Work a logic-only callback can do safely: expire deadlines, admit or park screenshot requests,
/// and answer state-only control requests for hidden windows. Input/geometry are never read here.
pub fn background(app: &mut FigApp, ctx: &Context) {
    let now = app.clock.now();
    expire(app, ctx, now);
    for r in std::mem::take(&mut app.control_queue) {
        if r.is_cancelled() {
            continue;
        }
        let target = r.params.get("window").and_then(Value::as_u64).unwrap_or(app.windows.focused);
        if !app.windows.exists(target) {
            let _ = r.reply.send(ControlReply { id: r.id, result: Err(format!("no window {target}")) });
            continue;
        }
        let is_hidden = hidden(ctx, target);
        if r.method == "ui.screenshot" && visibility(ctx, target) != Some(true) {
            // Hidden, minimised, or with visibility the platform has not reported: admit it now
            // so the request gets its deadline and a forced paint. Waiting for a frame that may
            // never draw would leave it queued past the transport's own timeout with no reply.
            admit(app, r, target, now);
            continue;
        }
        if !is_hidden && target == app.windows.current {
            // The window's frame admits and issues it against real geometry.
            app.control_queue.push(r);
            continue;
        }
        if !is_hidden {
            // Another visible window draws only when asked.
            ctx.request_repaint_of(crate::windows::viewport(target));
            app.control_queue.push(r);
            continue;
        }
        if r.method == "ui.input" {
            // Keep one synthetic batch until a real UI pass consumes it.
            app.control_queue.push(r);
            continue;
        }
        if r.method == "window.new" {
            let error = format!("window {target} is minimised or hidden: `window.new` needs it on screen");
            let _ = r.reply.send(ControlReply { id: r.id, result: Err(error) });
            continue;
        }
        // Outside a frame, so outside the frame's guard: within the window's swap, a panic still
        // ends in recovery copies and leaves every window its own state.
        let res = crate::windows::as_window(app, target, |app| {
            let mut res = Err("internal error".to_string());
            // Every control-channel gesture is fenced from the user's system clipboard.
            let before = app.control_synthetic;
            app.control_synthetic = true;
            crate::windows::guard(app, ctx, |app| res = handle(app, ctx, &r.method, &r.params));
            app.control_synthetic = before;
            res
        });
        let _ = r.reply.send(ControlReply { id: r.id, result: res.unwrap_or_else(|| Err(format!("no window {target}"))) });
    }
    request_paint(app, ctx);
}

/// The UI phase's control work: drain completed captures, admit and settle queued screenshots
/// against this pass's geometry, and run state-only requests for the window drawing now.
pub fn drain(app: &mut FigApp, ctx: &Context) {
    let now = app.clock.now();
    let viewport = ctx.viewport_id();
    // This pass's geometry defines the epoch: a resize, DPI change or zoom change (back or not)
    // invalidates captures issued under the old one.
    let (spec, epoch) = pass_geometry(app, ctx, viewport);
    expire(app, ctx, now);

    // Drain completed captures first, so their budget is free before new requests are admitted.
    drain_completed(app, ctx, viewport, now, epoch);

    // Admit queued captures for the window drawing now; other windows keep waiting for their frame.
    admit_queued(app, ctx, viewport, now);

    // Fill in this pass's geometry, enforce the byte budget (a post-callback check alone would
    // not bound peak memory), and release anything over it before issuing.
    enforce_capture_budget(app, viewport, spec);

    // Settle on completed UI frames only, then issue once. (The screenshot command itself forces
    // the paint that delivers it.) The request's document and window were fixed at admission and
    // are never rewritten here: a tab switched during settling is an error, not a capture of the
    // new one.
    //
    // `cumulative_frame_nr` counts frames whose final (presented) pass has finished. A frame whose
    // first pass calls `request_discard` is therefore counted once, after it presents its layout,
    // not when its discarded sizing pass runs. Passes after the first (`current_pass_index > 0`)
    // belong to such a frame and never add another count.
    settle_and_issue(app, ctx, viewport, now, epoch, spec);
    request_paint(app, ctx);
}

/// Establish this pass's screenshot geometry and epoch: a resize, DPI change or zoom change (even
/// back) bumps the epoch captures issued under the old one were admitted against.
fn pass_geometry(app: &mut FigApp, ctx: &Context, viewport: egui::ViewportId) -> (Option<CaptureSpec>, u64) {
    let spec = current_spec(ctx, viewport);
    if let Some(spec) = spec {
        let changed = app.capture.geometry.get(&viewport).is_none_or(|(old, _)| *old != spec);
        if changed {
            app.capture.epoch = app.capture.epoch.wrapping_add(1);
            app.capture.geometry.insert(viewport, (spec, app.capture.epoch));
        }
    }
    let epoch = app.capture.geometry.get(&viewport).map_or(0, |(_, e)| *e);
    (spec, epoch)
}

/// Take the image delivery for the shot at `index`; `None` when no image has arrived yet.
fn arrived_shot(app: &mut FigApp, index: usize) -> Option<(ShotRequest, Arc<egui::ColorImage>)> {
    let slot = app.capture.shots.get(index)?.slot.clone()?;
    let image = slot.take()?;
    let mut r = app.capture.shots.remove(index);
    r.release();
    Some((r, image))
}

/// Drain images delivered for this viewport: reply, re-queue for a geometry retry, or drop a
/// cancelled capture. Each viewport drains its own captures: their geometry and document differ.
fn drain_completed(app: &mut FigApp, ctx: &Context, viewport: egui::ViewportId, now: f64, epoch: u64) {
    let mut i = 0;
    while i < app.capture.shots.len() {
        if app.capture.shots[i].viewport != viewport {
            i += 1;
            continue;
        }
        let Some((r, image)) = arrived_shot(app, i) else {
            i += 1;
            continue;
        };
        settle_arrival(app, ctx, viewport, now, epoch, r, &image);
        ctx.request_repaint();
    }
}

/// Whether the pass geometry differs from the one the capture was issued at (even back to an old
/// size): the pixels cannot be accepted for the admitted surface.
fn geometry_moved(r: &ShotRequest, current: CaptureSpec, epoch: u64) -> bool {
    r.epoch != epoch || r.spec.is_none_or(|s| s.size_px != current.size_px || s.pixels_per_point != current.pixels_per_point)
}

/// The admitted surface is in whole UI points; the readback is in physical pixels. Up to one
/// pixel of rounding either way is the same surface, not a scale change. The difference is
/// absolute: an undersized readback is a scale mismatch too.
fn scale_mismatch(image: &egui::ColorImage, spec: CaptureSpec) -> bool {
    image.size[0].abs_diff(spec.size_px[0]) > 1 || image.size[1].abs_diff(spec.size_px[1]) > 1
}

/// Re-queue a capture against the new pass, or hand it back when its retries are exhausted.
fn requeue_capture(app: &mut FigApp, mut r: ShotRequest, current: CaptureSpec, epoch: u64) -> Option<ShotRequest> {
    if r.retries >= 2 {
        return Some(r);
    }
    r.retries += 1;
    r.spec = Some(current);
    r.epoch = epoch;
    r.reserved = r.spec.and_then(|s| s.cost()).unwrap_or(0);
    r.settle = 0;
    r.settle_frame = None;
    r.issued = false;
    app.capture.shots.push(r);
    None
}

/// Settle an arrived image: reply with it, retry against the new pass on a geometry/scale change,
/// or drop it when the transport cancelled or the window/document moved on.
fn settle_arrival(app: &mut FigApp, ctx: &Context, viewport: egui::ViewportId, now: f64, epoch: u64, mut r: ShotRequest, image: &egui::ColorImage) {
    let cancelled = r.state.is_cancelled();
    let current = current_spec(ctx, viewport);
    let (document, presentation) = app.windows.presentation(&app.session, r.window);
    if cancelled {
        // The transport stopped waiting: no reply, no file.
        return;
    }
    if now >= r.deadline {
        reply(&mut r, Err("the window didn't deliver a screenshot in time (is it visible?)".into()));
        return;
    }
    if r.window != app.windows.current || r.document != document || r.presentation != presentation {
        reply(&mut r, Err("the document changed while the screenshot was being captured".into()));
        return;
    }
    let Some(current) = current else {
        // Fail closed: with no representable current geometry the pixels cannot be proven to
        // belong to the surface that was admitted, so they are never the capture's reply.
        reply(&mut r, Err("the window's current geometry is unavailable (is it minimised?)".into()));
        return;
    };
    if geometry_moved(&r, current, epoch) {
        // Geometry moved between issuance and arrival (even back): retry against the new pass,
        // never accept the old pixels. Document changes are errors above.
        if let Some(mut r) = requeue_capture(app, r, current, epoch) {
            reply(&mut r, Err("the window kept changing size while the screenshot was captured".into()));
        }
        return;
    }
    if let Some(spec) = r.spec
        && scale_mismatch(image, spec)
    {
        if let Some(mut r) = requeue_capture(app, r, current, epoch) {
            reply(
                &mut r,
                Err(format!(
                    "the screenshot is {}x{} but the window is {}x{} (a scale mismatch)",
                    image.size[0], image.size[1], spec.size_px[0], spec.size_px[1]
                )),
            );
        }
        return;
    }
    // Encode outside the shared gate: a slow PNG must not hold the transport's timeout. The
    // fresh clock read and final write happen under the gate, so a timeout that arrives while the
    // image is being encoded either wins (no file, no reply) or finds the committed reply.
    let encoded = encode_shot(image, r.path.as_deref());
    commit_shot(&r.state, &r.id, &r.reply, r.deadline, &app.clock, encoded);
}

/// Admit queued captures for the window drawing now; other windows keep waiting for their frame.
fn admit_queued(app: &mut FigApp, ctx: &Context, viewport: egui::ViewportId, now: f64) {
    for r in std::mem::take(&mut app.control_queue) {
        route_queued(app, ctx, viewport, now, r);
    }
}

/// Route one queued control request: retarget another window, admit a capture, or run a
/// state-only request for the window drawing now.
fn route_queued(app: &mut FigApp, ctx: &Context, viewport: egui::ViewportId, now: f64, r: ControlRequest) {
    if r.is_cancelled() {
        return;
    }
    let target = r.params.get("window").and_then(Value::as_u64).unwrap_or(app.windows.focused);
    if target != app.windows.current {
        if app.windows.exists(target) {
            ctx.request_repaint_of(crate::windows::viewport(target));
            app.control_queue.push(r);
        } else {
            let _ = r.reply.send(ControlReply { id: r.id, result: Err(format!("no window {target}")) });
        }
        return;
    }
    if r.method == "ui.screenshot" {
        admit(app, r, target, now);
        return;
    }
    // Every control-channel gesture is fenced from the user's system clipboard (the flag is
    // restored so a batch already in a frame's raw input keeps its own provenance).
    let before = app.control_synthetic;
    app.control_synthetic = true;
    let res = handle(app, ctx, &r.method, &r.params);
    app.control_synthetic = before;
    if r.method == "ui.input" && std::mem::take(&mut app.control_input_queued) {
        // Acknowledge once the pass that applies the batch is presented, never from a sizing
        // pass whose input is thrown away.
        app.input_acks.push(crate::PendingAck { viewport, tx: r.reply, reply: ControlReply { id: r.id, result: res } });
    } else {
        let _ = r.reply.send(ControlReply { id: r.id, result: res });
    }
    ctx.request_repaint();
}

/// Reserve this request's actual surface against the capture budget; `true` when it must be
/// removed for exceeding it.
fn reserve_for_spec(r: &mut ShotRequest, spec: Option<CaptureSpec>, total: usize) -> bool {
    if !r.issued && r.spec != spec {
        // A request can resize during its settling frames. Reserve and validate this
        // actual surface before issuance, never the old (possibly much smaller) one.
        r.spec = spec;
        r.settle = 0;
        r.settle_frame = None;
        if spec.is_none() {
            r.reserved = 0;
        }
    }
    let Some(spec) = r.spec else { return false };
    let raw = spec.bytes().unwrap_or(usize::MAX);
    let cost = spec.cost().unwrap_or(usize::MAX);
    if raw > MAX_SHOT_IMAGE_BYTES || total.saturating_sub(r.reserved).saturating_add(cost) > MAX_SHOT_TOTAL_BYTES {
        reply(r, Err(format!("the screenshot needs {cost} bytes, over the capture budget")));
        return true;
    }
    if r.reserved != cost {
        r.reserved = cost;
    }
    false
}

/// Fill in this pass's geometry and enforce the byte budget (a post-callback check alone would
/// not bound peak memory), releasing anything over it before issuing.
fn enforce_capture_budget(app: &mut FigApp, viewport: egui::ViewportId, spec: Option<CaptureSpec>) {
    let mut i = 0;
    while i < app.capture.shots.len() {
        let total: usize = app.capture.shots.iter().map(|s| s.reserved).sum();
        let remove = {
            let r = &mut app.capture.shots[i];
            if r.viewport != viewport {
                i += 1;
                continue;
            }
            reserve_for_spec(r, spec, total)
        };
        if remove {
            let mut r = app.capture.shots.remove(i);
            r.release();
        } else {
            i += 1;
        }
    }
}

/// Count one completed frame into the capture's settling, reporting whether it may now be issued.
fn settled_for_issue(r: &mut ShotRequest, frame_nr: u64) -> bool {
    match r.settle_frame {
        None => {
            // Anchor the count at the first pass that sees this request's geometry; this
            // frame has not completed yet.
            r.settle_frame = Some(frame_nr);
            false
        }
        Some(seen) if frame_nr > seen => {
            r.settle_frame = Some(frame_nr);
            r.settle = r.settle.saturating_add(1);
            r.settle >= SHOT_SETTLE_PASSES
        }
        _ => false,
    }
}

/// Settle on completed UI frames only, then issue once (against this pass's surface).
fn settle_and_issue(app: &mut FigApp, ctx: &Context, viewport: egui::ViewportId, now: f64, epoch: u64, spec: Option<CaptureSpec>) {
    let first_pass = ctx.current_pass_index() == 0;
    let frame_nr = ctx.cumulative_frame_nr_for(viewport);
    for r in &mut app.capture.shots {
        if !first_pass || r.viewport != viewport || r.issued || r.spec.is_none() {
            continue;
        }
        if r.state.is_cancelled() || now >= r.deadline {
            continue;
        }
        if !settled_for_issue(r, frame_nr) {
            continue;
        }
        let slot = CaptureSlot::new(ctx, viewport);
        r.slot = Some(slot.clone());
        r.epoch = epoch;
        // Issued against this pass's surface: the size the readback is expected at.
        if let Some(spec) = spec {
            r.spec = Some(spec);
        }
        r.issued = true;
        ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Screenshot(CaptureSlot::callback(&slot)));
    }
}

/// An encoded screenshot that has not been written or returned yet. Encoding is the expensive
/// part, so it is done before the request's shared commit gate is taken.
enum EncodedShot {
    /// The PNG for a `path` request; writing it is part of the final commit.
    File { path: String, png: Vec<u8>, width: usize, height: usize },
    /// The complete base64 reply for a request without a path.
    Data(Value),
}

impl EncodedShot {
    /// Deliver the encoded shot: for `File`, write the PNG through `file_access` and return its
    /// path and dimensions; for `Data`, return the prebuilt result.
    fn finish(self) -> Result<Value, String> {
        match self {
            Self::File { path, png, width, height } => {
                figcraft_engine::file_access::write(&path, &png).map_err(|e| e.to_string())?;
                Ok(json!({"path": path, "width": width, "height": height}))
            }
            Self::Data(result) => Ok(result),
        }
    }
}

/// A screenshot's reply: the PNG written to `path` (through the automation roots, when the app
/// has them), or returned as base64 `data`. Control requests split this into [`encode_shot`] and
/// [`commit_shot`]; encoding and automation-root tests use the whole operation.
#[cfg(test)]
fn shot_reply(img: &egui::ColorImage, path: Option<&str>) -> Result<Value, String> {
    encode_shot(img, path)?.finish()
}

/// Encode the straight-alpha PNG and the data reply. No request gate is held here.
fn encode_shot(img: &egui::ColorImage, path: Option<&str>) -> Result<EncodedShot, String> {
    let [w, h] = img.size;
    let raw = w.checked_mul(h).and_then(|n| n.checked_mul(4)).ok_or("bad screenshot size")?;
    if raw > MAX_SHOT_IMAGE_BYTES {
        return Err(format!("the screenshot is {raw} bytes, over the capture budget"));
    }
    let mut rgba = Vec::with_capacity(raw);
    for p in &img.pixels {
        rgba.extend_from_slice(&p.to_srgba_unmultiplied());
    }
    let buf = image::RgbaImage::from_raw(w as u32, h as u32, rgba).ok_or("bad screenshot buffer")?;
    let mut png = std::io::Cursor::new(Vec::new());
    buf.write_to(&mut png, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    // The encoder copy is not needed once the PNG exists. Drop it before the (possibly bigger)
    // base64 reply is built so the peak is the larger phase accounted by `CaptureSpec::cost`,
    // not the sum of all four buffers.
    drop(buf);
    let png = png.into_inner();
    match path {
        Some(path) => Ok(EncodedShot::File { path: path.to_string(), png, width: w, height: h }),
        None => {
            if png.len() > MAX_SHOT_REPLY_BYTES {
                // Refuse before the base64 string (4/3 of the PNG) is allocated.
                return Err(format!("the encoded screenshot is {} bytes, over the reply budget", png.len()));
            }
            use base64::Engine as _;
            Ok(EncodedShot::Data(json!({"format": "png", "width": w, "height": h, "data": base64::engine::general_purpose::STANDARD.encode(&png)})))
        }
    }
}

/// Commit an encoded screenshot's file and reply while holding the request's shared gate. The
/// cancellation and a fresh deadline are rechecked under the gate: if the transport gave up while
/// the image was being encoded, no file is written; a commit that gets there first queues its
/// reply before the timeout can mark the request cancelled.
fn commit_shot(
    gate: &RequestGate,
    id: &Value,
    reply: &std::sync::mpsc::Sender<ControlReply>,
    deadline: f64,
    clock: &crate::FrameClock,
    encoded: Result<EncodedShot, String>,
) -> bool {
    gate.commit(|| {
        // Read the clock after the gate is held: a request whose deadline passed while it waited
        // for the gate must not write its file.
        let result = if clock.now() < deadline {
            encoded.and_then(EncodedShot::finish)
        } else {
            Err("the window didn't deliver a screenshot in time (is it visible?)".into())
        };
        // Enqueue before the gate is released and before `Replied` is visible to the transport.
        let _ = reply.send(ControlReply { id: id.clone(), result });
    })
    .is_some()
}

/// A controller button given as its standard-layout index, its key trigger name ("Gamepad A") or
/// its short name ("A", "LB", "Up").
fn pad_name(v: &Value) -> Option<&'static str> {
    use figcraft_doc::proto::{GAMEPAD_BUTTONS, gamepad_button};
    if let Some(i) = v.as_u64() {
        return usize::try_from(i).ok().and_then(gamepad_button);
    }
    let s = v.as_str()?.trim();
    GAMEPAD_BUTTONS.iter().copied().find(|b| b.eq_ignore_ascii_case(s) || b.get(8..).is_some_and(|short| short.eq_ignore_ascii_case(s)))
}

fn b(p: &Value, k: &str) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(false)
}

/// Handle one control method (also used by tests and the headless snapshot tool).
pub fn handle(app: &mut FigApp, ctx: &Context, method: &str, p: &Value) -> Result<Value, String> {
    match method {
        "engine.execute" => {
            let cmd = p.get("command").and_then(Value::as_str).ok_or("`command` required")?;
            let params = p.get("params").cloned().unwrap_or(json!({}));
            let layers_generation = app.session.clipboard.layers_generation;
            let r = app.session.execute(cmd, &params).map_err(|e| e.to_string());
            app.clear_comment_draft_after_tool_set(cmd, r.is_ok());
            app.invalidate_synthetic_copy(layers_generation);
            // Programmatic calls never open dialogs.
            app.session.ui_requests.retain(|u| !u.kind.starts_with("dialog."));
            app.handle_ui_requests();
            // A programmatic `doc.switch` changes the presented file just like a tab click: the
            // capture generation must see it even though this path bypasses `FigApp::run`.
            app.observe_presentation();
            r
        }
        "engine.commands" => Ok(Value::Array(
            figcraft_engine::all_commands()
                .iter()
                .map(|c| json!({"id": c.id, "label": c.label, "params": c.params, "menu": c.menu(), "shortcut": c.shortcut(), "undoable": c.history}))
                .collect(),
        )),
        "document.inspect" => {
            let depth = p.get("depth").and_then(Value::as_u64).unwrap_or(8).min(64) as usize;
            Ok(figcraft_engine::inspect::document(&app.session, depth, b(p, "allPages")))
        }
        "node.get" => {
            let id = p.get("id").and_then(figcraft_engine::params::id_of).ok_or("`id` required")?;
            let depth = p.get("depth").and_then(Value::as_u64).unwrap_or(1) as usize;
            app.session.doc().map(|d| figcraft_engine::inspect::node_json(d, id, depth)).map_err(|e| e.to_string())
        }
        "window.new" => {
            let index = p.get("index").and_then(Value::as_u64).map(|i| i as usize);
            crate::windows::open(app, ctx, index, p.get("path").and_then(Value::as_str), None).map(|id| json!({"window": id}))
        }
        "window.list" => Ok(crate::windows::list(app)),
        "window.focus" => crate::windows::focus(app, ctx, p.get("id").and_then(Value::as_u64).ok_or("`id` required")?),
        "ui.inspect" => {
            let mut v = app.inspect();
            // The name of the control with the keyboard (what a screen reader announces).
            if let Some(f) = v.get_mut("focus") {
                f["control"] = json!(crate::a11y::focused_name(ctx));
            }
            Ok(v)
        }
        "ui.pointer" => pointer(app, p),
        "ui.key" => {
            let key = p.get("key").and_then(Value::as_str).ok_or("`key` required")?;
            let m = figcraft_engine::keymap::Mods { shift: b(p, "shift"), alt: b(p, "alt"), cmd: b(p, "cmd"), ctrl: b(p, "ctrl") };
            let platform = crate::shortcuts::platform();
            use figcraft_engine::keymap::{Context, lookup, normalize};
            // Text editing keys go to the editor (after the shortcuts live while editing).
            if app.session.ds().map(|d| d.text_edit.is_some()).unwrap_or(false) {
                if let Some(bd) = lookup(platform, Context::Text, key, m).filter(|_| m.cmd || m.alt || m.ctrl) {
                    crate::menus::invoke(app, bd.id);
                    return Ok(json!({"command": bd.id}));
                }
                let (word, line) = figcraft_engine::keymap::text_motion(platform, key, m);
                figcraft_engine::textedit::key(&mut app.session, key, m.shift, word, line).map_err(|e| e.to_string())?;
                return Ok(json!({"ok": true}));
            }
            // Vector edit mode and shape builder keys (tools, nudging points, delete, join, Enter).
            if app.session.tool_key(key, m.shift, m.alt, m.cmd).map_err(|e| e.to_string())? {
                return Ok(json!({"ok": true}));
            }
            // The Comment tool's arrows step through the comment threads.
            if app.session.tool.tool == figcraft_engine::ToolId::Comment && m.is_none() && matches!(key, "ArrowUp" | "ArrowDown" | "Up" | "Down") {
                let r = crate::comments::step(app, ctx, if matches!(key, "ArrowUp" | "Up") { -1 } else { 1 });
                return Ok(json!({"comment": r.and_then(|v| v.get("id").cloned())}));
            }
            if let Some(bd) = lookup(platform, Context::Canvas, key, m) {
                crate::shortcuts::run_binding(app, bd.id);
                return Ok(json!({"command": bd.id}));
            }
            let (name, _) = normalize(key).ok_or_else(|| format!("unknown key `{key}`"))?;
            match name {
                "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" => {
                    let (dx, dy) = match name {
                        "ArrowLeft" => (-1, 0),
                        "ArrowRight" => (1, 0),
                        "ArrowUp" => (0, -1),
                        _ => (0, 1),
                    };
                    let (cmd, params) = if m.cmd || m.ctrl {
                        ("transform.nudgeSize", json!({"dw": dx, "dh": dy, "big": m.shift}))
                    } else {
                        ("transform.nudge", json!({"dx": dx, "dy": dy, "big": m.shift}))
                    };
                    app.session.execute(cmd, &params).map_err(|e| e.to_string())
                }
                _ => {
                    // Number keys: opacity (two in quick succession make an exact percentage). A
                    // hidden window's logic tick has a stale `ctx.input(i.time)`.
                    let now = app.clock.now();
                    match egui::Key::from_name(name) {
                        Some(k) if m.is_none() && crate::shortcuts::opacity_digit(app, k, now) => Ok(json!({"opacity": true})),
                        _ => Err(format!("no shortcut `{key}`")),
                    }
                }
            }
        }
        "ui.input" => {
            let list = p
                .get("events")
                .and_then(Value::as_array)
                .ok_or("`events` required: [{move|down|up: [x, y]} | {key, shift?, alt?, cmd?} | {text} | {leave: true}]")?;
            let pos = |v: &Value| -> Option<egui::Pos2> {
                let a = v.as_array()?;
                let p = egui::pos2(a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32);
                p.is_finite().then_some(p)
            };
            let mut events = Vec::new();
            let mut pads = Vec::new();
            for e in list.iter().take(256) {
                let button = if e.get("button").and_then(Value::as_str) == Some("secondary") {
                    egui::PointerButton::Secondary
                } else {
                    egui::PointerButton::Primary
                };
                let command = b(e, "cmd") || b(e, "ctrl");
                let mods = egui::Modifiers { alt: b(e, "alt"), ctrl: command, shift: b(e, "shift"), mac_cmd: false, command };
                if let Some(p) = e.get("move").and_then(pos) {
                    events.push(egui::Event::PointerMoved(p));
                } else if let Some((p, pressed)) =
                    e.get("down").and_then(pos).map(|p| (p, true)).or_else(|| e.get("up").and_then(pos).map(|p| (p, false)))
                {
                    events.push(egui::Event::PointerMoved(p));
                    events.push(egui::Event::PointerButton { pos: p, button, pressed, modifiers: mods });
                } else if let Some(k) = e.get("key").and_then(Value::as_str) {
                    let key = egui::Key::from_name(k).ok_or_else(|| format!("unknown key `{k}`"))?;
                    for pressed in [true, false] {
                        events.push(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: mods });
                    }
                } else if let Some(t) = e.get("text").and_then(Value::as_str) {
                    events.push(egui::Event::Text(t.to_string()));
                } else if let Some(d) = e.get("scroll").and_then(pos) {
                    // Content moves by `d` points (negative y scrolls down, like a wheel turned towards you).
                    events.push(egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        source: egui::MouseWheelSource::Wheel,
                        delta: d.to_vec2(),
                        phase: egui::TouchPhase::Move,
                        modifiers: mods,
                    });
                } else if let Some(g) = e.get("gamepad") {
                    let names = figcraft_doc::proto::GAMEPAD_BUTTONS.join(", ");
                    pads.push(pad_name(g).ok_or_else(|| format!("unknown controller button {g} (an index 0-16 or one of {names})"))?.to_string());
                } else if b(e, "leave") {
                    events.push(egui::Event::PointerGone);
                } else if let Some(h) = e.get("hold") {
                    let held = egui::Modifiers {
                        alt: b(h, "alt"),
                        ctrl: b(h, "cmd") || b(h, "ctrl"),
                        shift: b(h, "shift"),
                        mac_cmd: false,
                        command: b(h, "cmd") || b(h, "ctrl"),
                    };
                    events.push(egui::Event::ModifiersChanged(held));
                } else {
                    return Err(format!("unknown input event {e}"));
                }
            }
            // Controller buttons go straight to the prototype playing (its next frame takes them).
            if !pads.is_empty() {
                match (app.ui.present.as_mut(), app.ui.preview.as_mut()) {
                    (Some(p), _) => p.pads.extend(pads),
                    (None, Some(pane)) => pane.player.pads.extend(pads),
                    (None, None) => return Err("controller buttons go to a prototype that is playing: run proto.present or proto.preview".into()),
                }
            }
            app.control_input_queued = app.push_control_input(events);
            ctx.request_repaint();
            Ok(json!({"queued": list.len().min(256)}))
        }
        "ui.type" => {
            let text = p.get("text").and_then(Value::as_str).ok_or("`text` required")?;
            figcraft_engine::textedit::insert(&mut app.session, text).map_err(|e| e.to_string())?;
            Ok(json!({"ok": true}))
        }
        "ui.menu.invoke" => {
            let id = p.get("id").and_then(Value::as_str).ok_or("`id` required")?;
            crate::menus::invoke(app, id);
            Ok(json!({"ok": true, "lastError": app.ui.last_error}))
        }
        "ui.set" => {
            if let Some(t) = p.get("rightTab").and_then(Value::as_str) {
                app.ui.right_tab = match t {
                    "prototype" => crate::RightTab::Prototype,
                    "inspect" => crate::RightTab::Inspect,
                    _ => crate::RightTab::Design,
                };
            }
            if let Some(t) = p.get("leftTab").and_then(Value::as_str) {
                app.ui.left_tab = if t == "assets" { crate::LeftTab::Assets } else { crate::LeftTab::File };
            }
            if let Some(t) = p.get("theme").and_then(Value::as_str) {
                app.session.prefs.theme = t.to_string();
            }
            if let Some(h) = p.get("home").and_then(Value::as_bool) {
                app.ui.home = h;
            }
            if let Some(section) = p.get("homeSection").and_then(Value::as_str).and_then(crate::home::Section::parse) {
                app.ui.home_view.section = section;
            }
            if let Some(query) = p.get("homeSearch").and_then(Value::as_str) {
                app.ui.home_view.file_query = query.chars().take(256).collect();
            }
            // A folder of the project library: its page shows.
            if let Some(id) = p.get("homeFolder").and_then(Value::as_str).filter(|id| app.session.projects.folder(id).is_some()) {
                crate::home_projects::open_folder(app, id);
            }
            if let Some(kind) = p.get("homeProjectsKind").and_then(Value::as_str).and_then(crate::home_projects::KindFilter::parse) {
                app.ui.home_view.projects.kind = kind;
            }
            if let Some(filter) = p.get("homeProjectsFilter").and_then(Value::as_str) {
                app.ui.home_view.projects.filter = filter.chars().take(256).collect();
            }
            if let Some(view) = p.get("homeProjectsView").and_then(Value::as_str).and_then(crate::home_recents::View::parse) {
                view.store_section(&mut app.session.prefs, "projects");
            }
            if let Some(view) = p.get("homeRecentsView").and_then(Value::as_str).and_then(crate::home_recents::View::parse) {
                view.store(&mut app.session.prefs);
            }
            if let Some(key) = p.get("homeRecentsSort").and_then(Value::as_str).and_then(crate::home_recents::SortKey::parse) {
                crate::home_recents::Sort::by(key).store(&mut app.session.prefs);
            }
            if let Some(order) = p.get("homeRecentsOrder").and_then(Value::as_str).filter(|o| matches!(*o, "asc" | "desc")) {
                let sort = crate::home_recents::Sort::of(&app.session.prefs);
                crate::home_recents::Sort { descending: order == "desc", ..sort }.store(&mut app.session.prefs);
            }
            if let Some(h) = p.get("palette").and_then(Value::as_bool) {
                crate::menus::set_palette(&mut app.ui, h);
            }
            // true / false, or a top-level menu to open it at ("Plugins").
            match p.get("mainMenu") {
                Some(Value::Bool(h)) => app.ui.main_menu = *h,
                Some(Value::String(top)) => {
                    app.ui.main_menu = true;
                    crate::menus::open_main_menu(ctx, top);
                }
                _ => {}
            }
            if let Some(h) = p.get("pluginsWindow").and_then(Value::as_bool) {
                app.ui.plugins_window = h;
            }
            // The Plugins window's tab ("installed", "browse"), and the plugin of the listed
            // directory whose install asks for confirmation (null: none).
            if let Some(tab) = p.get("pluginsTab").and_then(Value::as_str) {
                app.ui.plugins_manage.tab =
                    if tab == "browse" { crate::plugins_ui::PluginsTab::Browse } else { crate::plugins_ui::PluginsTab::Installed };
            }
            match p.get("pluginConfirm") {
                Some(Value::String(id)) => app.ui.plugins_manage.confirm = Some(id.clone()),
                Some(Value::Null) => app.ui.plugins_manage.confirm = None,
                _ => {}
            }
            // A plugin command's parameters form: {id, command?}; null closes it.
            match p.get("pluginForm") {
                Some(Value::Null) => app.ui.plugin_form = None,
                Some(v @ Value::Object(_)) => {
                    let id = v.get("id").and_then(Value::as_str).ok_or("pluginForm: {id, command?}")?;
                    let command = v.get("command").and_then(Value::as_str).unwrap_or_default();
                    crate::plugins_ui::start(app, id, command);
                }
                _ => {}
            }
            // Give a region the keyboard as F6 would: toolbar, layers, canvas or properties.
            if let Some(r) = p.get("focusRegion").and_then(Value::as_str) {
                app.ui.nav.go(crate::a11y::Region::parse(r).ok_or_else(|| format!("unknown region `{r}`"))?);
            }
            // A right-click on the canvas at screen [sx, sy] (null closes the menu);
            // `contextSelectLayer` opens its Select layer submenu.
            match p.get("contextMenu") {
                Some(Value::Array(at)) => {
                    let (x, y) = (at.first().and_then(Value::as_f64).unwrap_or(0.0), at.get(1).and_then(Value::as_f64).unwrap_or(0.0));
                    crate::canvas::open_context_menu(app, egui::pos2(x as f32, y as f32), b(p, "cmd"));
                }
                Some(Value::Null) => app.ui.context_menu = None,
                _ => {}
            }
            if let Some(v) = p.get("contextSelectLayer").and_then(Value::as_bool) {
                crate::menus::open_select_layer(ctx, v);
            }
            if let Some(h) = p.get("iconGallery").and_then(Value::as_bool) {
                app.ui.icon_gallery = h;
            }
            // Comment thread popover: an id opens it, null closes it.
            if let Some(v) = p.get("commentOpen") {
                match v.as_u64() {
                    Some(id) => {
                        // Pins are drawn for the current page only: reveal the thread's page first.
                        let page = app.session.doc().ok().and_then(|d| d.comment(id)).map(|c| c.page);
                        if let Some(page) = page.filter(|p| app.session.page().is_ok_and(|cur| cur != *p)) {
                            app.session.execute("page.select", &json!({"id": page.0})).map_err(|e| e.to_string())?;
                        }
                        app.ui.comments.show(id);
                    }
                    None => app.ui.comments.close(),
                }
            }
            if let Some(v) = p.get("commentDraft") {
                match (v.get("x").and_then(Value::as_f64), v.get("y").and_then(Value::as_f64)) {
                    (Some(x), Some(y)) => crate::comments::start_draft(app, x, y),
                    _ => app.ui.comments.draft = None,
                }
            }
            if let Some(v) = p.get("showResolvedComments").and_then(Value::as_bool) {
                app.ui.comments.filter = if v { Filter::All } else { Filter::Open };
            }
            if let Some(v) = p.get("commentFilter").and_then(Value::as_str) {
                app.ui.comments.filter = Filter::parse(v).ok_or("commentFilter: open|resolved|mine|unread|all")?;
            }
            if let Some(v) = p.get("commentSort").and_then(Value::as_str) {
                app.ui.comments.sort = Sort::parse(v).ok_or("commentSort: date|page")?;
            }
            if let Some(v) = p.get("commentSearch").and_then(Value::as_str) {
                app.ui.comments.search = v.to_string();
            }
            // The open thread's reply field, with the cursor at its end (an `@` there offers names).
            if let Some(v) = p.get("commentReply").and_then(Value::as_str) {
                app.ui.comments.reply = v.to_string();
                if let Some(id) = app.ui.comments.open {
                    crate::comments::focus_reply(ctx, id, v.chars().count());
                }
            }
            // Edit a message of the open thread: its index (0: the first message); null stops.
            if let Some(v) = p.get("commentEdit") {
                app.ui.comments.editing = match (app.ui.comments.open, v.as_u64()) {
                    (Some(id), Some(i)) => {
                        let text = app.session.doc().ok().and_then(|d| d.comment(id)).and_then(|c| c.message(i as usize)).map(|m| m.text.clone());
                        Some((id, i as usize, text.ok_or("commentEdit: no such message")?))
                    }
                    _ => None,
                };
            }
            if let Some(v) = p.get("presentComments").and_then(Value::as_bool) {
                app.ui.present_view.comments = v;
                if let Some(pres) = app.ui.present.as_mut() {
                    pres.view.comments = v;
                }
            }
            // Colour picker on the selection's paint `{fills?, index?}`; null closes it.
            if let Some(v) = p.get("colorPicker") {
                app.ui.picker = v.is_object().then(|| crate::colorpicker::PickerTarget::Paint {
                    fills: v.get("fills").and_then(Value::as_bool).unwrap_or(true),
                    index: v.get("index").and_then(Value::as_u64).unwrap_or(0).min(1000) as usize,
                    ids: app.session.selection(),
                });
            }
            // The Variables editor: true / a collection (id or name) / {library, collection?} (an
            // enabled library's, by id or name) opens it, false closes it.
            match p.get("variables") {
                Some(Value::Bool(open)) => app.ui.variables = open.then(Default::default),
                Some(Value::String(c)) => app.ui.variables = Some(crate::variables_panel::Editor::on(Some(c.clone()))),
                Some(v @ Value::Object(_)) => {
                    let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
                    app.ui.variables = Some(match text("library") {
                        Some(lib) => crate::variables_panel::Editor::in_library(lib, text("collection")),
                        None => crate::variables_panel::Editor::on(text("collection")),
                    });
                }
                _ => {}
            }
            // Layers panel search: a string opens it with that filter, null closes it.
            if let Some(v) = p.get("layerFilter") {
                app.ui.layer_search = v.is_string();
                app.ui.layer_filter = v.as_str().unwrap_or_default().chars().take(200).collect();
            }
            if p.get("collapseLayers").and_then(Value::as_bool) == Some(true) {
                app.ui.expanded.clear();
            }
            // The interaction editor popover in the Prototype panel: [layer id, index]; null closes it.
            match p.get("protoEditor") {
                Some(Value::Null) => crate::proto_panel::open_editor(ctx, None),
                Some(Value::Array(a)) => {
                    let id = a.first().and_then(Value::as_u64).ok_or("protoEditor: [layer id, interaction index]")?;
                    let k = a.get(1).and_then(Value::as_u64).unwrap_or(0) as usize;
                    crate::proto_panel::open_editor(ctx, Some((figcraft_doc::NodeId(id), k)));
                }
                _ => {}
            }
            // The prototype preview pane: true / a frame id opens it, false or null closes it.
            match p.get("preview") {
                Some(Value::Bool(false) | Value::Null) => app.ui.preview = None,
                Some(v @ (Value::Bool(true) | Value::Number(_))) => {
                    let params = if v.is_number() { json!({"start": v}) } else { json!({}) };
                    app.session.execute("proto.preview", &params).map_err(|e| e.to_string())?;
                    app.handle_ui_requests();
                }
                _ => {}
            }
            // Scroll the presented screen (the full-window player, else the preview pane) to [x, y].
            if let Some(s) = p.get("presentScroll").and_then(Value::as_array) {
                let at = |i: usize| s.get(i).and_then(Value::as_f64).unwrap_or(0.0);
                let player = match (app.ui.present.as_mut(), app.ui.preview.as_mut()) {
                    (Some(p), _) => Some(p),
                    (None, Some(pane)) => Some(&mut pane.player),
                    _ => None,
                };
                player.ok_or("nothing is presented")?.scroll_to(at(0), at(1));
            }
            // The style picker: a slot (fill, stroke, effect, grid, text) opens it for the selection, null
            // closes it; `stylePickerEdit` (a style id or name) opens its editor; `stylePickerView`
            // is `list` or `grid`.
            if let Some(v) = p.get("stylePicker") {
                match v.as_str().and_then(crate::style_picker::StyleSlot::parse) {
                    Some(slot) => crate::style_picker::open_for_selection(app, slot, p.get("stylePickerEdit").and_then(Value::as_str)),
                    None => app.ui.style_picker = None,
                }
            }
            if let Some(v) = p.get("stylePickerView").and_then(Value::as_str) {
                app.ui.style_grid = v == "grid";
            }
            // The Color profile dialog: a profile id opens it with that profile chosen, null closes it.
            if let Some(v) = p.get("colorProfileDialog") {
                app.ui.color_profile = v.as_str().and_then(figcraft_color::ColorProfile::parse);
            }
            if let Some(v) = p.get("exportPreview").and_then(Value::as_bool) {
                app.ui.export_thumb = v;
            }
            // Dev Mode's compare view: a frame or section opens it, null closes it; `devCompareVersion`
            // the saved version it compares with (null: when it was marked ready for dev),
            // `devCompareMode` is sideBySide or overlay, `devCompareSplit` the overlay slider (0–1).
            if let Some(v) = p.get("devCompare") {
                app.ui.dev.compare = figcraft_engine::params::id_of(v).map(|id| crate::dev_compare::CompareView::new(id, None));
            }
            if let Some(c) = app.ui.dev.compare.as_mut() {
                if let Some(v) = p.get("devCompareVersion") {
                    c.version = v.as_str().map(str::to_string);
                }
                match p.get("devCompareMode").and_then(Value::as_str) {
                    Some("overlay") => c.mode = crate::dev_compare::Mode::Overlay,
                    Some("sideBySide") => c.mode = crate::dev_compare::Mode::SideBySide,
                    Some(_) => return Err("devCompareMode: sideBySide|overlay".into()),
                    None => {}
                }
                if let Some(x) = p.get("devCompareSplit").and_then(Value::as_f64).filter(|x| x.is_finite()) {
                    c.split = x.clamp(0.0, 1.0) as f32;
                }
            }
            // The branch review dialog: a branch id opens it, null closes it; `branchReviewPicks`
            // chooses sides ({<conflict key>: "branch" | "main"}).
            if let Some(v) = p.get("branchReview") {
                match v.as_str() {
                    Some(id) => crate::branch_ui::open(app, Some(id)),
                    None => app.ui.branch_review = None,
                }
            }
            if let (Some(r), Some(picks)) = (app.ui.branch_review.as_mut(), p.get("branchReviewPicks").and_then(Value::as_object)) {
                for (k, v) in picks {
                    let side = v.as_str().and_then(figcraft_engine::branches::Side::parse).ok_or("branchReviewPicks: \"branch\" or \"main\"")?;
                    r.picks.insert(k.clone(), side);
                }
            }
            // Which annotations the canvas shows: all, none (uncategorised) or a category.
            if let Some(v) = p.get("annotationFilter").and_then(Value::as_str) {
                let doc = app.session.doc().map_err(|e| e.to_string())?;
                app.ui.dev.categories = crate::dev_mode::CategoryFilter::parse(doc, v).ok_or("annotationFilter: all|none|a category id or name")?;
            }
            if b(p, "resetFrameStats") {
                app.frame_stats.reset();
            }
            Ok(app.inspect())
        }
        "ui.render" => {
            let scale = p.get("scale").and_then(Value::as_f64).unwrap_or(1.0);
            let ids: Vec<figcraft_doc::NodeId> =
                p.get("ids").and_then(Value::as_array).map(|a| a.iter().filter_map(figcraft_engine::params::id_of).collect()).unwrap_or_default();
            let mut params = json!({"format": "png", "scale": scale});
            if let Some(path) = p.get("path").and_then(Value::as_str) {
                params["path"] = json!(path);
            }
            if !ids.is_empty() {
                params["ids"] = json!(ids);
            }
            app.session.execute("file.export", &params).map_err(|e| e.to_string())
        }
        "app.quit" => {
            app.ui.quit = true;
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
            Ok(json!({"ok": true}))
        }
        other => Err(format!("unknown method `{other}`")),
    }
}

fn pointer(app: &mut FigApp, p: &Value) -> Result<Value, String> {
    let zoom = app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0);
    let at = |x: Option<f64>, y: Option<f64>, sx: Option<f64>, sy: Option<f64>| -> Option<(f64, f64)> {
        match (x, y, sx, sy) {
            (Some(x), Some(y), _, _) => Some((x, y)),
            (_, _, Some(sx), Some(sy)) => {
                let pt = app.to_page(egui::pos2(sx as f32, sy as f32));
                Some((pt.x, pt.y))
            }
            _ => None,
        }
    };
    let f = |k: &str| p.get(k).and_then(Value::as_f64);
    let (x, y) = at(f("x"), f("y"), f("sx"), f("sy")).ok_or("pass x, y (page) or sx, sy (screen)")?;
    let pressure = |k: &str| f(k).filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 1.0) as f32);
    let mk = |kind: PointerKind, x: f64, y: f64| PointerEvent {
        kind,
        x,
        y,
        sx: x * zoom,
        sy: y * zoom,
        shift: b(p, "shift"),
        alt: b(p, "alt"),
        cmd: b(p, "cmd"),
        ctrl: b(p, "ctrl"),
        zoom,
        pressure: pressure("pressure"),
    };
    // A drag's pressure goes from `pressure` to `pressureTo`.
    let (pr0, pr1) = (pressure("pressure"), pressure("pressureTo").or(pressure("pressure")));
    let kind = p.get("kind").and_then(Value::as_str).unwrap_or("click");
    let run = |app: &mut FigApp, ev: PointerEvent| app.session.pointer(&ev).map_err(|e| e.to_string());
    match kind {
        "down" => run(app, mk(PointerKind::Down, x, y))?,
        "move" => run(app, mk(PointerKind::Move, x, y))?,
        "up" => run(app, mk(PointerKind::Up, x, y))?,
        "hover" => run(app, mk(PointerKind::Hover, x, y))?,
        "double" => run(app, mk(PointerKind::DoubleClick, x, y))?,
        "click" => {
            run(app, mk(PointerKind::Down, x, y))?;
            run(app, mk(PointerKind::Up, x, y))?;
        }
        "drag" => {
            let to = p.get("to").and_then(Value::as_array).ok_or("drag needs `to: [x, y]`")?;
            let tx = to.first().and_then(Value::as_f64).ok_or("bad `to`")?;
            let ty = to.get(1).and_then(Value::as_f64).ok_or("bad `to`")?;
            let steps = p.get("steps").and_then(Value::as_u64).unwrap_or(8).clamp(1, 200);
            run(app, mk(PointerKind::Down, x, y))?;
            for i in 1..=steps {
                let t = i as f64 / steps as f64;
                let pressure = pr0.zip(pr1).map(|(a, b)| a + (b - a) * t as f32);
                run(app, PointerEvent { pressure, ..mk(PointerKind::Move, x + (tx - x) * t, y + (ty - y) * t) })?;
            }
            run(app, PointerEvent { pressure: pr1, ..mk(PointerKind::Up, tx, ty) })?;
        }
        other => return Err(format!("unknown pointer kind `{other}`")),
    }
    app.handle_ui_requests();
    Ok(json!({"selection": app.session.selection(), "tool": app.session.tool.tool.id()}))
}

#[cfg(test)]
#[path = "control_roots_tests.rs"]
mod roots_tests;

#[cfg(test)]
#[path = "control_capture_tests.rs"]
mod capture_tests;
