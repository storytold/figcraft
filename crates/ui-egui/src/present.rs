//! The prototype player: presents a frame full-window ([`show`]) or in the preview pane floating
//! over the canvas ([`show_pane`]), and follows its interactions. Both run the same [`player`].
//!
//! The player works on its own copy of the document (interactive components, variables and scroll
//! offsets change it without touching the user's file) and copies the editor's document again
//! whenever it changes, so the preview pane follows edits live. On a device, the presented frame
//! is sized to the device's screen ([`fit_viewport`]): what doesn't fit is clipped, or scrolls when
//! the frame's overflow allows it. A responsive prototype's frame is instead resized to the screen
//! (the device's, else the player's whole area) and laid out again, its constraints and auto layout
//! responding (`figcraft_engine::cmd::proto::respond`, on the player's copy, from the design each
//! time). Device bodies are drawn in code in the device's colour. Every frame is rendered through
//! `figcraft_render::render_node`
//! into a texture. Smart animate interpolates a temporary document between the outgoing and
//! incoming states ([`smart_interpolate`]) and renders it each animation frame; the other
//! transitions move or fade the two cached textures ([`transition_frames`]).
//!
//! The full-window presenter sizes the prototype by its [`PresentView`] (fit, fit width, fill or
//! actual size; Z cycles them); a prototype bigger than the window pans. Scrolling keeps its
//! momentum after a drag or the wheel lets go ([`fling_axis`]), and a drag past the end of a
//! scroll range stretches ([`rubber_band`]) and springs back on release.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use egui::{Align2, Color32, CornerRadius, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use figcraft_color::{Color, Paint, PaintKind};
use figcraft_doc::proto::{
    Action, Device, DeviceColor, Direction, Easing, KeyMods, Overflow, OverlayPosition, PrototypeSettings, ScrollPosition, Transition, Trigger,
};
use figcraft_doc::{Document, HConstraint, Node, NodeId, NodeKind, VConstraint};
use figcraft_geom::{Affine, Point};
use serde_json::{Value, json};

use crate::FigApp;
use crate::a11y::Tip;
use crate::i18n::{tr, tr_noop};
use crate::icons::{self, Icon};
use crate::theme::{ACCENT, Tokens, regular, semibold};
use crate::widgets;

/// A mouse trigger waiting out its delay: interaction `k` of `node`, hit at `rect` while `root`
/// (the screen or the top overlay) showed; due at `due` on the player's clock.
#[derive(Clone, Copy, Debug)]
struct Waiting {
    node: NodeId,
    k: usize,
    rect: Option<Rect>,
    root: NodeId,
    due: f64,
}

/// A page in the player's history (what Back returns to, and how).
#[derive(Clone, Debug)]
pub struct Visit {
    pub frame: NodeId,
    pub transition: Transition,
    pub dur: f64,
    pub easing: Easing,
}

/// An open overlay.
#[derive(Clone, Debug)]
pub struct OverlayState {
    pub id: NodeId,
    pub position: OverlayPosition,
    pub close_on_outside: bool,
    pub background: Option<Color>,
    /// Screen rect of the layer that opened it (manual position).
    pub anchor: Option<Rect>,
    /// Manual position: the overlay's top-left from the anchor's (page units).
    pub offset: Option<[f64; 2]>,
    opened: f64,
    transition: Transition,
    dur: f64,
    easing: Easing,
}

#[derive(Clone)]
struct FrameTex {
    /// The player's revision, the pixel scale and the frames its animated GIFs show.
    key: (u64, u64, u64),
    tex: egui::TextureHandle,
    /// Page-space rect the texture covers.
    bounds: figcraft_geom::Rect,
}

/// A transition in progress from a captured state to the current one.
struct Anim {
    from_doc: Document,
    from_frame: NodeId,
    from_tex: Option<FrameTex>,
    transition: Transition,
    start: f64,
    dur: f64,
    easing: Easing,
    /// Started by a drag: while the pointer stays down the transition follows it (where the drag
    /// began, the unit vector it moves along and the distance that completes it, screen points).
    follow: Option<(Pos2, Vec2, f32)>,
}

/// What a wheel or drag scrolls: a frame with overflow (offsets in page units), or the whole view
/// when the prototype is bigger than the window (offsets in screen points).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scroller {
    Frame(NodeId),
    View,
}

/// Scrolling that keeps going after the wheel or the pointer let go, slowing down; past the end of
/// the range (only after a drag, `bounce`) it springs back.
#[derive(Clone, Copy, Debug)]
struct Fling {
    target: Scroller,
    /// Units per second.
    vel: (f64, f64),
    /// How quickly it slows down (1/s).
    decay: f64,
    bounce: bool,
    /// When it last moved (UI clock, seconds).
    at: f64,
}

/// How quickly a fling slows down after a drag (1/s; it travels its speed / decay).
const DECAY_DRAG: f64 = 2.5;
/// How quickly the wheel's coast slows down (1/s).
const DECAY_WHEEL: f64 = 8.0;
/// Stiffness of the spring that pulls an overscroll back (1/s, critically damped).
const SPRING: f64 = 16.0;
/// A fling inside its range stops below this speed (units/s).
const FLING_STOP: f64 = 6.0;
/// The wheel coasts on when it was faster than this as it stopped (screen points/s).
const COAST_MIN: f64 = 40.0;
/// How far a drag past the end can stretch the content at most (screen points).
const RUBBER_PTS: f64 = 240.0;

/// How the full-window presenter sizes the prototype.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PresentScale {
    /// Scaled down to fit the window (never above 100%).
    #[default]
    Fit,
    /// Scaled to the window's width; what's taller pans.
    FitWidth,
    /// Scaled to cover the whole window; what overflows pans.
    Fill,
    /// At 100%.
    Actual,
}

impl PresentScale {
    const ALL: [PresentScale; 4] = [PresentScale::Fit, PresentScale::FitWidth, PresentScale::Fill, PresentScale::Actual];

    /// Its id in commands (`proto.presentOptions {scale}`).
    pub fn id(self) -> &'static str {
        match self {
            PresentScale::Fit => "fit",
            PresentScale::FitWidth => "fitWidth",
            PresentScale::Fill => "fill",
            PresentScale::Actual => "actual",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|x| x.id() == s)
    }

    /// The options the presenter offers, in the order Z cycles them: on a device there's no Fit
    /// width (the device fits, fills the window or shows at 100%).
    pub fn options(device: bool) -> &'static [PresentScale] {
        if device { &[PresentScale::Fit, PresentScale::Fill, PresentScale::Actual] } else { &Self::ALL }
    }

    pub fn label(self, device: bool) -> &'static str {
        match (self, device) {
            (PresentScale::Fit, false) => tr("Fit to window"),
            (PresentScale::FitWidth, _) => tr("Fit width"),
            (PresentScale::Fill, false) => tr("Fill window"),
            (PresentScale::Actual, false) => tr("Actual size (100%)"),
            (PresentScale::Fit, true) => tr("Fit device to window"),
            (PresentScale::Fill, true) => tr("Zoom device to fill window"),
            (PresentScale::Actual, true) => tr("Device at 100%"),
        }
    }

    /// The option after this one (Z), wrapping round.
    pub fn next(self, device: bool) -> Self {
        let opts = Self::options(device);
        let i = opts.iter().position(|x| *x == self);
        opts.get(i.map_or(0, |i| (i + 1) % opts.len())).copied().unwrap_or_default()
    }
}

/// The full-window presenter's options (kept for later presentations; the preview pane always
/// fits, with the device drawn).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PresentView {
    pub scale: PresentScale,
    /// Draw the device's body around its screen.
    pub device_frame: bool,
    /// Show the comments on the presented frame (Shift+C).
    pub comments: bool,
    /// A click where nothing reacts flashes the hotspots (the layers with interactions).
    pub hotspot_hints: bool,
    /// A panel beside the prototype shows the Inspect properties of the layer under the pointer.
    pub inspect: bool,
}

impl Default for PresentView {
    fn default() -> Self {
        Self { scale: PresentScale::Fit, device_frame: true, comments: false, hotspot_hints: true, inspect: false }
    }
}

/// How long the hotspot hints take to fade out (seconds).
const FLASH_SECS: f64 = 0.5;

/// A frame scrolling from one offset to another.
type ScrollMove = (NodeId, (f64, f64), (f64, f64));

/// "Scroll to" in progress: each moving frame with its offsets from and to.
struct ScrollAnim {
    frames: Vec<ScrollMove>,
    start: f64,
    dur: f64,
    easing: Easing,
}

#[derive(Clone)]
struct Snapshot {
    current: NodeId,
    doc: Document,
    overlays: Vec<OverlayState>,
    history: Vec<Visit>,
    scroll: HashMap<NodeId, (f64, f64)>,
}

/// A "while hovering / while pressing" state that reverts when the pointer leaves / lifts.
struct Temp {
    rect: Rect,
    press: bool,
    snap: Snapshot,
    transition: Transition,
    dur: f64,
    easing: Easing,
}

pub struct Presenter {
    pub current: NodeId,
    pub history: Vec<Visit>,
    pub overlays: Vec<OverlayState>,
    /// Overlays playing their opening transition backwards after being closed (closed at, state).
    closing: Vec<(f64, OverlayState)>,
    /// Scroll offsets of frames with overflow (page units).
    pub scroll: HashMap<NodeId, (f64, f64)>,
    /// Milliseconds the last smart-animate frame took to build and render.
    pub render_ms: f64,
    start: NodeId,
    original: Option<Document>,
    /// The player's document as designed, with what the prototype changed (variables set,
    /// variants changed); `doc` is it laid out for the screen.
    design: Option<Document>,
    /// The player's document as presented: `design` with the presented frame resized to `fit`.
    doc: Option<Document>,
    /// The screen a responsive prototype's frame is resized to (None: frames keep their size).
    fit: Option<(f64, f64)>,
    rev: u64,
    display: Option<(u64, Document)>,
    scroll_anim: Option<ScrollAnim>,
    anim: Option<Anim>,
    anim_tex: Option<egui::TextureHandle>,
    temp: Option<Temp>,
    textures: HashMap<NodeId, FrameTex>,
    /// Small current-screen preview in the full-window bar, cached until its content changes.
    bar_thumb: Option<(NodeId, usize, egui::TextureHandle)>,
    arrived: f64,
    delays_fired: HashSet<(NodeId, usize)>,
    /// Mouse triggers waiting out their delay.
    waiting: Vec<Waiting>,
    hovered: Vec<NodeId>,
    press: Option<Pos2>,
    dragged: bool,
    flash_until: f64,
    /// The active screen's hotspots as last laid out (screen rects).
    hot: Vec<(NodeId, Rect)>,
    /// The time of the last frame played.
    clock: f64,
    /// The layer the Inspect panel shows (the last one under the pointer).
    inspected: Option<NodeId>,
    urls: Vec<String>,
    /// The editor's document as last copied (the player copies it again when it changes).
    source: Option<Arc<Document>>,
    /// Scrolling by dragging: what it scrolls and its offset when the drag began.
    drag_scroll: Option<(Scroller, (f64, f64))>,
    /// How the full-window presenter sizes the prototype.
    pub view: PresentView,
    /// How far the view is panned when the prototype is bigger than the window (points; None:
    /// centred across, at the top).
    pan: Option<(f64, f64)>,
    /// How far the view can pan, and the zoom it shows at (from the last layout).
    pan_range: (f64, f64),
    zoom: f64,
    /// Scrolling that carries on after the input let go.
    fling: Option<Fling>,
    /// The wheel's last target and its speed (units/s), while it turns.
    wheel: Option<(Scroller, (f64, f64))>,
    /// The frame shown last in each section (navigating to a section returns there).
    sections: HashMap<NodeId, NodeId>,
    /// The frame shown last, where it was placed and what it could draw into (comment pins go there).
    shown: Option<(NodeId, Place, Rect)>,
    /// Controller buttons pressed since the last frame (as key trigger names), for the next frame.
    pub pads: Vec<String>,
    /// The prototype has gamepad triggers: the player polls the controllers.
    uses_pads: bool,
    /// The controller buttons held at the last poll (bit i: standard layout index i).
    pads_held: u32,
    /// Clicks on widgets' clickable parts (widget, target), for [`Presenter::click_widgets`].
    widget_clicks: Vec<(NodeId, String)>,
}

impl Presenter {
    /// Close the top overlay; it animates out with its opening transition reversed.
    fn close_overlay(&mut self, now: f64) -> bool {
        let Some(o) = self.overlays.pop() else { return false };
        if !matches!(o.transition, Transition::Instant) && o.dur > 0.0 {
            // An overlay closed mid-opening reverses from where it got to.
            let shown = ((now - o.opened) / o.dur).clamp(0.0, 1.0);
            self.closing.push((now - (1.0 - shown) * o.dur, o));
        }
        self.hovered.clear();
        true
    }

    pub fn new(start: NodeId) -> Self {
        Self {
            current: start,
            history: Vec::new(),
            overlays: Vec::new(),
            closing: Vec::new(),
            scroll: HashMap::new(),
            render_ms: 0.0,
            start,
            original: None,
            design: None,
            doc: None,
            fit: None,
            rev: 0,
            display: None,
            scroll_anim: None,
            anim: None,
            anim_tex: None,
            temp: None,
            textures: HashMap::new(),
            bar_thumb: None,
            arrived: -1.0,
            delays_fired: HashSet::new(),
            waiting: Vec::new(),
            hovered: Vec::new(),
            press: None,
            dragged: false,
            flash_until: 0.0,
            hot: Vec::new(),
            clock: 0.0,
            inspected: None,
            urls: Vec::new(),
            source: None,
            drag_scroll: None,
            view: PresentView::default(),
            pan: None,
            pan_range: (0.0, 0.0),
            zoom: 1.0,
            fling: None,
            wheel: None,
            sections: HashMap::new(),
            shown: None,
            pads: Vec::new(),
            uses_pads: false,
            pads_held: 0,
            widget_clicks: Vec::new(),
        }
    }

    /// Stop scrolling motion (a fling, the wheel's coast, a drag).
    fn stop_scrolling(&mut self) {
        self.fling = None;
        self.wheel = None;
        self.drag_scroll = None;
    }

    /// Size the prototype another way; the view starts over at its top.
    pub fn set_scale(&mut self, scale: PresentScale) {
        self.view.scale = scale;
        self.pan = None;
        self.stop_scrolling();
    }

    /// A new device changes the viewport and may relayout every frame. Start at its top instead
    /// of carrying scroll offsets or a drag from the old screen into the new one.
    fn reset_for_device(&mut self) {
        self.pan = None;
        self.scroll.clear();
        self.scroll_anim = None;
        self.press = None;
        self.dragged = false;
        self.stop_scrolling();
        self.rev += 1;
    }

    /// The current offset of `t` (a frame's scroll, else the view's pan).
    fn offset(&self, t: Scroller) -> (f64, f64) {
        match t {
            Scroller::Frame(f) => self.scroll.get(&f).copied().unwrap_or((0.0, 0.0)),
            Scroller::View => self.pan.unwrap_or((self.pan_range.0 / 2.0, 0.0)),
        }
    }

    /// How far `t` scrolls (`base`: the player's document without scroll offsets).
    fn limits(&self, base: &Document, t: Scroller) -> (f64, f64) {
        match t {
            Scroller::Frame(f) => self.range(base, f),
            Scroller::View => self.pan_range,
        }
    }

    fn set_offset(&mut self, t: Scroller, v: (f64, f64)) {
        if !v.0.is_finite() || !v.1.is_finite() {
            return;
        }
        match t {
            Scroller::Frame(f) => {
                if self.scroll.get(&f) != Some(&v) {
                    self.scroll.insert(f, v);
                    self.rev += 1;
                }
            }
            Scroller::View => self.pan = Some(v),
        }
    }

    /// Move a fling on to `now`; true while it is still moving.
    fn step_fling(&mut self, now: f64) -> bool {
        let Some(mut f) = self.fling.take() else { return false };
        let Some(base) = self.doc.as_ref() else { return false };
        let r = self.limits(base, f.target);
        let x = self.offset(f.target);
        let dt = (now - f.at).clamp(0.0, 0.25);
        f.at = now;
        let (nx, vx, done_x) = fling_axis(x.0, f.vel.0, r.0, dt, f.decay, f.bounce);
        let (ny, vy, done_y) = fling_axis(x.1, f.vel.1, r.1, dt, f.decay, f.bounce);
        self.set_offset(f.target, (nx, ny));
        f.vel = (vx, vy);
        if done_x && done_y {
            return false;
        }
        self.fling = Some(f);
        true
    }

    /// Follow the editor's document: copy it the first time and whenever it changed, keeping the
    /// screen, the overlays and the scroll offsets that still exist (a deleted screen goes back to
    /// the start). Runtime state (variables set, variants changed) starts over from the edit.
    pub fn sync(&mut self, doc: &Arc<Document>, now: f64) {
        if self.source.as_ref().is_some_and(|s| Arc::ptr_eq(s, doc)) {
            return;
        }
        let first = self.source.is_none();
        self.source = Some(doc.clone());
        let copy = Document::clone(doc);
        self.original = Some(copy.clone());
        self.design = Some(copy);
        self.relayout();
        if first {
            self.arrived = now;
            return;
        }
        if !doc.contains(self.current) {
            if doc.contains(self.start) {
                self.restart(self.start, now);
            }
            return;
        }
        self.history.retain(|v| doc.contains(v.frame));
        self.overlays.retain(|o| doc.contains(o.id));
        self.closing.retain(|(_, o)| doc.contains(o.id));
        // These hold states of the document before the edit.
        self.temp = None;
        self.anim = None;
        self.scroll_anim = None;
        self.stop_scrolling();
        self.scroll.retain(|f, _| doc.contains(*f));
        if let Some(laid) = self.doc.as_ref() {
            let clamped: Vec<(NodeId, (f64, f64))> = self.scroll.iter().map(|(f, s)| (*f, clamp_scroll(*s, self.range(laid, *f)))).collect();
            self.scroll.extend(clamped);
        }
        self.rev += 1;
    }

    /// Lay the document out for the screen: `design` with the presented frame resized to `fit`
    /// when the prototype is responsive (its constraints and auto layout respond), instances
    /// playing their variants' interactions and fixed layers on top.
    fn relayout(&mut self) {
        let mut d = self.design.clone();
        if let Some(d) = d.as_mut() {
            if let Some(size) = self.fit {
                figcraft_engine::cmd::proto::respond(d, self.current, size);
            }
            figcraft_engine::proto_run::play_interactions(d);
            raise_fixed(d);
        }
        self.uses_pads = d.as_ref().is_some_and(figcraft_doc::proto::uses_gamepad);
        self.doc = d;
        self.rev += 1;
    }

    /// Responsive prototypes: resize the presented frame to a screen of `size` (None: frames
    /// keep their size). Lays out again only when it changes.
    fn respond_to(&mut self, size: Option<(f64, f64)>) {
        if self.fit != size {
            self.fit = size;
            self.relayout();
        }
    }

    /// The player's document with scroll offsets applied and the presented frame sized to the
    /// device's screen (cached per revision).
    fn display(&mut self) -> Option<Document> {
        if let Some((r, d)) = &self.display
            && *r == self.rev
        {
            return Some(d.clone());
        }
        let doc = self.doc.as_ref()?;
        let mut d = apply_scroll(doc, &self.scroll);
        if let Some(vp) = viewport(doc, self.current) {
            fit_viewport(&mut d, self.current, vp);
        }
        self.display = Some((self.rev, d.clone()));
        Some(d)
    }

    /// How far `frame` scrolls (in `doc`, without scroll offsets): the presented frame within the
    /// device's screen, other frames within themselves.
    fn range(&self, doc: &Document, frame: NodeId) -> (f64, f64) {
        let vp = if frame == self.current { viewport(doc, frame) } else { None };
        scroll_range_in(doc, frame, vp)
    }

    /// What "Scroll to" `dest` moves: every scrolling frame around it, innermost first, brings the
    /// layer (then the frame holding it) to its top-left as far as it can scroll, the innermost
    /// `offset` short of it. Each entry is (frame, offset now, offset after).
    fn scroll_to_layer(&self, doc: &Document, dest: NodeId, offset: [f64; 2]) -> Vec<ScrollMove> {
        let mut out = Vec::new();
        let mut target = dest;
        // The innermost scrolling frame leaves `offset` before the layer.
        let mut off = offset.map(|v| if v.is_finite() { v } else { 0.0 });
        for f in doc.ancestors(dest) {
            let (rx, ry) = self.range(doc, f);
            let Some(tn) = doc.node(target) else { break };
            if rx <= 0.0 && ry <= 0.0 {
                continue;
            }
            let rel = doc.world_transform(f).inverse() * doc.world_transform(target);
            let b = figcraft_geom::transform_rect_bbox(rel, tn.local_rect());
            let from = self.scroll.get(&f).copied().unwrap_or((0.0, 0.0));
            let to = clamp_scroll((if rx > 0.0 { b.x0 - off[0] } else { from.0 }, if ry > 0.0 { b.y0 - off[1] } else { from.1 }), (rx, ry));
            out.push((f, from, to));
            target = f;
            off = [0.0, 0.0];
        }
        out
    }

    /// Scroll the presented frame to `(x, y)` (page units, clamped to what it can scroll).
    pub fn scroll_to(&mut self, x: f64, y: f64) {
        let Some(doc) = self.doc.as_ref() else { return };
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        let to = clamp_scroll((x, y), self.range(doc, self.current));
        self.scroll.insert(self.current, to);
        self.scroll_anim = None;
        self.stop_scrolling();
        self.rev += 1;
    }

    /// The player's state for `ui.inspect`.
    pub fn inspect(&self) -> Value {
        let doc = self.doc.as_ref();
        let pan = self.offset(Scroller::View);
        json!({
            "current": self.current,
            "name": doc.and_then(|d| d.node(self.current)).map(|n| n.name.as_str()),
            "start": self.start,
            "history": self.history.iter().map(|v| v.frame).collect::<Vec<_>>(),
            "overlays": self.overlays.iter().map(|o| o.id).collect::<Vec<_>>(),
            "scroll": self.scroll.get(&self.current).map_or([0.0, 0.0], |s| [s.0, s.1]),
            "scrollRange": doc.map(|d| { let r = self.range(d, self.current); [r.0, r.1] }),
            "device": doc.and_then(|d| page_settings(d, self.current).device()).map(|d| d.name),
            "landscape": doc.is_some_and(|d| page_settings(d, self.current).landscape()),
            "viewport": doc.and_then(|d| viewport(d, self.current)).map(|v| [v.0, v.1]),
            "scale": self.view.scale.id(),
            "deviceFrame": self.view.device_frame,
            "comments": self.view.comments,
            "hotspotHints": self.view.hotspot_hints,
            "hotspots": self.hot.iter().map(|(id, r)| json!({"id": id, "rect": [r.min.x, r.min.y, r.max.x, r.max.y]})).collect::<Vec<_>>(),
            "hotspotFlash": self.flash_until > self.clock,
            "waiting": self.waiting.len(),
            "inspect": self.view.inspect,
            "inspected": self.inspected,
            "deviceColor": doc.and_then(|d| page_settings(d, self.current).device_color()).map(|c| c.name),
            "responsive": self.fit.map(|f| [f.0, f.1]),
            "frameSize": doc.and_then(|d| d.node(self.current)).map(|n| [n.width, n.height]),
            "pan": [pan.0, pan.1],
            "panRange": [self.pan_range.0, self.pan_range.1],
            "zoom": self.zoom,
            "flinging": self.fling.is_some(),
            "gamepad": self.uses_pads,
        })
    }

    fn snapshot(&self) -> Option<Snapshot> {
        Some(Snapshot {
            current: self.current,
            doc: self.design.clone()?,
            overlays: self.overlays.clone(),
            history: self.history.clone(),
            scroll: self.scroll.clone(),
        })
    }

    /// Start a transition from the state as it is now; call before changing the state.
    fn begin(&mut self, transition: Transition, dur: f64, easing: Easing, now: f64) {
        let Some(from_doc) = self.display() else { return };
        let from_tex = self.textures.get(&self.current).cloned();
        self.anim = if matches!(transition, Transition::Instant) || dur <= 0.0 {
            None
        } else {
            Some(Anim { from_doc, from_frame: self.current, from_tex, transition, start: now, dur, easing, follow: None })
        };
    }

    fn arrive(&mut self, now: f64) {
        if let Some(doc) = self.doc.as_ref() {
            figcraft_engine::proto_run::remember_sections(doc, self.current, &mut self.sections);
        }
        self.arrived = now;
        self.delays_fired.clear();
        self.waiting.clear();
        self.hovered.clear();
        self.rev += 1;
    }

    /// Show `frame`. It starts scrolled to the top unless `preserve` keeps the scroll position of
    /// the frame being left (as far as `frame` can scroll).
    fn go(&mut self, frame: NodeId, transition: Transition, dur: f64, easing: Easing, now: f64, remember: bool, preserve: bool) {
        self.begin(transition, dur, easing, now);
        if remember {
            self.history.push(Visit { frame: self.current, transition, dur, easing });
        }
        let kept = self.scroll.get(&self.current).copied().filter(|_| preserve);
        self.overlays.clear();
        self.scroll.clear();
        self.scroll_anim = None;
        self.stop_scrolling();
        self.pan = None;
        self.temp = None;
        self.current = frame;
        self.relayout();
        if let Some(s) = kept
            && let Some(doc) = self.doc.as_ref()
        {
            let to = clamp_scroll(s, self.range(doc, frame));
            self.scroll.insert(frame, to);
        }
        self.arrive(now);
    }

    /// Start over at `start` with the document as the editor has it.
    pub fn restart(&mut self, start: NodeId, now: f64) {
        self.start = start;
        self.current = start;
        self.sections.clear();
        self.history.clear();
        self.overlays.clear();
        self.scroll.clear();
        self.scroll_anim = None;
        self.stop_scrolling();
        self.pan = None;
        self.anim = None;
        self.temp = None;
        self.design = self.original.clone();
        self.relayout();
        self.arrive(now);
    }

    fn revert(&mut self, now: f64) {
        let Some(t) = self.temp.take() else { return };
        self.begin(t.transition.reversed(), t.dur, t.easing, now);
        self.current = t.snap.current;
        self.design = Some(t.snap.doc);
        self.relayout();
        self.overlays = t.snap.overlays;
        self.history = t.snap.history;
        self.scroll = t.snap.scroll;
        self.stop_scrolling();
        self.hovered.clear();
        self.rev += 1;
    }

    /// A mouse enter, leave, down or up on `node` (shown on `root`) met its interaction `k`: its
    /// actions run now, or after its delay while `root` is still what shows (again once only
    /// while it waits).
    fn gesture(&mut self, node: NodeId, k: usize, rect: Option<Rect>, now: f64, root: NodeId) -> bool {
        let delay = self.doc.as_ref().and_then(|d| d.node(node)).and_then(|n| n.interactions.get(k)).map_or(0.0, |it| it.delay_secs());
        if delay <= 0.0 {
            return self.fire(node, k, rect, now);
        }
        if !self.waiting.iter().any(|w| (w.node, w.k) == (node, k)) {
            self.waiting.push(Waiting { node, k, rect, root, due: now + delay });
        }
        false
    }

    /// Run the delayed mouse triggers that are due (those whose screen or overlay went away are
    /// dropped). True when one changed what shows.
    fn fire_waiting(&mut self, root: NodeId, now: f64) -> bool {
        let (due, later): (Vec<Waiting>, Vec<Waiting>) = std::mem::take(&mut self.waiting).into_iter().partition(|w| w.due <= now);
        self.waiting = later;
        self.waiting.retain(|w| w.root == root);
        let mut did = false;
        for w in due.into_iter().filter(|w| w.root == root) {
            // An earlier one may have left the screen: the rest are dropped with it.
            did = did || self.fire(w.node, w.k, w.rect, now);
        }
        did
    }

    /// Run the actions of interaction `k` on `node`. `rect` is the layer's screen rect.
    fn fire(&mut self, node: NodeId, k: usize, rect: Option<Rect>, now: f64) -> bool {
        let Some(it) = self.doc.as_ref().and_then(|d| d.node(node)).and_then(|n| n.interactions.get(k)).cloned() else { return false };
        let dur = it.easing.seconds(it.duration_ms);
        let temp = match it.trigger {
            Trigger::OnHover => Some(false),
            Trigger::OnPress => Some(true),
            _ => None,
        };
        let snap = if temp.is_some() && self.temp.is_none() { self.snapshot() } else { None };
        let mut did = false;
        for a in &it.actions {
            did |= self.perform(a, node, it.transition, dur, it.easing, rect, now);
        }
        if did && let (Some(press), Some(snap), Some(rect)) = (temp, snap, rect) {
            self.temp = Some(Temp { rect, press, snap, transition: it.transition, dur, easing: it.easing });
        }
        did
    }

    /// Run the widget clicks the player took: each calls its plugin back on the player's copy (the
    /// prototype's own state of the widget), as on the canvas but without touching the file.
    /// Returns what went wrong, for a toast.
    pub fn click_widgets(&mut self, s: &figcraft_engine::Session) -> Option<String> {
        let mut error = None;
        for (id, target) in std::mem::take(&mut self.widget_clicks) {
            let Some(doc) = self.design.clone() else { break };
            match figcraft_engine::widgets::click_on(s, doc, id, &target) {
                Ok((d, _)) => {
                    self.design = Some(d);
                    self.relayout();
                }
                Err(e) => error = Some(e.to_string()),
            }
        }
        error
    }

    fn perform(&mut self, a: &Action, node: NodeId, tr: Transition, dur: f64, easing: Easing, rect: Option<Rect>, now: f64) -> bool {
        self.perform_at(a, node, tr, dur, easing, rect, now, 0)
    }

    /// One action; `depth` counts the conditional actions it is inside.
    #[allow(clippy::too_many_arguments)]
    fn perform_at(&mut self, a: &Action, node: NodeId, tr: Transition, dur: f64, easing: Easing, rect: Option<Rect>, now: f64, depth: usize) -> bool {
        let Some(doc) = self.doc.clone() else { return false };
        match a {
            Action::Navigate { dest, preserve_scroll } => {
                let Some(top) = figcraft_engine::proto_run::navigate_target(&doc, *dest, &self.sections) else { return false };
                if !doc.contains(top) || (top == self.current && self.overlays.is_empty()) {
                    return false;
                }
                self.go(top, tr, dur, easing, now, true, *preserve_scroll);
                true
            }
            Action::Back => {
                if self.close_overlay(now) {
                    return true;
                }
                match self.history.pop() {
                    Some(v) => {
                        self.go(v.frame, v.transition.reversed(), v.dur, v.easing, now, false, false);
                        true
                    }
                    None => false,
                }
            }
            Action::ScrollTo { dest, offset } => {
                let frames = self.scroll_to_layer(&doc, *dest, *offset);
                if frames.is_empty() {
                    return false;
                }
                if matches!(tr, Transition::Instant) || dur <= 0.0 {
                    self.scroll.extend(frames.iter().map(|(f, _, to)| (*f, *to)));
                    self.rev += 1;
                } else {
                    self.scroll_anim = Some(ScrollAnim { frames, start: now, dur, easing });
                }
                true
            }
            Action::OpenOverlay { dest, position, close_on_outside, background, offset } => {
                if !doc.contains(*dest) {
                    return false;
                }
                self.delays_fired.retain(|(id, _)| !doc.is_ancestor(*dest, *id));
                self.overlays.push(OverlayState {
                    id: *dest,
                    position: *position,
                    close_on_outside: *close_on_outside,
                    background: *background,
                    anchor: rect,
                    offset: *offset,
                    opened: now,
                    transition: tr,
                    dur,
                    easing,
                });
                self.hovered.clear();
                true
            }
            Action::SwapOverlay { dest } => {
                if !doc.contains(*dest) {
                    return false;
                }
                self.delays_fired.retain(|(id, _)| !doc.is_ancestor(*dest, *id));
                match self.overlays.last_mut() {
                    Some(o) => {
                        o.id = *dest;
                        o.opened = now;
                        o.transition = tr;
                        o.dur = dur;
                        o.easing = easing;
                    }
                    None => self.overlays.push(OverlayState {
                        id: *dest,
                        position: OverlayPosition::Center,
                        close_on_outside: true,
                        background: None,
                        anchor: rect,
                        offset: None,
                        opened: now,
                        transition: tr,
                        dur,
                        easing,
                    }),
                }
                self.hovered.clear();
                true
            }
            Action::CloseOverlay => self.close_overlay(now),
            Action::OpenUrl { url } => {
                if !url.trim().is_empty() {
                    self.urls.push(url.clone());
                }
                true
            }
            Action::ChangeTo { dest } => {
                let Some(inst) = figcraft_engine::proto_run::change_to_target(&doc, node, *dest) else { return false };
                if !doc.contains(*dest) {
                    return false;
                }
                self.begin(tr, dur, easing, now);
                let Some(d) = self.design.as_mut() else { return false };
                if let Some(n) = d.node_mut(inst)
                    && let NodeKind::Instance { component, .. } = &mut n.kind
                {
                    *component = *dest;
                }
                figcraft_engine::components::sync_instance(d, inst);
                self.relayout();
                true
            }
            Action::SetVariable { variable, value, expr } => {
                let Some(d) = self.design.as_mut() else { return false };
                let set = figcraft_engine::proto_run::set_variable(d, node, variable, value, expr.as_ref());
                if set {
                    self.relayout();
                }
                set
            }
            Action::SetVariableMode { collection, mode } => {
                let Some(d) = self.design.as_mut() else { return false };
                let set = figcraft_engine::proto_run::set_mode(d, self.current, collection, mode);
                if set {
                    self.relayout();
                }
                set
            }
            Action::Conditional { blocks } => {
                if depth >= figcraft_doc::proto::MAX_ACTION_DEPTH {
                    return false;
                }
                // Evaluated now: earlier actions of the interaction may have changed variables.
                let mut did = false;
                for inner in figcraft_engine::proto_run::branch(&doc, node, blocks) {
                    did |= self.perform_at(&inner, node, tr, dur, easing, rect, now, depth + 1);
                }
                did
            }
        }
    }

    fn texture(&mut self, ctx: &egui::Context, doc: &Document, id: NodeId, px_scale: f64, now: f64) -> Option<FrameTex> {
        // Animated GIFs play from when the frame arrived: a new texture when one of them moves on,
        // and a wake-up for that moment (nothing else draws frames while it shows). egui wakes a
        // display frame before the time asked for, a little earlier still at times: ask that much
        // later, and count a frame due within a few milliseconds as due.
        let t = (now - self.arrived).max(0.0) + 0.005;
        let clock = figcraft_render::images::animation_clock(doc, id, t);
        if let Some((_, next)) = clock {
            crate::repaint_in(ctx, next + f64::from(ctx.input(|i| i.predicted_dt)));
        }
        let key = (self.rev, px_scale.to_bits(), clock.map_or(0, |c| c.0));
        if let Some(t) = self.textures.get(&id)
            && t.key == key
        {
            return Some(t.clone());
        }
        let bounds = figcraft_render::export_bounds(doc, id)?;
        let pix = figcraft_render::render_node_at(doc, id, texture_scale(ctx, bounds, px_scale), t)?;
        let img = crate::render_worker::screen_image(&pix, doc.color_profile);
        let tex = ctx.load_texture(format!("present-{}", id.0), img, egui::TextureOptions::LINEAR);
        let ft = FrameTex { key, tex, bounds };
        self.textures.insert(id, ft.clone());
        Some(ft)
    }
}

/// The most pixels a presented frame's texture holds (bigger prototypes, at 100% or filling the
/// window, are drawn from a smaller texture scaled up).
const MAX_TEXTURE_PIXELS: f64 = 16_777_216.0;

/// The pixel scale, at most `px`, at which `bounds` makes a texture the GPU takes (no side past
/// its limit) of at most [`MAX_TEXTURE_PIXELS`].
fn texture_scale(ctx: &egui::Context, bounds: figcraft_geom::Rect, px: f64) -> f64 {
    let side = ctx.input(|i| i.max_texture_side) as f64 - 2.0;
    let (w, h) = (bounds.width().max(1.0), bounds.height().max(1.0));
    let s = px.min(side / w.max(h)).min((MAX_TEXTURE_PIXELS / (w * h)).sqrt());
    if s.is_finite() && s > 0.0 { s } else { 1e-3 }
}

/// Page → screen mapping for one presented frame.
#[derive(Clone, Copy, Debug)]
pub struct Place {
    pub page_min: (f64, f64),
    pub origin: Pos2,
    pub scale: f64,
}

impl Place {
    fn centered(fb: figcraft_geom::Rect, center: Pos2, scale: f64) -> Self {
        let size = vec2((fb.width() * scale) as f32, (fb.height() * scale) as f32);
        Place { page_min: (fb.x0, fb.y0), origin: center - size / 2.0, scale }
    }
    pub fn pt(&self, x: f64, y: f64) -> Pos2 {
        pos2(self.origin.x + ((x - self.page_min.0) * self.scale) as f32, self.origin.y + ((y - self.page_min.1) * self.scale) as f32)
    }
    pub fn rect(&self, r: figcraft_geom::Rect) -> Rect {
        Rect::from_min_max(self.pt(r.x0, r.y0), self.pt(r.x1, r.y1))
    }
    /// The page point under screen point `q`.
    pub fn page(&self, q: Pos2) -> Point {
        Point::new(self.page_min.0 + f64::from(q.x - self.origin.x) / self.scale, self.page_min.1 + f64::from(q.y - self.origin.y) / self.scale)
    }
}

// ---------------------------------------------------------------------------------------------
// Pure prototype logic (tested below).

/// Layer names from below `root` down to `id`, joined (the smart-animate matching key).
pub fn name_path(doc: &Document, root: NodeId, id: NodeId) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    let mut cur = id;
    for _ in 0..1024 {
        if cur == root {
            parts.reverse();
            return Some(parts.join("\u{1f}"));
        }
        let n = doc.node(cur)?;
        parts.push(n.name.as_str());
        cur = n.parent?;
    }
    None
}

/// Smart-animate pairs `(from layer, to layer)`: same name path below the two roots; repeated
/// paths pair up in layer order.
pub fn match_layers(from: &Document, from_root: NodeId, to: &Document, to_root: NodeId) -> Vec<(NodeId, NodeId)> {
    let mut pool: HashMap<String, VecDeque<NodeId>> = HashMap::new();
    for id in from.subtree(from_root).into_iter().skip(1) {
        if let Some(p) = name_path(from, from_root, id) {
            pool.entry(p).or_default().push_back(id);
        }
    }
    let mut out = Vec::new();
    for id in to.subtree(to_root).into_iter().skip(1) {
        if let Some(p) = name_path(to, to_root, id)
            && let Some(q) = pool.get_mut(&p)
            && let Some(f) = q.pop_front()
        {
            out.push((f, id));
        }
    }
    out
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Centre, rotation and scale of a node box `(w, h)` placed by `m`.
fn decompose(m: Affine, (w, h): (f64, f64)) -> Option<[f64; 5]> {
    let [a, b, c, d, _, _] = m.as_coeffs();
    let sx = a.hypot(b);
    if !sx.is_finite() || sx < 1e-9 {
        return None;
    }
    let sy = (a * d - b * c) / sx;
    let ctr = m * Point::new(w / 2.0, h / 2.0);
    Some([ctr.x, ctr.y, b.atan2(a), sx, sy])
}

/// Interpolate two placements of a box: centre, rotation (shortest way round) and scale; the box
/// size is `size` (already interpolated).
pub fn lerp_transform(a: Affine, sa: (f64, f64), b: Affine, sb: (f64, f64), size: (f64, f64), t: f64) -> Affine {
    match (decompose(a, sa), decompose(b, sb)) {
        (Some(x), Some(y)) if (x[4] < 0.0) == (y[4] < 0.0) => {
            let mut da = y[2] - x[2];
            while da > std::f64::consts::PI {
                da -= std::f64::consts::TAU;
            }
            while da < -std::f64::consts::PI {
                da += std::f64::consts::TAU;
            }
            let (cx, cy) = (lerp(x[0], y[0], t), lerp(x[1], y[1], t));
            let ang = x[2] + da * t;
            let (sx, sy) = (lerp(x[3], y[3], t), lerp(x[4], y[4], t));
            Affine::translate((cx, cy)) * Affine::rotate(ang) * Affine::scale_non_uniform(sx, sy) * Affine::translate((-size.0 / 2.0, -size.1 / 2.0))
        }
        _ => {
            if t < 0.5 {
                a
            } else {
                b
            }
        }
    }
}

fn lerp_paints(from: &[Paint], to: &mut [Paint], t: f64) {
    for (f, p) in from.iter().zip(to.iter_mut()) {
        if let (PaintKind::Solid { color: fc }, PaintKind::Solid { color: tc }) = (&f.kind, &mut p.kind) {
            *tc = fc.lerp(*tc, t.clamp(0.0, 1.0) as f32);
            p.opacity = lerp(f64::from(f.opacity), f64::from(p.opacity), t.clamp(0.0, 1.0)) as f32;
        }
    }
}

/// The document as it looks at eased progress `t` of a smart-animate transition from
/// `from_root` (in `from`) to `to_root` (in `to`): matched layers tween position, size, rotation,
/// opacity, corner radius and solid fill colours; layers only in the destination fade in and
/// layers only in the source are copied in and fade out. Render `to_root` of the result.
pub fn smart_interpolate(from: &Document, from_root: NodeId, to: &Document, to_root: NodeId, t: f64) -> Document {
    let mut out = to.clone();
    // Springs and back easings go past 1 before they settle: matched layers overshoot too.
    if !t.is_finite() || t == 1.0 {
        return out;
    }
    let t = t.max(-1.0);
    let fade = t.clamp(0.0, 1.0);
    let pairs = match_layers(from, from_root, to, to_root);
    let src_of: HashMap<NodeId, NodeId> = pairs.iter().map(|(f, d)| (*d, *f)).collect();
    let dst_of: HashMap<NodeId, NodeId> = pairs.iter().copied().collect();
    let fw_inv = from.world_transform(from_root).inverse();
    let tw_inv = to.world_transform(to_root).inverse();
    if let (Some(fr), Some(m)) = (from.node(from_root), out.node_mut(to_root)) {
        lerp_paints(&fr.fills, &mut m.fills, t);
    }
    let mut interp: HashMap<NodeId, Affine> = HashMap::new();
    interp.insert(to_root, Affine::IDENTITY);
    let mut faded: HashSet<NodeId> = HashSet::new();
    for id in to.subtree(to_root).into_iter().skip(1) {
        let Some(n) = to.node(id) else { continue };
        let parent = n.parent.unwrap_or(to_root);
        let p_interp = interp.get(&parent).copied().unwrap_or(Affine::IDENTITY);
        match src_of.get(&id).and_then(|s| from.node(*s).map(|sn| (*s, sn))) {
            Some((s, sn)) => {
                let rel_s = fw_inv * from.world_transform(s);
                let rel_d = tw_inv * to.world_transform(id);
                let (w, h) = (lerp(sn.width, n.width, t).max(0.0), lerp(sn.height, n.height, t).max(0.0));
                let xf = lerp_transform(rel_s, (sn.width, sn.height), rel_d, (n.width, n.height), (w, h), t);
                interp.insert(id, xf);
                if let Some(m) = out.node_mut(id) {
                    m.width = w;
                    m.height = h;
                    m.transform = p_interp.inverse() * xf;
                    m.opacity = lerp(f64::from(sn.opacity), f64::from(n.opacity), fade) as f32;
                    for (r, (a, b)) in m.corners.radii.iter_mut().zip(sn.corners.radii.iter().zip(n.corners.radii.iter())) {
                        *r = lerp(*a, *b, t).max(0.0);
                    }
                    lerp_paints(&sn.fills, &mut m.fills, t);
                    lerp_paints(&sn.strokes, &mut m.strokes, t);
                }
            }
            None => {
                interp.insert(id, p_interp * n.transform);
                if !faded.contains(&parent)
                    && let Some(m) = out.node_mut(id)
                {
                    m.opacity *= fade as f32;
                }
                faded.insert(id);
            }
        }
    }
    // Layers only in the source: copy the top-most ones in, fading out.
    let tops: Vec<NodeId> = from
        .subtree(from_root)
        .into_iter()
        .skip(1)
        .filter(|id| !dst_of.contains_key(id))
        .filter(|id| from.node(*id).and_then(|n| n.parent).is_some_and(|p| p == from_root || dst_of.contains_key(&p)))
        .collect();
    for id in tops {
        let Some(n) = from.node(id) else { continue };
        let pf = n.parent.unwrap_or(from_root);
        let matched_parent = dst_of.get(&pf).copied().filter(|p| out.node(*p).is_some_and(|pn| pn.kind.is_container()));
        let (parent, local) = match matched_parent {
            Some(p) if pf != from_root => (p, n.transform),
            _ => (to_root, fw_inv * from.world_transform(id)),
        };
        let nodes: Vec<Node> = from.subtree(id).into_iter().filter_map(|i| from.node(i).cloned()).collect();
        let mut map = BTreeMap::new();
        for nn in &nodes {
            map.insert(nn.id, out.alloc_id());
        }
        let index = from.index_in_parent(id);
        if let Ok(new) = out.paste_nodes(&nodes, &map, id, parent, index)
            && let Some(m) = out.node_mut(new)
        {
            m.transform = local;
            m.opacity *= (1.0 - fade) as f32;
        }
    }
    out
}

/// How far `frame`'s content can scroll `(x, y)` (0 when it doesn't overflow or doesn't scroll).
pub fn scroll_range(doc: &Document, frame: NodeId) -> (f64, f64) {
    scroll_range_in(doc, frame, None)
}

/// How far `frame`'s content can scroll when `viewport` (page units, from its top-left) of it
/// shows, as a device's screen shows a top-level frame: the frame itself is content then, so a
/// frame taller than the screen scrolls through its whole height. Without a viewport the frame
/// shows at its own size and what overflows it scrolls. Only the axes its overflow allows scroll.
pub fn scroll_range_in(doc: &Document, frame: NodeId, viewport: Option<(f64, f64)>) -> (f64, f64) {
    let Some(n) = doc.node(frame) else { return (0.0, 0.0) };
    if n.overflow == Overflow::None {
        return (0.0, 0.0);
    }
    let (mut x1, mut y1) = (0.0f64, 0.0f64);
    for c in &n.children {
        let Some(cn) = doc.node(*c) else { continue };
        if !cn.visible || cn.scroll_position == ScrollPosition::Fixed {
            continue;
        }
        let b = figcraft_geom::transform_rect_bbox(cn.transform, cn.local_rect());
        x1 = x1.max(b.x1);
        y1 = y1.max(b.y1);
    }
    if let Some(l) = &n.layout {
        x1 += l.padding[1];
        y1 += l.padding[2];
    }
    let (vw, vh) = match viewport {
        Some(v) => {
            x1 = x1.max(n.width);
            y1 = y1.max(n.height);
            v
        }
        None => (n.width, n.height),
    };
    let rx = if matches!(n.overflow, Overflow::Horizontal | Overflow::Both) { (x1 - vw).max(0.0) } else { 0.0 };
    let ry = if matches!(n.overflow, Overflow::Vertical | Overflow::Both) { (y1 - vh).max(0.0) } else { 0.0 };
    (if rx.is_finite() { rx } else { 0.0 }, if ry.is_finite() { ry } else { 0.0 })
}

/// `(x, y)` kept within `0..=range` on each axis (0 for non-finite values).
fn clamp_scroll((x, y): (f64, f64), (rx, ry): (f64, f64)) -> (f64, f64) {
    let c = |v: f64, r: f64| if v.is_finite() && r.is_finite() { v.clamp(0.0, r.max(0.0)) } else { 0.0 };
    (c(x, rx), c(y, ry))
}

/// One axis of a fling after `dt` seconds: from offset `x` moving at `v` (units/s) in a range
/// `0..=r`. Inside the range it slows down exponentially (`decay`, 1/s: it travels `v / decay`);
/// past an end it either stops there or, with `bounce`, a critically damped spring pulls it back.
/// Both are solved exactly over short sub-steps, so the motion doesn't depend on the frame rate.
/// Returns the new offset and speed, and whether it has come to rest.
pub fn fling_axis(x: f64, v: f64, r: f64, dt: f64, decay: f64, bounce: bool) -> (f64, f64, bool) {
    if !(x.is_finite() && v.is_finite() && r.is_finite() && dt.is_finite() && decay > 0.0) || r <= 0.0 {
        return (if r.is_finite() && x.is_finite() { x.clamp(0.0, r.max(0.0)) } else { 0.0 }, 0.0, true);
    }
    // A frame is never longer than a second, nor a fling faster than a million units a second.
    let dt = dt.clamp(0.0, 1.0);
    let steps = (dt * 240.0).ceil().clamp(1.0, 64.0);
    let h = dt / steps;
    let (mut x, mut v) = (x, v.clamp(-1e6, 1e6));
    for _ in 0..steps as usize {
        if (0.0..=r).contains(&x) {
            let e = (-decay * h).exp();
            x += v * (1.0 - e) / decay;
            v *= e;
            if !bounce && !(0.0..=r).contains(&x) {
                x = x.clamp(0.0, r);
                v = 0.0;
            }
        } else {
            let end = x.clamp(0.0, r);
            let (d, e) = (x - end, (-SPRING * h).exp());
            let c = v + SPRING * d;
            x = end + (d + c * h) * e;
            v = (v - SPRING * c * h) * e;
        }
    }
    if !x.is_finite() || !v.is_finite() {
        return (0.0, 0.0, true);
    }
    let end = x.clamp(0.0, r);
    if x == end && v.abs() < FLING_STOP {
        return (x, 0.0, true);
    }
    if x != end && (x - end).abs() < 0.5 && v.abs() < FLING_STOP * 4.0 {
        return (end, 0.0, true);
    }
    (x, v, false)
}

/// Where a drag to offset `x` shows in a range `0..=r`: past an end it stretches less the further
/// it goes, never more than `limit` past.
pub fn rubber_band(x: f64, r: f64, limit: f64) -> f64 {
    if !x.is_finite() || !r.is_finite() || r <= 0.0 || !limit.is_finite() || limit <= 0.0 {
        return if x.is_finite() && r.is_finite() { x.clamp(0.0, r.max(0.0)) } else { 0.0 };
    }
    let band = |over: f64| limit * (1.0 - 1.0 / (over * 0.55 / limit + 1.0));
    if x < 0.0 {
        -band(-x)
    } else if x > r {
        r + band(x - r)
    } else {
        x
    }
}

/// The prototype settings of the page holding `id` (defaults when there's none).
pub fn page_settings(doc: &Document, id: NodeId) -> PrototypeSettings {
    doc.page_of(id)
        .and_then(|pg| doc.node(pg))
        .and_then(|n| match &n.kind {
            NodeKind::Page { prototype, .. } => Some(prototype.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// The part of top-level `frame` its page's device shows (page units): the frame clipped to the
/// device's screen. None without a device: the frame then shows whole.
pub fn viewport(doc: &Document, frame: NodeId) -> Option<(f64, f64)> {
    let d = page_settings(doc, frame).device()?;
    let n = doc.node(frame)?;
    Some((n.width.min(d.width), n.height.min(d.height)))
}

/// Size `frame` to the `(w, h)` of it a screen shows. The content keeps its place (what doesn't
/// fit is clipped or scrolls); fixed layers follow their constraints to the screen's edges, so a
/// tab bar pinned to the frame's bottom sits at the bottom of the screen (stretching constraints
/// keep the layer's size).
pub fn fit_viewport(doc: &mut Document, frame: NodeId, (w, h): (f64, f64)) {
    let Some(n) = doc.node(frame) else { return };
    let (dw, dh) = (w - n.width, h - n.height);
    if !dw.is_finite() || !dh.is_finite() || (dw == 0.0 && dh == 0.0) {
        return;
    }
    let fixed: Vec<NodeId> =
        n.children.iter().copied().filter(|c| doc.node(*c).is_some_and(|c| c.scroll_position == ScrollPosition::Fixed)).collect();
    for c in fixed {
        if let Some(m) = doc.node_mut(c) {
            let dx = match m.constraints.horizontal {
                HConstraint::Right => dw,
                HConstraint::Center => dw / 2.0,
                _ => 0.0,
            };
            let dy = match m.constraints.vertical {
                VConstraint::Bottom => dh,
                VConstraint::Center => dh / 2.0,
                _ => 0.0,
            };
            m.transform = Affine::translate((dx, dy)) * m.transform;
        }
    }
    if let Some(m) = doc.node_mut(frame) {
        m.width = w;
        m.height = h;
    }
}

/// Shift scrolling frames' children by their scroll offsets. Fixed children stay put; sticky
/// children scroll until they reach the frame's top/left edge, then stay there.
pub fn apply_scroll(doc: &Document, scroll: &HashMap<NodeId, (f64, f64)>) -> Document {
    let mut out = doc.clone();
    let sticky = |pos: f64, s: f64| if pos >= 0.0 && pos - s < 0.0 { -pos } else { -s };
    for (&frame, &(sx, sy)) in scroll {
        if !sx.is_finite() || !sy.is_finite() || (sx == 0.0 && sy == 0.0) {
            continue;
        }
        let Some(n) = doc.node(frame) else { continue };
        for &c in &n.children {
            let Some(cn) = doc.node(c) else { continue };
            let (dx, dy) = match cn.scroll_position {
                ScrollPosition::Fixed => continue,
                ScrollPosition::Scrolls => (-sx, -sy),
                ScrollPosition::Sticky => {
                    let b = figcraft_geom::transform_rect_bbox(cn.transform, cn.local_rect());
                    (sticky(b.x0, sx), sticky(b.y0, sy))
                }
            };
            if let Some(m) = out.node_mut(c) {
                m.transform = Affine::translate((dx, dy)) * m.transform;
            }
        }
    }
    out
}

/// Fixed layers draw (and take clicks) above the content that scrolls under them: each frame's
/// fixed children move to the top of its children, keeping their order (for the player's copy).
pub fn raise_fixed(doc: &mut Document) {
    let fixed = |c: &NodeId| doc.node(*c).is_some_and(|c| c.scroll_position == ScrollPosition::Fixed);
    let moves: Vec<(NodeId, Vec<NodeId>)> = doc
        .nodes()
        .filter(|n| !matches!(n.kind, NodeKind::Page { .. }) && n.children.iter().any(fixed))
        .filter_map(|n| {
            let (top, mut order): (Vec<NodeId>, Vec<NodeId>) = n.children.iter().partition(|c| fixed(c));
            order.extend(top);
            (order != n.children).then_some((n.id, order))
        })
        .collect();
    for (id, order) in moves {
        if let Some(n) = doc.node_mut(id) {
            n.children = order;
        }
    }
}

/// Where the outgoing and incoming frames are drawn at eased progress `t`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FramePlacement {
    /// Offsets as fractions of each frame's size.
    pub from_off: (f32, f32),
    pub from_alpha: f32,
    pub to_off: (f32, f32),
    pub to_alpha: f32,
    pub from_on_top: bool,
}

/// The unit vector a direction moves along (screen space, y down).
pub fn direction_vec(d: Direction) -> (f32, f32) {
    match d {
        Direction::Left => (-1.0, 0.0),
        Direction::Right => (1.0, 0.0),
        Direction::Top => (0.0, -1.0),
        Direction::Bottom => (0.0, 1.0),
    }
}

/// Frame offsets/opacities for a transition. Directions name where things move: "Move in, left"
/// brings the new frame in from the right edge moving left.
pub fn transition_frames(tr: Transition, t: f32) -> FramePlacement {
    let mul = |(x, y): (f32, f32), k: f32| (x * k, y * k);
    let base = FramePlacement { from_off: (0.0, 0.0), from_alpha: 1.0, to_off: (0.0, 0.0), to_alpha: 1.0, from_on_top: false };
    match tr {
        Transition::Instant => FramePlacement { from_alpha: 0.0, ..base },
        Transition::Dissolve | Transition::SmartAnimate => FramePlacement { to_alpha: t.clamp(0.0, 1.0), ..base },
        Transition::MoveIn { direction } => FramePlacement { to_off: mul(direction_vec(direction), -(1.0 - t)), ..base },
        Transition::MoveOut { direction } => FramePlacement { from_off: mul(direction_vec(direction), t), from_on_top: true, ..base },
        Transition::Push { direction } => {
            let d = direction_vec(direction);
            FramePlacement { from_off: mul(d, t), to_off: mul(d, -(1.0 - t)), ..base }
        }
        Transition::SlideIn { direction } => {
            let d = direction_vec(direction);
            FramePlacement { from_off: mul(d, 0.3 * t), to_off: mul(d, -(1.0 - t)), ..base }
        }
        Transition::SlideOut { direction } => {
            let d = direction_vec(direction);
            FramePlacement { from_off: mul(d, t), to_off: mul(d, -0.3 * (1.0 - t)), from_on_top: true, ..base }
        }
    }
}

/// Screen rect of an overlay of `size` over the frame drawn at `base`. A manual one sits `offset`
/// (screen points) from the top-left of `anchor`, the layer that opened it; without an offset,
/// right below it.
pub fn overlay_rect(pos: OverlayPosition, base: Rect, size: Vec2, anchor: Option<Rect>, offset: Option<Vec2>) -> Rect {
    let cx = base.center().x - size.x / 2.0;
    let cy = base.center().y - size.y / 2.0;
    let (x, y) = match pos {
        OverlayPosition::Center => (cx, cy),
        OverlayPosition::TopLeft => (base.min.x, base.min.y),
        OverlayPosition::TopCenter => (cx, base.min.y),
        OverlayPosition::TopRight => (base.max.x - size.x, base.min.y),
        OverlayPosition::BottomLeft => (base.min.x, base.max.y - size.y),
        OverlayPosition::BottomCenter => (cx, base.max.y - size.y),
        OverlayPosition::BottomRight => (base.max.x - size.x, base.max.y - size.y),
        OverlayPosition::Manual => match (anchor, offset) {
            (Some(a), Some(o)) => (a.min.x + o.x, a.min.y + o.y),
            (Some(a), None) => (a.min.x, a.max.y),
            (None, _) => (cx, cy),
        },
    };
    Rect::from_min_size(pos2(x, y), size)
}

/// The hotspots of the screen `root` (its visible layers with interactions, bottom-most first)
/// placed on screen and clipped by `screen`.
fn hotspots(doc: &Document, root: NodeId, place: &Place, screen: Rect) -> Vec<(NodeId, Rect)> {
    figcraft_engine::proto_run::hotspots(doc, root)
        .into_iter()
        .map(|(id, b)| (id, place.rect(b).intersect(screen)))
        .filter(|(_, r)| r.is_positive())
        .collect()
}

/// The first interaction index on `id` whose trigger satisfies `want`.
fn find_trigger(doc: &Document, id: NodeId, want: impl Fn(&Trigger) -> bool) -> Option<usize> {
    doc.node(id)?.interactions.iter().position(|it| want(&it.trigger))
}

// ---------------------------------------------------------------------------------------------
// The player UI, shared by the full-window presenter and the preview pane.

struct Layout {
    disp: Document,
    scale: f64,
    place: Place,
    /// What shows the prototype: the device's screen, else the presented frame itself.
    screen: Rect,
    /// What the presented frame may draw into: the device's screen, else the whole area (shadows
    /// and overflowing content show around a frame presented without a device).
    clip: Rect,
    device: Option<Device>,
    /// The device is turned to landscape.
    landscape: bool,
    /// The colour of the device's body, when it is drawn around its screen.
    color: Option<DeviceColor>,
    overlays: Vec<(NodeId, Place, Rect)>,
    /// Overlays animating out (same order as `Presenter::closing`).
    closing: Vec<(NodeId, Place, Rect)>,
}

impl Layout {
    /// The active layer (the top overlay, else the presented frame `current`), where it's placed
    /// and what it may draw into.
    fn active(&self, current: NodeId) -> (NodeId, Place, Rect) {
        match self.overlays.last() {
            Some((id, pl, _)) => (*id, *pl, self.screen),
            None => (current, self.place, self.clip),
        }
    }
}

/// The soonest "after delay" interaction in `root`'s subtree that hasn't fired yet, and the seconds
/// until it's due (zero or less: due now). Timers start when the screen arrives, an overlay's when
/// it opens.
fn next_delay(p: &Presenter, doc: &Document, root: NodeId, now: f64) -> Option<((NodeId, usize), f64)> {
    let since = p.overlays.last().filter(|o| o.id == root).map_or(p.arrived, |o| o.opened);
    let mut next: Option<((NodeId, usize), f64)> = None;
    for id in doc.subtree(root) {
        let Some(n) = doc.node(id) else { continue };
        for (k, it) in n.interactions.iter().enumerate() {
            if let Trigger::AfterDelay { ms } = it.trigger
                && !p.delays_fired.contains(&(id, k))
            {
                let due = since + f64::from(ms) / 1000.0 - now;
                if next.is_none_or(|(_, d)| due < d) {
                    next = Some(((id, k), due));
                }
            }
        }
    }
    next
}

/// The width of a device's screen of `size` held upright (its short side when turned).
fn upright_w(size: Vec2, landscape: bool) -> f32 {
    if landscape { size.y } else { size.x }
}

/// Bezel width around a device's screen of upright width `w`, and the screen's corner radius.
fn bezel(device: &str, w: f32) -> (f32, f32) {
    match device {
        "MacBook Air" => ((w * 0.025).max(6.0), 0.0),
        "Desktop" => ((w * 0.02).max(6.0), 0.0),
        d if d.starts_with("iPad") => ((w * 0.05).max(8.0), w * 0.04),
        _ => ((w * 0.06).max(6.0), w * 0.13),
    }
}

/// How far a device's body reaches past its screen of `size`: (each side, top, bottom). A phone
/// turned to landscape has its side buttons above and below.
fn body_outset(device: &str, size: Vec2, landscape: bool) -> (f32, f32, f32) {
    let w = upright_w(size, landscape);
    let (b, _) = bezel(device, w);
    match device {
        "MacBook Air" => (b + w * 0.08, b, b + (w * 0.022).max(6.0)),
        "Desktop" => (b, b, b + w * 0.08 + 6.0),
        _ if landscape => (b, b + 3.0, b + 3.0),
        _ => (b + 3.0, b, b),
    }
}

/// Place the presented frame in `area`: on its device's screen (the whole device sized by the
/// view's scale option), else the frame itself. Fit and 100% keep the chrome's padding from the
/// edges; Fit never goes above 100%. What doesn't fit the window pans (`Presenter::pan`). The
/// preview pane always fits. A responsive prototype's frame is first resized to the device's
/// screen, or without a device to the whole area (shown at 100%).
fn layout(p: &mut Presenter, area: Rect, chrome: Chrome) -> Option<Layout> {
    let settings = page_settings(p.doc.as_ref()?, p.current);
    let (device, landscape) = (settings.device(), settings.landscape());
    let fills = settings.responsive && device.is_none();
    let whole_area = (f64::from(area.width().round()), f64::from(area.height().round()));
    p.respond_to(settings.responsive.then(|| device.map_or(whole_area, |d| (d.width, d.height))));
    let disp = p.display()?;
    let fb = disp.world_bounds(p.current)?;
    let view = if chrome.pane { PresentView::default() } else { p.view };
    // Custom sizes are just a screen: no body.
    let body = device.filter(|d| view.device_frame && !d.colors.is_empty());
    let pad = if !fills && matches!(view.scale, PresentScale::Fit | PresentScale::Actual) { chrome.pad } else { 0.0 };
    let avail = area.shrink(pad);
    let (sw, sh) = device.map_or((fb.width(), fb.height()), |d| (d.width, d.height));
    // The screen at `scale`, the body's outsets around it and the whole size.
    let outer = |scale: f64| {
        let size = vec2((sw * scale) as f32, (sh * scale) as f32);
        let (x, top, bottom) = body.map_or((0.0, 0.0, 0.0), |d| body_outset(d.name, size, landscape));
        (size, x, top, vec2(size.x + 2.0 * x, size.y + top + bottom))
    };
    // The factor that makes something of size `o` fit, fill… the available space.
    let factor = |o: Vec2| {
        let (kx, ky) = (avail.width() / o.x.max(1.0), avail.height() / o.y.max(1.0));
        f64::from(match view.scale {
            PresentScale::Fit => kx.min(ky),
            PresentScale::FitWidth => kx,
            PresentScale::Fill => kx.max(ky),
            PresentScale::Actual => 1.0,
        })
    };
    let mut scale = if fills { 1.0 } else { factor(vec2(sw as f32, sh as f32)) };
    // The body doesn't scale exactly with the screen (its edges have a minimum): size it once more.
    if body.is_some() {
        scale *= factor(outer(scale).3);
    }
    let max = if view.scale == PresentScale::Fit { 1.0 } else { 16.0 };
    let scale = if scale.is_finite() { scale.clamp(0.01, max) } else { 0.01 };
    let (size, x, top, whole) = outer(scale);
    p.zoom = scale;
    // What's bigger than the window pans: starting centred across and at the top.
    let over = |d: f32| if d > 0.5 { d } else { 0.0 };
    let excess = vec2(over(whole.x - avail.width()), over(whole.y - avail.height()));
    p.pan_range = (f64::from(excess.x), f64::from(excess.y));
    // While a drag or fling moves the view it may stretch past the ends; otherwise it stays in range.
    let live = matches!(p.drag_scroll, Some((Scroller::View, _))) || p.fling.is_some_and(|f| f.target == Scroller::View);
    if !live {
        p.pan = p.pan.map(|v| clamp_scroll(v, p.pan_range));
    }
    let pan = p.offset(Scroller::View);
    let along = |excess: f32, min: f32, centre: f32, len: f32, pan: f64| if excess > 0.0 { min - pan as f32 } else { centre - len / 2.0 };
    let min = pos2(along(excess.x, avail.min.x, area.center().x, whole.x, pan.0), along(excess.y, avail.min.y, area.center().y, whole.y, pan.1));
    let screen = Rect::from_min_size(min + vec2(x, top), size);
    let place = Place::centered(fb, screen.center(), scale);
    let place_overlay = |o: &OverlayState| {
        let ob = disp.world_bounds(o.id)?;
        let size = vec2((ob.width() * scale) as f32, (ob.height() * scale) as f32);
        let offset = o.offset.map(|[x, y]| vec2((x * scale) as f32, (y * scale) as f32));
        let r = overlay_rect(o.position, screen, size, o.anchor, offset);
        Some((o.id, Place { page_min: (ob.x0, ob.y0), origin: r.min, scale }, r))
    };
    let overlays = p.overlays.iter().filter_map(place_overlay).collect();
    let closing = p.closing.iter().filter_map(|(_, o)| place_overlay(o)).collect();
    let clip = if device.is_some() { screen } else { area };
    let color = body.and(settings.device_color());
    Some(Layout { disp, scale, place, screen, clip, device, landscape, color, overlays, closing })
}

/// Where the player sits: the whole window or the preview pane.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Chrome {
    pane: bool,
    bar_h: f32,
    /// Space kept around the device or frame.
    pad: f32,
    /// Corner radius of the window around the player (the pane's).
    round: u8,
    /// The player may take keys (the pane only while hovered, and never while the editor is
    /// typing: text on the canvas or in a field).
    keys: bool,
}

const FULL: Chrome = Chrome { pane: false, bar_h: 40.0, pad: 32.0, round: 0, keys: true };

/// What the player's top bar asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BarAction {
    Close,
    Restart,
    /// Present full-window (from the pane).
    Present,
    /// The full-window presenter's options menu (handled in the bar).
    Options,
    /// Turn responsive presentation on or off (saved in the file: `proto.settings`).
    Responsive(bool),
    /// Show or hide the comments on the presented frame.
    Comments,
    /// Show or hide the Inspect panel.
    Inspect,
}

/// The full-window presenter (while `app.ui.present` is set). Escape or Close ends it.
pub fn show(app: &mut FigApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let now = ctx.input(|i| i.time);
    let Some(mut pres) = app.ui.present.take() else { return };
    if let Ok(ds) = app.session.ds() {
        pres.sync(&ds.doc, now);
    }
    poll_pads(app, &mut pres, &ctx);
    let mut action = None;
    // Typing a reply to a comment keeps the keys from the prototype; Shift+C shows the comments.
    let typing = ctx.egui_wants_keyboard_input();
    if !typing && ctx.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::C)) {
        action = Some(BarAction::Comments);
    }
    if pres.view.inspect {
        let t = Tokens::get(&ctx);
        egui::Panel::right("present_inspect")
            .resizable(false)
            .exact_size(INSPECT_W)
            .frame(egui::Frame::NONE.fill(t.panel).stroke(Stroke::new(1.0, t.border)))
            .show(ui, |ui| inspect_panel(app, ui, &pres));
    }
    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
        action = player(&mut pres, ui, ui.max_rect(), now, Chrome { keys: !typing, ..FULL }).or(action);
    });
    let picked_device = navigation_bar(&mut pres, &ctx, now);
    // Keys the prototype left alone: C shows the comments, F fills the screen.
    if !typing {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::C)) {
            action = Some(BarAction::Comments);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F)) {
            let full = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full));
        }
    }
    match action {
        Some(BarAction::Comments) => pres.view.comments = !pres.view.comments,
        Some(BarAction::Inspect) => pres.view.inspect = !pres.view.inspect,
        _ => {}
    }
    if pres.view.comments
        && let Some((frame, place, clip)) = pres.shown
    {
        crate::comments::presenter(app, &ctx, frame, place, clip);
    } else if app.ui.comments.open.is_some() {
        app.ui.comments.close();
    }
    finish(app, &mut pres);
    // The options carry over to the next presentation.
    app.ui.present_view = pres.view;
    if let Some(BarAction::Responsive(on)) = action {
        app.run("proto.settings", json!({"responsive": on}));
    }
    if let Some(device) = picked_device {
        choose_device(app, &mut pres, &device);
    }
    if action == Some(BarAction::Close) || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui.present_uid = None;
        return;
    }
    app.ui.present = Some(pres);
}

/// Device choices in the presenter use the screen's real dimensions and reflow a copy of the
/// design through constraints and auto layout. "No device" returns to the original frame size.
fn choose_device(app: &mut FigApp, p: &mut Presenter, device: &str) {
    let responsive = device != "None";
    if app.run("proto.settings", json!({"device": device, "responsive": responsive})).is_some() {
        p.reset_for_device();
    }
}

/// Controls that stay in reach while viewing a prototype, including screen stepping, device and
/// scale. The selected device is returned to the caller so it can go through the normal document
/// command (and undo/save) rather than changing only the player's copy.
fn navigation_bar(p: &mut Presenter, ctx: &egui::Context, now: f64) -> Option<String> {
    let t = Tokens::get(ctx);
    let doc = p.doc.as_ref()?;
    let frames = doc.top_frames(doc.page_of(p.current).unwrap_or(p.current));
    let index = frames.iter().position(|id| *id == p.current);
    let settings = page_settings(doc, p.current);
    let mut navigate = None;
    let mut device_choice = None;
    egui::Area::new(Id::new("present_navigation")).anchor(Align2::CENTER_BOTTOM, vec2(0.0, -18.0)).order(egui::Order::Foreground).show(ctx, |ui| {
        egui::Frame::NONE
            .fill(t.panel)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(CornerRadius::same(9))
            .inner_margin(egui::Margin::symmetric(8, 5))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let prev = index.and_then(|i| i.checked_sub(1)).and_then(|i| frames.get(i)).copied();
                    let next = index.and_then(|i| frames.get(i + 1)).copied();
                    let previous =
                        ui.add_enabled(prev.is_some(), egui::Button::new("").min_size(vec2(26.0, 24.0))).on_hover_text(tr("Previous frame"));
                    icons::paint(
                        ui.painter(),
                        Rect::from_center_size(previous.rect.center(), vec2(21.0, 21.0)),
                        Icon::ChevronLeft,
                        if prev.is_some() { t.icon } else { t.icon_secondary },
                    );
                    if previous.clicked() {
                        navigate = prev;
                    }
                    ui.label(egui::RichText::new(format!("{} / {}", index.map_or(0, |i| i + 1), frames.len())).color(t.text));
                    let following = ui.add_enabled(next.is_some(), egui::Button::new("").min_size(vec2(26.0, 24.0))).on_hover_text(tr("Next frame"));
                    icons::paint(
                        ui.painter(),
                        Rect::from_center_size(following.rect.center(), vec2(21.0, 21.0)),
                        Icon::ChevronRight,
                        if next.is_some() { t.icon } else { t.icon_secondary },
                    );
                    if following.clicked() {
                        navigate = next;
                    }
                    ui.separator();
                    let model = if settings.device.is_empty() || settings.device == "None" { tr("No device") } else { &settings.device };
                    let device_menu = ui.menu_button(format!("{model}    "), |ui| {
                        navigation_menu(ui, |ui| {
                            egui::ScrollArea::vertical().max_height(410.0).show(ui, |ui| {
                                if ui.selectable_label(settings.device.is_empty() || settings.device == "None", tr("No device")).clicked() {
                                    device_choice = Some("None".to_owned());
                                    ui.close();
                                }
                                ui.separator();
                                for device in PrototypeSettings::DEVICES {
                                    let label = format!("{}  {}×{}", device.name, device.width as u32, device.height as u32);
                                    if ui.selectable_label(settings.device == device.name, label).clicked() {
                                        device_choice = Some(device.name.to_owned());
                                        ui.close();
                                    }
                                }
                                ui.separator();
                                if ui.selectable_label(settings.device == PrototypeSettings::CUSTOM_DEVICE, tr("Custom size")).clicked() {
                                    device_choice = Some(PrototypeSettings::CUSTOM_DEVICE.to_owned());
                                    ui.close();
                                }
                            });
                        });
                    });
                    paint_menu_chevron(ui.painter(), device_menu.response.rect, t.icon_secondary);
                    let scale_menu = ui.menu_button(format!("{}    ", tr(p.view.scale.label(settings.device().is_some()))), |ui| {
                        navigation_menu(ui, |ui| {
                            for scale in PresentScale::options(settings.device().is_some()) {
                                let label = egui::RichText::new(tr(scale.label(settings.device().is_some()))).color(t.menu_text);
                                if ui.selectable_label(p.view.scale == *scale, label).clicked() {
                                    p.set_scale(*scale);
                                    ui.close();
                                }
                            }
                        });
                    });
                    paint_menu_chevron(ui.painter(), scale_menu.response.rect, t.icon_secondary);
                });
            });
    });
    if let Some(id) = navigate {
        p.go(id, Transition::Instant, 0.0, Easing::Linear, now, true, false);
    }
    device_choice
}

fn paint_menu_chevron(painter: &egui::Painter, button: Rect, color: Color32) {
    let icon = Rect::from_center_size(pos2(button.right() - 11.0, button.center().y), vec2(17.0, 17.0));
    icons::paint(painter, icon, Icon::Chevron, color);
}

fn navigation_menu(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let t = Tokens::get(ui.ctx());
    ui.visuals_mut().override_text_color = Some(t.menu_text);
    egui::Frame::NONE.fill(t.menu).stroke(Stroke::new(1.0, t.border)).corner_radius(CornerRadius::same(7)).inner_margin(egui::Margin::same(6)).show(
        ui,
        |ui| {
            ui.set_min_width(212.0);
            add(ui);
        },
    );
}

/// How often the player polls the controllers while its prototype has gamepad triggers (seconds).
const PAD_POLL_SECS: f64 = 1.0 / 30.0;

/// Before a player's frame: the controller buttons pressed since the last one, polled only while
/// the prototype has gamepad triggers and the window has the focus (systems and browsers report
/// controllers to the focused window only; focus coming back draws a frame, which polls again).
fn poll_pads(app: &mut FigApp, p: &mut Presenter, ctx: &egui::Context) {
    if !p.uses_pads || !ctx.input(|i| i.focused) {
        return;
    }
    let held = app.services.gamepad_held();
    let down = held & !p.pads_held;
    p.pads_held = held;
    // Each press once, in layout order.
    let pressed = (0..u32::BITS as usize).filter(|i| down & (1 << i) != 0).filter_map(figcraft_doc::proto::gamepad_button);
    p.pads.extend(pressed.map(str::to_string));
    crate::repaint_in(ctx, PAD_POLL_SECS);
}

/// After a player's frame: the links it opens and the widget clicks it took.
fn finish(app: &mut FigApp, p: &mut Presenter) {
    for u in p.urls.drain(..) {
        app.services.open_url(&u);
    }
    if let Some(e) = p.click_widgets(&app.session) {
        app.toast(e);
    }
}

/// The width of the presenter's Inspect panel.
const INSPECT_W: f32 = 280.0;

/// The presenter's Inspect panel: the inspected layer's properties, as in the Inspect tab.
fn inspect_panel(app: &mut FigApp, ui: &mut Ui, pres: &Presenter) {
    let t = Tokens::get(ui.ctx());
    let found = pres.inspected.zip(pres.doc.as_ref()).and_then(|(id, doc)| doc.node(id).map(|n| (id, doc, n.name.clone())));
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let Some((id, doc, name)) = found else {
            widgets::section_header(ui, tr("Inspect"), &[], false);
            widgets::note(ui, tr("Hover a layer to see its properties"), t.text_secondary);
            return;
        };
        widgets::section_header(ui, &name, &[], false);
        for section in figcraft_engine::devinspect::properties(doc, id) {
            crate::right::inspect_section(app, ui, &section);
        }
    });
}

/// The top-most visible layer of `root` (it included) under the screen point `q`.
fn layer_at(doc: &Document, root: NodeId, place: &Place, q: Pos2) -> Option<NodeId> {
    let mut hidden: HashSet<NodeId> = HashSet::new();
    let mut hit = None;
    for id in doc.subtree(root) {
        let Some(n) = doc.node(id) else { continue };
        if !n.visible || n.parent.is_some_and(|p| hidden.contains(&p)) {
            hidden.insert(id);
            continue;
        }
        if doc.world_bounds(id).is_some_and(|b| place.rect(b).contains(q)) {
            hit = Some(id);
        }
    }
    hit
}

/// The prototype preview pane: the player in a floating, resizable window over the canvas. It
/// follows edits live, and selecting another top-level frame in the editor shows that frame.
pub struct Pane {
    pub player: Presenter,
    /// The open file it previews (another tab closes it).
    doc_uid: u64,
    /// The top-level frame last selected in the editor.
    selected: Option<NodeId>,
}

/// The top-level frame of the editor's first selected layer.
fn selected_frame(ds: &figcraft_engine::DocState) -> Option<NodeId> {
    ds.selection.first().and_then(|i| ds.doc.top_level_frame(*i))
}

impl Pane {
    pub fn new(start: NodeId, session: &figcraft_engine::Session) -> Self {
        let (doc_uid, selected) = session.ds().map(|ds| (ds.uid, selected_frame(ds))).unwrap_or((0, None));
        Pane { player: Presenter::new(start), doc_uid, selected }
    }
}

/// The preview pane's window id (its position and size persist under it).
pub fn pane_id() -> Id {
    Id::new("proto_preview_pane")
}

/// Show the preview pane while `app.ui.preview` is set (call once per frame, before shortcuts:
/// keys a hovered pane's prototype uses don't reach the editor).
pub fn show_pane(app: &mut FigApp, ctx: &egui::Context) {
    let Some(mut pane) = app.ui.preview.take() else { return };
    let Ok(ds) = app.session.ds() else { return };
    if ds.uid != pane.doc_uid {
        return;
    }
    let now = ctx.input(|i| i.time);
    let (doc, selected) = (ds.doc.clone(), selected_frame(ds));
    pane.player.sync(&doc, now);
    // Deselecting and coming back to the same frame keeps the preview where it got to.
    if selected.is_some() && selected != pane.selected {
        pane.selected = selected;
        if let Some(f) = selected.filter(|f| *f != pane.player.current) {
            pane.player.restart(f, now);
        }
    }
    let t = Tokens::get(ctx);
    let canvas = app.canvas.rect;
    let style = ctx.global_style();
    let typing = ds.text_edit.is_some() || crate::a11y::field_focused(ctx);
    let chrome = Chrome { pane: true, bar_h: 36.0, pad: 12.0, round: style.visuals.window_corner_radius.nw, keys: !typing };
    poll_pads(app, &mut pane.player, ctx);
    let mut action = None;
    egui::Window::new(tr("Prototype preview"))
        .id(pane_id())
        .title_bar(false)
        .collapsible(false)
        .resizable(true)
        .default_size(vec2(320.0, 600.0))
        .min_size(vec2(220.0, 260.0))
        .default_pos(pos2(canvas.max.x - 336.0, canvas.min.y + 16.0))
        .constrain_to(ctx.content_rect())
        .frame(egui::Frame::window(&style).fill(t.panel).inner_margin(egui::Margin::same(0)))
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
            action = player(&mut pane.player, ui, rect, now, chrome);
        });
    finish(app, &mut pane.player);
    match action {
        Some(BarAction::Close) => {}
        Some(BarAction::Present) => {
            let start = pane.player.current;
            app.ui.preview = Some(pane);
            app.run("proto.present", json!({"start": start}));
        }
        _ => app.ui.preview = Some(pane),
    }
}

/// The black glass around a device's screen.
const GLASS: Color32 = Color32::from_rgb(0x16, 0x16, 0x18);

/// A device colour lighter (`k` > 0, towards white) or darker (`k` < 0, towards black).
fn shade([r, g, b]: [u8; 3], k: f32) -> Color32 {
    let k = k.clamp(-1.0, 1.0);
    let f = |c: u8| {
        let c = f32::from(c);
        (if k >= 0.0 { c + (255.0 - c) * k } else { c * (1.0 + k) }).round().clamp(0.0, 255.0) as u8
    };
    Color32::from_rgb(f(r), f(g), f(b))
}

/// The width of the coloured band round a device's glass (upright screen width `w`, bezel `b`).
fn rim(w: f32, b: f32) -> f32 {
    (w * 0.016).clamp(1.5, (b * 0.5).max(1.5))
}

/// Body of the presentation device behind the screen `s` (drawn in code: rounded slabs in the
/// device's colour with black glass round the screen). Turned to landscape (a quarter turn
/// counter-clockwise), a phone's left side is at the bottom.
fn device_body(p: &egui::Painter, device: &str, s: Rect, landscape: bool, color: DeviceColor) {
    let (body, edge, metal) = (shade(color.rgb, 0.0), shade(color.rgb, -0.3), shade(color.rgb, 0.12));
    let w = upright_w(s.size(), landscape);
    let (b, screen_r) = bezel(device, w);
    let band = rim(w, b);
    let radius = |r: f32| CornerRadius::same(r.clamp(0.0, 255.0) as u8);
    match device {
        "MacBook Air" => {
            // The lid's coloured edge round black glass, then the base: a thin slab wider than
            // the lid, with a finger notch.
            let lid = s.expand(b);
            p.rect_filled(lid, radius(b * 1.2), body);
            p.rect_filled(lid.shrink(band), radius(b * 1.2 - band), GLASS);
            let base = Rect::from_min_max(pos2(lid.min.x - w * 0.08, lid.max.y), pos2(lid.max.x + w * 0.08, lid.max.y + (w * 0.022).max(6.0)));
            p.rect_filled(base, CornerRadius { nw: 2, ne: 2, sw: 10, se: 10 }, metal);
            let notch = Rect::from_center_size(pos2(base.center().x, base.min.y + 1.5), vec2(w * 0.12, 3.0));
            p.rect_filled(notch, CornerRadius::same(2), edge);
        }
        "Desktop" => {
            let screen = s.expand(b);
            p.rect_filled(screen, CornerRadius::same(8), body);
            p.rect_filled(screen.shrink(band), radius(8.0 - band), GLASS);
            let neck =
                Rect::from_min_max(pos2(screen.center().x - w * 0.06, screen.max.y), pos2(screen.center().x + w * 0.06, screen.max.y + w * 0.08));
            p.rect_filled(neck, CornerRadius::ZERO, metal);
            let foot = Rect::from_min_max(pos2(neck.min.x - w * 0.06, neck.max.y), pos2(neck.max.x + w * 0.06, neck.max.y + 6.0));
            p.rect_filled(foot, CornerRadius::same(3), edge);
        }
        _ => {
            // Phones and tablets: a rounded slab (phones with side buttons).
            let slab = s.expand(b);
            if !device.starts_with("iPad") {
                // (on the left side, from the top, length) as fractions of the upright side.
                for (left, f0, h) in [(true, 0.18, 0.06), (true, 0.27, 0.1), (false, 0.24, 0.13)] {
                    let btn = if landscape {
                        let y = if left { slab.max.y - 0.5 } else { slab.min.y - 2.5 };
                        Rect::from_min_size(pos2(slab.min.x + slab.width() * f0, y), vec2(slab.width() * h, 2.5))
                    } else {
                        let x = if left { slab.min.x - 2.0 } else { slab.max.x };
                        Rect::from_min_size(pos2(x, slab.min.y + slab.height() * f0), vec2(2.5, slab.height() * h))
                    };
                    p.rect_filled(btn, CornerRadius::same(1), edge);
                }
            }
            p.rect_filled(slab, radius(screen_r + b), body);
            p.rect_stroke(slab, radius(screen_r + b), Stroke::new(1.0, edge), StrokeKind::Inside);
            // The glass reaches round the screen's square corners (masked by `device_glass`).
            let glass = b - band;
            p.rect_filled(slab.shrink(band), radius((screen_r + glass).min(glass * 3.4)), GLASS);
        }
    }
}

/// What sits on top of the screen: rounded screen corners (glass masks the frame's square
/// corners) and a phone's camera pill (at the left when turned to landscape).
fn device_glass(p: &egui::Painter, device: &str, s: Rect, landscape: bool) {
    let w = upright_w(s.size(), landscape);
    let (_, screen_r) = bezel(device, w);
    if screen_r <= 0.0 {
        return;
    }
    p.add(corner_masks(s, screen_r.min(s.width() / 2.0).min(s.height() / 2.0), GLASS));
    if device.starts_with("iPhone") {
        let pill = if landscape {
            Rect::from_center_size(pos2(s.min.x + w * 0.045, s.center().y), vec2(w * 0.085, w * 0.3))
        } else {
            Rect::from_center_size(pos2(s.center().x, s.min.y + w * 0.045), vec2(w * 0.3, w * 0.085))
        };
        p.rect_filled(pill, CornerRadius::same((w * 0.0425).min(255.0) as u8), Color32::BLACK);
    }
}

/// Fill what lies between each corner of `s` and its rounding of radius `r` (fans of triangles
/// from the corner to the arc).
fn corner_masks(s: Rect, r: f32, color: Color32) -> egui::Shape {
    const SEGMENTS: usize = 12;
    if !(r.is_finite() && r > 0.0) {
        return egui::Shape::Noop;
    }
    let mut mesh = egui::Mesh::default();
    // (corner, centre of its arc, angle the arc starts at).
    let corners = [
        (s.left_top(), s.left_top() + vec2(r, r), std::f32::consts::PI),
        (s.right_top(), s.right_top() + vec2(-r, r), std::f32::consts::FRAC_PI_2 * 3.0),
        (s.right_bottom(), s.right_bottom() + vec2(-r, -r), 0.0),
        (s.left_bottom(), s.left_bottom() + vec2(r, -r), std::f32::consts::FRAC_PI_2),
    ];
    for (corner, centre, start) in corners {
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(corner, color);
        for k in 0..=SEGMENTS {
            let a = start + std::f32::consts::FRAC_PI_2 * k as f32 / SEGMENTS as f32;
            mesh.colored_vertex(centre + r * vec2(a.cos(), a.sin()), color);
        }
        for k in 0..SEGMENTS as u32 {
            mesh.add_triangle(base, base + 1 + k, base + 2 + k);
        }
    }
    egui::Shape::mesh(mesh)
}

/// One frame of the player in `rect`: the top bar, then the prototype (on its device) below.
/// Returns what the bar asked for (restarting is done here).
fn player(p: &mut Presenter, ui: &mut Ui, rect: Rect, now: f64, chrome: Chrome) -> Option<BarAction> {
    let ctx = ui.ctx().clone();
    p.clock = now;
    let bar = Rect::from_min_size(rect.min, vec2(rect.width(), chrome.bar_h));
    let area = Rect::from_min_max(pos2(rect.min.x, bar.max.y), rect.max);
    let bg = crate::theme::doc_c32(ui.ctx(), p.doc.as_ref().map(|d| page_settings(d, p.current)).unwrap_or_default().background);
    let r = chrome.round;
    ui.painter().rect_filled(area, CornerRadius { nw: 0, ne: 0, sw: r, se: r }, bg);
    let action = top_bar(p, ui, bar, now, chrome);
    if action == Some(BarAction::Restart) {
        p.restart(p.start, now);
    }
    if p.doc.as_ref().is_none_or(|d| !d.contains(p.current)) {
        if !chrome.pane {
            return Some(BarAction::Close);
        }
        ui.painter().text(area.center(), Align2::CENTER_CENTER, tr("Select a frame to preview it"), regular(12.0), Color32::from_gray(170));
        return action;
    }
    // Scroll animation (Scroll to).
    if let Some(sa) = &p.scroll_anim {
        let raw = ((now - sa.start) / sa.dur.max(1e-3)).clamp(0.0, 1.0);
        let e = sa.easing.eval(raw);
        for (f, from, to) in &sa.frames {
            p.scroll.insert(*f, (lerp(from.0, to.0, e), lerp(from.1, to.1, e)));
        }
        p.rev += 1;
        if raw >= 1.0 {
            p.scroll_anim = None;
        }
        ctx.request_repaint();
    }
    // Scrolling that carries on after the input let go (repaints only while it moves).
    if p.step_fling(now) {
        ctx.request_repaint();
    }
    let resp = ui.interact(area, ui.id().with("present_area"), Sense::click_and_drag());
    let over = resp.contains_pointer();
    if let Some(l) = layout(p, area, chrome) {
        // The pane only takes the keyboard while the pointer is over it.
        input(p, &ctx, &l, over, chrome.keys && (!chrome.pane || over), chrome.pane, now);
    }
    let Some(l) = layout(p, area, chrome) else { return action };
    p.shown = Some((p.current, l.place, l.clip.intersect(area)));
    let painter = ui.painter_at(area);
    let body = l.device.zip(l.color);
    if l.device.is_some() {
        if let Some((d, c)) = body {
            device_body(&painter, d.name, l.screen, l.landscape, c);
        }
        painter.rect_filled(l.screen, CornerRadius::ZERO, bg);
    }
    draw(p, &painter, &ctx, &l, now);
    if p.view.inspect && !chrome.pane {
        inspect_hover(p, &painter, &ctx, &l, area);
    }
    if let Some((d, _)) = body {
        device_glass(&painter, d.name, l.screen, l.landscape);
    }
    // Timers: wake up for the soonest pending "after delay" of the layer that can fire it (they
    // wait while a transition plays, which draws every frame anyway).
    if p.anim.is_none()
        && let Some((_, due)) = next_delay(p, &l.disp, l.active(p.current).0, now)
    {
        crate::repaint_in(&ctx, due.min(60.0));
    }
    // And for the soonest mouse trigger waiting out its delay.
    if let Some(due) = p.waiting.iter().map(|w| w.due - now).reduce(f64::min) {
        crate::repaint_in(&ctx, due.clamp(0.0, 60.0));
    }
    action
}

/// Inspecting: the layer under the pointer becomes the inspected one; it is outlined.
fn inspect_hover(p: &mut Presenter, painter: &egui::Painter, ctx: &egui::Context, l: &Layout, area: Rect) {
    let (root, place, clip) = l.active(p.current);
    if let Some(q) = ctx.pointer_hover_pos().filter(|q| area.contains(*q) && clip.contains(*q))
        && let Some(id) = layer_at(&l.disp, root, &place, q)
    {
        p.inspected = Some(id);
    }
    if let Some(id) = p.inspected.filter(|id| *id == root || l.disp.is_ancestor(root, *id))
        && let Some(b) = l.disp.world_bounds(id)
    {
        painter.with_clip_rect(clip).rect_stroke(place.rect(b), CornerRadius::ZERO, Stroke::new(1.5, ACCENT), StrokeKind::Outside);
    }
}

/// The bar above the prototype: the flow picker and the frame's name on the left, the buttons on
/// the right (text on the dark full-window bar, icons in the pane, which follows the theme).
fn top_bar(p: &mut Presenter, ui: &mut Ui, bar: Rect, now: f64, chrome: Chrome) -> Option<BarAction> {
    let t = Tokens::get(ui.ctx());
    let (fill, text, dim, hover) = (t.panel, t.text, t.text_secondary, t.hover);
    let r = chrome.round;
    ui.painter().rect_filled(bar, CornerRadius { nw: r, ne: r, sw: 0, se: 0 }, fill);
    if chrome.pane {
        ui.painter().line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0, t.border));
    }
    let source_key = p.source.as_ref().map_or(0, |s| Arc::as_ptr(s) as usize);
    if !chrome.pane && p.bar_thumb.as_ref().is_none_or(|(id, source, _)| *id != p.current || *source != source_key) {
        p.bar_thumb =
            p.source.as_ref().and_then(|d| crate::assets_panel::thumbnail(ui.ctx(), d, p.current, 38.0)).map(|tex| (p.current, source_key, tex));
    }
    let doc = p.doc.as_ref()?;
    let name = doc.node(p.current).map(|n| n.name.clone()).unwrap_or_default();
    let flows = match doc.page_of(p.start).and_then(|pg| doc.node(pg)).map(|n| &n.kind) {
        Some(NodeKind::Page { flows, .. }) => flows.clone(),
        _ => vec![],
    };
    let settings = page_settings(doc, p.current);
    let device = settings.device();
    let mut action = None;
    // Right: the buttons, right to left (English labels key their ids; shown translated).
    let buttons: &[(BarAction, &str, Icon)] = if chrome.pane {
        &[
            (BarAction::Close, tr_noop("Close preview"), Icon::Close),
            (BarAction::Present, tr_noop("Present"), Icon::Play),
            (BarAction::Restart, tr_noop("Restart"), Icon::Reset),
        ]
    } else {
        &[
            (BarAction::Close, tr_noop("Close"), Icon::Close),
            (BarAction::Restart, tr_noop("Restart"), Icon::Reset),
            (BarAction::Options, tr_noop("Options"), Icon::Dots),
            (BarAction::Comments, tr_noop("Comments"), Icon::Comment),
            (BarAction::Inspect, tr_noop("Inspect"), Icon::Code),
        ]
    };
    let mut x = bar.max.x - if chrome.pane { 6.0 } else { 8.0 };
    for (act, label, icon) in buttons {
        let shown = tr(label);
        let w = if chrome.pane { 24.0 } else { ui.painter().layout_no_wrap(shown.to_string(), regular(11.5), text).size().x + 16.0 };
        let br = Rect::from_min_size(pos2(x - w, bar.center().y - 12.0), vec2(w, 24.0));
        x = br.min.x - if chrome.pane { 2.0 } else { 4.0 };
        let mut resp = ui.interact(br, ui.id().with(("pbar", *label)), Sense::click());
        if chrome.pane {
            resp = resp.tip(shown);
        }
        let on = (*act == BarAction::Comments && p.view.comments) || (*act == BarAction::Inspect && p.view.inspect);
        if on {
            ui.painter().rect_filled(br, CornerRadius::same(5), ACCENT);
        } else if resp.hovered() {
            ui.painter().rect_filled(br, CornerRadius::same(5), hover);
        }
        if *act == BarAction::Comments {
            resp = resp.tip(tr("Show comments (Shift+C)"));
        }
        if *act == BarAction::Inspect {
            resp = resp.tip(tr("Inspect the layer under the pointer: size, colours, typography"));
        }
        if chrome.pane {
            icons::paint(ui.painter(), br.shrink(5.0), *icon, t.icon);
        } else {
            ui.painter().text(br.center(), Align2::CENTER_CENTER, shown, regular(11.5), text);
        }
        if *act == BarAction::Options {
            let menu = crate::right::menu_popup(&resp).width(220.0).show(|ui| options_menu(p, ui, device, settings.responsive, now));
            action = action.or(menu.and_then(|m| m.inner));
        } else if resp.clicked() {
            action = Some(*act);
        }
    }
    // Left: the flow picker and the frame's name, clipped before the buttons.
    let left = ui.painter().with_clip_rect(Rect::from_min_max(bar.min, pos2(x, bar.max.y)));
    let mut lx = bar.min.x + if chrome.pane { 8.0 } else { 12.0 };
    if !chrome.pane {
        let thumb = Rect::from_center_size(pos2(lx + 15.0, bar.center().y), vec2(28.0, 28.0));
        left.rect_filled(thumb, CornerRadius::same(4), t.field);
        if let Some((_, _, tex)) = &p.bar_thumb {
            let size = tex.size_vec2();
            if size.x > 0.0 && size.y > 0.0 {
                let scaled = size * (26.0 / size.x.max(size.y));
                let dst = Rect::from_center_size(thumb.center(), scaled);
                left.image(tex.id(), dst, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
        }
        lx += 38.0;
    }
    // The flow playing: its name on the picker, its description on hover; the menu lists each
    // flow with its description.
    let playing = flows.iter().find(|f| f.start == p.start);
    if !flows.is_empty() {
        let label = playing.map_or(tr("Flows"), |f| f.name.as_str()).to_owned();
        let tip = playing.map(|f| f.description.trim()).filter(|d| !d.is_empty()).unwrap_or(tr("Flows")).to_string();
        let galley = left.layout_no_wrap(label, semibold(11.5), text);
        let fr = Rect::from_min_size(pos2(lx, bar.center().y - 12.0), vec2(galley.size().x + 30.0, 24.0));
        let resp = ui.interact(fr.intersect(left.clip_rect()), ui.id().with("pflows"), Sense::click()).tip(tip);
        if resp.hovered() {
            left.rect_filled(fr, CornerRadius::same(5), hover);
        }
        left.galley(pos2(fr.min.x + 8.0, fr.center().y - galley.size().y / 2.0), galley, text);
        paint_menu_chevron(&left, fr, t.icon_secondary);
        crate::right::menu_popup(&resp).width(240.0).show(|ui| {
            let t = Tokens::get(ui.ctx());
            for f in &flows {
                if crate::right::menu_check(ui, f.start == p.start, &f.name, "") {
                    p.restart(f.start, now);
                    ui.close();
                }
                let d = f.description.trim();
                if !d.is_empty() {
                    egui::Frame::NONE.inner_margin(egui::Margin { left: 26, right: 8, top: 0, bottom: 4 }).show(ui, |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(d).font(regular(10.5)).color(t.text_secondary)).wrap());
                    });
                }
            }
        });
        lx = fr.max.x + 8.0;
        left.text(pos2(lx, bar.center().y), Align2::LEFT_CENTER, "/", regular(11.5), dim);
        lx += 14.0;
    }
    left.text(pos2(lx, bar.center().y), Align2::LEFT_CENTER, name, regular(11.5), text);
    action
}

/// The full-window presenter's options menu: how the prototype is sized (Z cycles them), whether
/// the frame is responsive (a file setting: the frame takes the window's size, or on a device the
/// screen's, instead of being scaled or shown at a fixed size), the device's body, hotspot hints,
/// restart (R).
fn options_menu(p: &mut Presenter, ui: &mut Ui, device: Option<Device>, responsive: bool, now: f64) -> Option<BarAction> {
    use crate::right::menu_check;
    let mut action = None;
    let on_device = device.is_some();
    // Without a device, responsive is one of the sizes: the frame fills the window at 100%.
    if !on_device && menu_check(ui, responsive, tr("Responsive"), "") {
        action = Some(BarAction::Responsive(true));
        ui.close();
    }
    for s in PresentScale::options(on_device) {
        if menu_check(ui, p.view.scale == *s && (on_device || !responsive), s.label(on_device), "") {
            p.set_scale(*s);
            if !on_device && responsive {
                action = Some(BarAction::Responsive(false));
            }
            ui.close();
        }
    }
    if (on_device || !responsive) && menu_check(ui, false, tr("Next size option"), "Z") {
        p.set_scale(p.view.scale.next(on_device));
        ui.close();
    }
    ui.separator();
    // On a device: the frame takes the screen's size, or keeps its own (clipped or scrolling).
    if on_device {
        for (on, label) in [(true, tr("Responsive")), (false, tr("Fixed size"))] {
            if menu_check(ui, responsive == on, label, "") {
                action = (responsive != on).then_some(BarAction::Responsive(on));
                ui.close();
            }
        }
        ui.separator();
    }
    if device.is_some_and(|d| !d.colors.is_empty()) && menu_check(ui, p.view.device_frame, tr("Show device frame"), "") {
        p.view.device_frame = !p.view.device_frame;
        ui.close();
    }
    if menu_check(ui, p.view.hotspot_hints, tr("Show hotspot hints"), "") {
        p.view.hotspot_hints = !p.view.hotspot_hints;
        ui.close();
    }
    if menu_check(ui, false, tr("Restart"), "R") {
        p.restart(p.start, now);
        ui.close();
    }
    action
}

/// Scrolling frames under the screen point `q`, innermost first: those of `root` (in the display
/// document `disp`) that can scroll (`base` is the player's document without scroll offsets).
fn scrollers_at(p: &Presenter, base: &Document, disp: &Document, root: NodeId, place: &Place, q: Pos2) -> Vec<NodeId> {
    let mut out: Vec<NodeId> = disp
        .subtree(root)
        .into_iter()
        .filter(|f| disp.node(*f).is_some_and(|n| n.visible && n.overflow != Overflow::None))
        .filter(|f| disp.world_bounds(*f).is_some_and(|b| place.rect(b).contains(q)))
        .filter(|f| p.range(base, *f) != (0.0, 0.0))
        .collect();
    out.reverse();
    out
}

/// Keep a key the prototype used from reaching the editor's shortcuts.
fn consume_key(ctx: &egui::Context, key: egui::Key) {
    ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Key { key: k, .. } if *k == key)));
}

/// Screen points → `t`'s units (page units for frames, points for the view).
fn units_per_point(t: Scroller, scale: f64) -> f64 {
    match t {
        Scroller::Frame(_) => 1.0 / scale.max(1e-6),
        Scroller::View => 1.0,
    }
}

/// Pointer, wheel, keys and timers. `over_area`: the pointer is over the player; `keys_on`: the
/// player takes the keyboard; `pane`: it plays in the preview pane (which always fits).
#[allow(clippy::too_many_arguments)]
fn input(p: &mut Presenter, ctx: &egui::Context, l: &Layout, over_area: bool, keys_on: bool, pane: bool, now: f64) {
    let (pos, pressed, released, down, wheel, velocity, dt, keys) = ctx.input(|i| {
        let keys: Vec<(egui::Key, egui::Modifiers)> = i
            .events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } => Some((*key, *modifiers)),
                _ => None,
            })
            .collect();
        (
            i.pointer.hover_pos(),
            i.pointer.button_pressed(egui::PointerButton::Primary),
            i.pointer.button_released(egui::PointerButton::Primary),
            i.pointer.primary_down(),
            i.smooth_scroll_delta,
            i.pointer.velocity(),
            f64::from(i.unstable_dt).clamp(1.0 / 500.0, 0.1),
            keys,
        )
    });
    let keys = if keys_on { keys } else { Vec::new() };
    let pads = std::mem::take(&mut p.pads);
    // A transition a drag started follows the pointer while it stays down, then plays on from there.
    if let Some(a) = p.anim.as_mut()
        && let Some((from, dir, len)) = a.follow
    {
        match pos.filter(|_| down) {
            Some(q) => a.start = now - f64::from(((q - from).dot(dir) / len).clamp(0.0, 1.0)) * a.dur,
            None => a.follow = None,
        }
    }
    let pos = pos.filter(|_| over_area || p.press.is_some());
    // A drag that scrolled lets go: it carries on at the pointer's speed (and springs back from
    // past the ends).
    if !down && let Some((t, _)) = p.drag_scroll.take() {
        let u = units_per_point(t, l.scale);
        let r = p.doc.as_ref().map_or((0.0, 0.0), |b| p.limits(b, t));
        let along = |v: f32, r: f64| if r > 0.0 { -f64::from(v) * u } else { 0.0 };
        p.fling = Some(Fling { target: t, vel: (along(velocity.x, r.0), along(velocity.y, r.1)), decay: DECAY_DRAG, bounce: true, at: now });
    }
    // The active layer: the top overlay, else the frame.
    let (root, place, clip) = l.active(p.current);
    let doc = &l.disp;
    let hot = hotspots(doc, root, &place, clip);
    p.hot.clone_from(&hot);
    if p.fire_waiting(root, now) {
        return;
    }
    // "While hovering / pressing" reverts.
    if let Some(tmp) = &p.temp {
        let leave = if tmp.press { released || !down } else { pos.is_none_or(|q| !tmp.rect.contains(q)) };
        if leave {
            p.revert(now);
            return;
        }
    }
    // The wheel scrolls the innermost frame under the pointer that can still move that way, else
    // pans a view bigger than the window. When it stops it coasts on, slowing down.
    if wheel != Vec2::ZERO
        && let Some(q) = pos
        && let Some(base) = p.doc.clone()
    {
        let mut targets: Vec<Scroller> = scrollers_at(p, &base, doc, root, &place, q).into_iter().map(Scroller::Frame).collect();
        if p.pan_range != (0.0, 0.0) {
            targets.push(Scroller::View);
        }
        let mut moved = None;
        for t in targets {
            let u = units_per_point(t, l.scale);
            let cur = p.offset(t);
            let next = clamp_scroll((cur.0 - f64::from(wheel.x) * u, cur.1 - f64::from(wheel.y) * u), p.limits(&base, t));
            if (next.0 - cur.0).abs() > 1e-6 || (next.1 - cur.1).abs() > 1e-6 {
                p.set_offset(t, next);
                p.scroll_anim = None;
                p.fling = None;
                moved = Some((t, ((next.0 - cur.0) / dt, (next.1 - cur.1) / dt)));
                break;
            }
        }
        // The wheel's speed, smoothed over ~50 ms whatever the frame rate.
        p.wheel = moved.map(|(t, inst)| {
            let prev = p.wheel.filter(|w| w.0 == t).map_or(inst, |w| w.1);
            let a = 1.0 - (-dt / 0.05).exp();
            (t, (lerp(prev.0, inst.0, a), lerp(prev.1, inst.1, a)))
        });
    } else if wheel == Vec2::ZERO
        && let Some((t, v)) = p.wheel.take()
        && v.0.hypot(v.1) > COAST_MIN * units_per_point(t, l.scale)
    {
        p.fling = Some(Fling { target: t, vel: v, decay: DECAY_WHEEL, bounce: false, at: now });
    }
    let hits: Vec<(NodeId, Rect)> = hot.iter().rev().filter(|(_, r)| pos.is_some_and(|q| r.contains(q))).copied().collect();
    if !hits.is_empty() {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    // First (top-most) hit that has a matching trigger.
    let pick = |want: &dyn Fn(&Trigger) -> bool| -> Option<(NodeId, usize, Rect)> {
        hits.iter().find_map(|(id, r)| find_trigger(doc, *id, want).map(|k| (*id, k, *r)))
    };
    // Enter / leave.
    let ids: Vec<NodeId> = hits.iter().map(|h| h.0).collect();
    let entered: Vec<(NodeId, Rect)> = hits.iter().filter(|(id, _)| !p.hovered.contains(id)).copied().collect();
    let left: Vec<NodeId> = p.hovered.iter().filter(|id| !ids.contains(id)).copied().collect();
    p.hovered = ids;
    for id in left {
        if let Some(k) = find_trigger(doc, id, |t| *t == Trigger::MouseLeave)
            && p.gesture(id, k, None, now, root)
        {
            return;
        }
    }
    for (id, r) in &entered {
        if let Some(k) = find_trigger(doc, *id, |t| *t == Trigger::MouseEnter)
            && p.gesture(*id, k, Some(*r), now, root)
        {
            return;
        }
    }
    if p.temp.is_none()
        && !entered.is_empty()
        && let Some((id, k, r)) = pick(&|t| *t == Trigger::OnHover)
        && entered.iter().any(|e| e.0 == id)
        && p.fire(id, k, Some(r), now)
    {
        return;
    }
    // Press / drag / release.
    if pressed && pos.is_some() && over_area {
        p.press = pos;
        p.dragged = false;
        // Pressing catches a fling (one springing back from past an end carries on).
        let overscrolled = |f: &Fling| p.doc.as_ref().is_some_and(|b| clamp_scroll(p.offset(f.target), p.limits(b, f.target)) != p.offset(f.target));
        p.fling = p.fling.filter(overscrolled);
        p.wheel = None;
        if let (Some((_, _, orect)), Some(o), Some(q)) = (l.overlays.last(), p.overlays.last(), pos)
            && !orect.contains(q)
        {
            if o.close_on_outside {
                p.close_overlay(now);
                p.press = None;
                p.hovered.clear();
            }
            return;
        }
        if let Some((id, k, r)) = pick(&|t| *t == Trigger::MouseDown)
            && p.gesture(id, k, Some(r), now, root)
        {
            return;
        }
        if let Some((id, k, r)) = pick(&|t| *t == Trigger::OnPress)
            && p.fire(id, k, Some(r), now)
        {
            return;
        }
    }
    if down
        && !p.dragged
        && let (Some(start), Some(q)) = (p.press, pos)
        && start.distance(q) > 6.0
    {
        p.dragged = true;
        let at_start: Vec<(NodeId, Rect)> = hot.iter().rev().filter(|(_, r)| r.contains(start)).copied().collect();
        // A directional transition starts only on a drag its way (else the drag may scroll).
        let d = q - start;
        let drags =
            |id: NodeId| doc.node(id)?.interactions.iter().position(|it| it.trigger == Trigger::OnDrag && it.transition.accepts_drag(d.x, d.y));
        if let Some((id, k, r)) = at_start.iter().find_map(|(id, r)| drags(*id).map(|k| (*id, k, *r)))
            && p.fire(id, k, Some(r), now)
        {
            p.press = None;
            // The transition it started follows the pointer: along its direction, else the drag's
            // main axis, completing over the screen's size that way.
            if let Some(a) = p.anim.as_mut().filter(|a| a.start == now) {
                let (x, y) = a
                    .transition
                    .direction()
                    .map_or_else(|| if d.x.abs() >= d.y.abs() { (d.x.signum(), 0.0) } else { (0.0, d.y.signum()) }, direction_vec);
                let len = if x != 0.0 { l.screen.width() } else { l.screen.height() };
                a.follow = Some((start, vec2(x, y), len.max(1.0)));
            }
            return;
        }
        // A drag begun inside a mobile screen moves its content, even if the pointer leaves the
        // screen. The device body stays in place even when the drag begins outside its screen.
        // Panning the whole view remains available only without a mobile device.
        if let Some(base) = p.doc.clone() {
            let horizontal = d.x.abs() >= d.y.abs();
            let scrolls = scrollers_at(p, &base, doc, root, &place, start);
            let on_axis = |range: (f64, f64)| if horizontal { range.0 > 0.0 } else { range.1 > 0.0 };
            let mobile = page_settings(&base, root).device().is_some_and(|device| device.rotates);
            let target = scrolls
                .iter()
                .copied()
                .find(|f| on_axis(p.range(&base, *f)))
                .or_else(|| scrolls.first().copied())
                .map(Scroller::Frame)
                .or((!mobile && p.pan_range != (0.0, 0.0)).then_some(Scroller::View));
            if let Some(t) = target {
                p.fling = None;
                p.drag_scroll = Some((t, clamp_scroll(p.offset(t), p.limits(&base, t))));
            }
        }
    }
    // Dragging scrolls with the pointer; past the ends it stretches.
    if down
        && let (Some((t, from)), Some(start), Some(q)) = (p.drag_scroll, p.press, pos)
        && let Some(base) = p.doc.as_ref()
    {
        let (d, u) = (q - start, units_per_point(t, l.scale));
        let r = p.limits(base, t);
        let limit = RUBBER_PTS * u;
        let to = (rubber_band(from.0 - f64::from(d.x) * u, r.0, limit), rubber_band(from.1 - f64::from(d.y) * u, r.1, limit));
        p.set_offset(t, to);
        p.scroll_anim = None;
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    if released && p.press.is_some() {
        let dragged = p.dragged;
        p.press = None;
        p.dragged = false;
        p.drag_scroll = None;
        if let Some((id, k, r)) = pick(&|t| *t == Trigger::MouseUp)
            && p.gesture(id, k, Some(r), now, root)
        {
            return;
        }
        if !dragged {
            if let Some((id, k, r)) = pick(&|t| *t == Trigger::OnClick)
                && p.fire(id, k, Some(r), now)
            {
                return;
            }
            // A widget's clickable part calls its plugin back (after the frame, with the session).
            if let Some(q) = pos
                && let Some(t) = layer_at(doc, root, &place, q).and_then(|hit| figcraft_engine::widgets::target_at(doc, hit, place.page(q)))
            {
                p.widget_clicks.push(t);
                return;
            }
            // Clicking where nothing reacts flashes the hotspots.
            if p.view.hotspot_hints {
                p.flash_until = now + FLASH_SECS;
            }
        }
    }
    // Controller buttons: pressed alone, anywhere on the screen, like keys.
    for name in &pads {
        let found = doc.subtree(root).into_iter().find_map(|id| find_trigger(doc, id, |t| t.fires_on_key(name, KeyMods::default())).map(|k| (id, k)));
        if let Some((id, k)) = found
            && p.fire(id, k, None, now)
        {
            return;
        }
    }
    // Keys.
    for (key, mods) in &keys {
        let (name, held) = (key.name(), KeyMods { ctrl: mods.ctrl, alt: mods.alt, shift: mods.shift, cmd: mods.mac_cmd });
        let found = doc.subtree(root).into_iter().find_map(|id| find_trigger(doc, id, |t| t.fires_on_key(name, held)).map(|k| (id, k)));
        if let Some((id, k)) = found {
            if p.fire(id, k, None, now) {
                consume_key(ctx, *key);
                return;
            }
            continue;
        }
        // The player's own keys, when the prototype doesn't use them: R restarts, Z cycles the
        // full-window presenter's size options.
        if mods.is_none() {
            match key {
                egui::Key::R => {
                    p.restart(p.start, now);
                    consume_key(ctx, *key);
                    return;
                }
                // (A responsive frame filling the window has no size options.)
                egui::Key::Z if !pane && (l.device.is_some() || p.fit.is_none()) => {
                    p.set_scale(p.view.scale.next(l.device.is_some()));
                    consume_key(ctx, *key);
                    return;
                }
                _ => {}
            }
        }
        // The arrows (and Space or N forward) step through top-level frames when nothing else
        // uses them.
        let forward = match key {
            egui::Key::ArrowRight => Some(true),
            egui::Key::Space | egui::Key::N if mods.is_none() => Some(true),
            egui::Key::ArrowLeft => Some(false),
            _ => None,
        };
        if let Some(forward) = forward
            && p.overlays.is_empty()
            && let Some(base) = p.doc.as_ref()
        {
            let frames = base.top_frames(base.page_of(p.current).unwrap_or(p.current));
            if let Some(i) = frames.iter().position(|f| *f == p.current) {
                let next = if forward { frames.get(i + 1) } else { i.checked_sub(1).and_then(|j| frames.get(j)) };
                if let Some(n) = next.copied() {
                    p.go(n, Transition::Instant, 0.0, Easing::Linear, now, true, false);
                    consume_key(ctx, *key);
                    return;
                }
            }
        }
    }
    // After delay (timers start when the frame appears; not while animating).
    if p.anim.is_none()
        && let Some(((id, k), due)) = next_delay(p, doc, root, now)
        && due <= 0.0
    {
        p.delays_fired.insert((id, k));
        p.fire(id, k, None, now);
    }
}

fn draw(p: &mut Presenter, painter: &egui::Painter, ctx: &egui::Context, l: &Layout, now: f64) {
    let ppp = f64::from(ctx.pixels_per_point());
    let px = l.scale * ppp;
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    let to_tex = p.texture(ctx, &l.disp, p.current, px, now);
    // Transition progress.
    let mut drawn = false;
    if let Some(a) = &p.anim {
        let raw = ((now - a.start) / a.dur.max(1e-3)).clamp(0.0, 1.0);
        if raw >= 1.0 {
            p.anim = None;
        } else {
            ctx.request_repaint();
            let e = a.easing.eval(raw);
            let from_place = a.from_doc.world_bounds(a.from_frame).map(|fb| Place::centered(fb, l.screen.center(), l.scale));
            if matches!(a.transition, Transition::SmartAnimate) {
                #[cfg(not(target_arch = "wasm32"))]
                let t0 = std::time::Instant::now();
                let tmp = smart_interpolate(&a.from_doc, a.from_frame, &l.disp, p.current, e);
                let bounds = figcraft_render::export_bounds(&tmp, p.current);
                if let Some(b) = bounds
                    && let Some(pix) = figcraft_render::render_node(&tmp, p.current, texture_scale(ctx, b, px))
                {
                    let img = crate::render_worker::screen_image(&pix, tmp.color_profile);
                    let tex = match &mut p.anim_tex {
                        Some(t) => {
                            t.set(img, egui::TextureOptions::LINEAR);
                            t.clone()
                        }
                        None => {
                            let t = ctx.load_texture("present-anim", img, egui::TextureOptions::LINEAR);
                            p.anim_tex = Some(t.clone());
                            t
                        }
                    };
                    painter.with_clip_rect(l.screen).image(tex.id(), l.place.rect(b), uv, Color32::WHITE);
                    drawn = true;
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    p.render_ms = t0.elapsed().as_secs_f64() * 1000.0;
                }
            } else if let (Some(ft), Some(to), Some(fp)) = (&a.from_tex, &to_tex, from_place) {
                let f = transition_frames(a.transition, e as f32);
                let from_rect = fp.rect(ft.bounds);
                let from_frame_rect = a.from_doc.world_bounds(a.from_frame).map(|b| fp.rect(b)).unwrap_or(from_rect);
                let to_rect = l.place.rect(to.bounds);
                let off = |r: Rect, o: (f32, f32)| vec2(r.width() * o.0, r.height() * o.1);
                // Without a device a dissolve shows both frames whole; on one, the screen clips.
                let dissolve_both = l.device.is_none() && matches!(a.transition, Transition::Dissolve);
                let clip = painter.with_clip_rect(if dissolve_both { l.screen.union(from_frame_rect) } else { l.screen });
                let draw_from = |p: &egui::Painter| {
                    if f.from_alpha > 0.0 {
                        p.image(ft.tex.id(), from_rect.translate(off(from_frame_rect, f.from_off)), uv, Color32::WHITE.gamma_multiply(f.from_alpha));
                    }
                };
                let draw_to = |p: &egui::Painter| {
                    p.image(to.tex.id(), to_rect.translate(off(l.screen, f.to_off)), uv, Color32::WHITE.gamma_multiply(f.to_alpha));
                };
                if f.from_on_top {
                    draw_to(&clip);
                    draw_from(&clip);
                } else {
                    draw_from(&clip);
                    draw_to(&clip);
                }
                drawn = true;
            }
        }
    }
    if !drawn && let Some(to) = &to_tex {
        painter.with_clip_rect(l.clip).image(to.tex.id(), l.place.rect(to.bounds), uv, Color32::WHITE);
    }
    // Overlays (each with its background and opening transition); closing ones run it backwards.
    let mut ovs: Vec<(f64, OverlayState)> =
        p.overlays.iter().map(|o| (if o.dur > 0.0 { ((now - o.opened) / o.dur).clamp(0.0, 1.0) } else { 1.0 }, o.clone())).collect();
    let mut placed: Vec<(NodeId, Place, Rect)> = l.overlays.clone();
    for ((at, o), pl) in p.closing.iter().zip(l.closing.iter()) {
        if now - at >= o.dur {
            continue;
        }
        ovs.insert(0, (1.0 - ((now - at) / o.dur.max(1e-3)).clamp(0.0, 1.0), o.clone()));
        placed.insert(0, *pl);
    }
    let on_screen = painter.with_clip_rect(l.screen);
    for ((raw, o), (id, place, orect)) in ovs.iter().zip(placed.iter()) {
        let raw = *raw;
        if raw < 1.0 {
            ctx.request_repaint();
        }
        let e = o.easing.eval(raw) as f32;
        let f = if matches!(o.transition, Transition::Instant) {
            transition_frames(Transition::Dissolve, 1.0)
        } else {
            transition_frames(o.transition, e)
        };
        if let Some(bgc) = o.background {
            let c = crate::theme::doc_c32(ctx, bgc);
            on_screen.rect_filled(l.screen, CornerRadius::ZERO, c.gamma_multiply(raw as f32));
        }
        if let Some(tex) = p.texture(ctx, &l.disp, *id, px, now) {
            let r = place.rect(tex.bounds).translate(vec2(orect.width() * f.to_off.0, orect.height() * f.to_off.1));
            on_screen.image(tex.tex.id(), r, uv, Color32::WHITE.gamma_multiply(f.to_alpha));
        }
    }
    // Finished closing animations go (after drawing, so they stayed paired with the layout).
    p.closing.retain(|(at, o)| now - at < o.dur);
    // Hotspot flash after a click on nothing.
    if now < p.flash_until {
        let (root, place, clip) = l.active(p.current);
        let a = ((p.flash_until - now) / FLASH_SECS) as f32;
        for (_, r) in hotspots(&l.disp, root, &place, clip) {
            painter.rect_filled(r, CornerRadius::same(4), ACCENT.gamma_multiply(0.25 * a));
            painter.rect_stroke(r, CornerRadius::same(4), Stroke::new(1.5, ACCENT.gamma_multiply(a)), StrokeKind::Outside);
        }
        ctx.request_repaint();
    }
}

#[cfg(test)]
#[path = "present_variables_tests.rs"]
mod variables_tests;

#[cfg(test)]
#[path = "present_behaviour_tests.rs"]
mod behaviour_tests;

#[cfg(test)]
#[path = "present_tail_tests.rs"]
mod tail_tests;

#[cfg(test)]
#[path = "present_delay_tests.rs"]
mod delay_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use figcraft_doc::NodeKind;

    use crate::present_tests::{Ui, id};

    #[test]
    fn device_picker_reflows_without_resizing_the_saved_frame_and_can_revert() {
        let mut session = figcraft_engine::Session::new();
        let frame = id(&session.execute("node.create", &json!({"type": "frame", "width": 894, "height": 844})).unwrap());
        let right = id(&session
            .execute("node.create", &json!({"type": "rectangle", "parent": frame, "x": 804, "y": 20, "width": 80, "height": 40}))
            .unwrap());
        session.execute("node.set", &json!({"id": right, "props": {"constraints": {"horizontal": "right", "vertical": "top"}}})).unwrap();
        let mut ui = Ui::new(session);
        ui.run("proto.present", json!({"start": frame}));

        let mut presenter = ui.app.ui.present.take().unwrap();
        presenter.pan = Some((100.0, 200.0));
        presenter.scroll.insert(frame, (75.0, 50.0));
        choose_device(&mut ui.app, &mut presenter, "iPhone 17");
        assert!(presenter.pan.is_none() && presenter.scroll.is_empty());
        ui.app.ui.present = Some(presenter);
        ui.frame(vec![]);

        let shown = &ui.app.ui.present.as_ref().unwrap().doc.as_ref().unwrap();
        assert_eq!((shown.node(frame).unwrap().width, shown.node(frame).unwrap().height), (402.0, 874.0));
        assert_eq!(shown.world_bounds(right).unwrap().x0, 312.0, "right constraint follows the phone edge");
        assert_eq!(ui.app.inspect()["presenter"]["viewport"], json!([402.0, 874.0]));
        assert_eq!(ui.app.inspect()["presenter"]["scrollRange"], json!([0.0, 0.0]));
        assert_eq!(ui.app.session.doc().unwrap().node(frame).unwrap().width, 894.0, "the saved design keeps its geometry");

        let area = Rect::from_min_max(pos2(0.0, 40.0), pos2(1440.0, 900.0));
        let screen_before = layout(ui.app.ui.present.as_mut().unwrap(), area, FULL).unwrap().screen;
        ui.button(pos2(720.0, 440.0), true);
        ui.frame(vec![egui::Event::PointerMoved(pos2(300.0, 300.0))]);
        ui.button(pos2(300.0, 300.0), false);
        assert_eq!(ui.app.inspect()["presenter"]["scroll"], json!([0.0, 0.0]), "fitted content does not move");
        assert_eq!(layout(ui.app.ui.present.as_mut().unwrap(), area, FULL).unwrap().screen, screen_before);

        let mut presenter = ui.app.ui.present.take().unwrap();
        choose_device(&mut ui.app, &mut presenter, "None");
        ui.app.ui.present = Some(presenter);
        ui.frame(vec![]);
        assert_eq!(ui.app.inspect()["presenter"]["frameSize"], json!([894.0, 844.0]));
        assert_eq!(ui.app.inspect()["presenter"]["responsive"], Value::Null);
    }

    #[test]
    fn device_presets_show_their_screen_at_the_real_resolution_in_their_body() {
        let mut session = figcraft_engine::Session::new();
        let frame = id(&session.execute("node.create", &json!({"type": "frame", "width": 894, "height": 844})).unwrap());
        let mut ui = Ui::new(session);
        ui.run("proto.present", json!({"start": frame}));
        let area = Rect::from_min_max(pos2(0.0, 40.0), pos2(1440.0, 860.0));

        for (name, width, height) in
            [("iPad mini", 744.0, 1133.0), ("MacBook Air", 1280.0, 832.0), ("Desktop", 1440.0, 1024.0), ("iPhone 17", 402.0, 874.0)]
        {
            let mut presenter = ui.app.ui.present.take().unwrap();
            choose_device(&mut ui.app, &mut presenter, name);
            ui.app.ui.present = Some(presenter);
            ui.frame(vec![]);

            let presenter = ui.app.ui.present.as_mut().unwrap();
            let laid = layout(presenter, area, FULL).unwrap();
            assert!(laid.color.is_some(), "{name}: drawn in its body");
            assert!((f64::from(laid.screen.width()) / laid.scale - width).abs() < 0.1, "{name}: width");
            assert!((f64::from(laid.screen.height()) / laid.scale - height).abs() < 0.1, "{name}: height");
            assert_eq!(ui.app.session.doc().unwrap().node(frame).unwrap().width, 894.0);
        }
    }

    /// A phone shows a frame's content past its screen only as far as the frame's overflow lets it
    /// scroll: with "No scrolling" it's clipped, like on every other device.
    #[test]
    fn a_phone_scrolls_only_what_the_frames_overflow_allows() {
        let on_phone = |overflow: &str| {
            let mut session = figcraft_engine::Session::new();
            let frame = id(&session.execute("node.create", &json!({"type": "frame", "width": 894, "height": 844})).unwrap());
            session.execute("node.create", &json!({"type": "rectangle", "parent": frame, "x": 650, "y": 1050, "width": 80, "height": 80})).unwrap();
            session.execute("node.set", &json!({"id": frame, "props": {"overflow": overflow}})).unwrap();
            let mut ui = Ui::new(session);
            ui.run("proto.present", json!({"start": frame}));
            let mut presenter = ui.app.ui.present.take().unwrap();
            choose_device(&mut ui.app, &mut presenter, "iPhone 17");
            ui.app.ui.present = Some(presenter);
            ui.frame(vec![]);
            (ui, frame)
        };
        let start = pos2(720.0, 440.0);
        let drag = |ui: &mut Ui, to: Pos2| {
            ui.button(start, true);
            ui.frame(vec![egui::Event::PointerMoved(to)]);
            ui.button(to, false);
            ui.app.inspect()["presenter"].clone()
        };

        let (mut ui, frame) = on_phone("none");
        assert_eq!(ui.app.inspect()["presenter"]["scrollRange"], json!([0.0, 0.0]));
        assert_eq!(drag(&mut ui, start - vec2(100.0, 120.0))["scroll"], json!([0.0, 0.0]), "no scrolling stays clipped");
        assert_eq!(ui.app.session.doc().unwrap().node(frame).unwrap().width, 894.0);

        let (mut ui, frame) = on_phone("both");
        let range = &ui.app.inspect()["presenter"]["scrollRange"];
        assert!(range[0].as_f64().unwrap() > 0.0 && range[1].as_f64().unwrap() > 0.0, "{range}");
        let scroll = drag(&mut ui, start - vec2(100.0, 120.0))["scroll"].clone();
        assert!(scroll[0].as_f64().unwrap() > 0.0 && scroll[1].as_f64().unwrap() > 0.0, "{scroll}");
        assert_eq!(ui.app.session.doc().unwrap().node(frame).unwrap().width, 894.0);

        // The content keeps following a drag after the pointer leaves the screen, while the
        // device itself stays fixed on the stage.
        ui.frames(60, Vec::new);
        let area = Rect::from_min_max(pos2(0.0, 40.0), pos2(1440.0, 900.0));
        let screen_before = layout(ui.app.ui.present.as_mut().unwrap(), area, FULL).unwrap().screen;
        ui.button(start, true);
        ui.frame(vec![egui::Event::PointerMoved(pos2(300.0, 300.0))]);
        let during = ui.app.inspect()["presenter"].clone();
        assert!(during["scroll"][0].as_f64().unwrap() > 0.0, "{during}");
        assert_eq!(layout(ui.app.ui.present.as_mut().unwrap(), area, FULL).unwrap().screen, screen_before);
        ui.button(pos2(300.0, 300.0), false);
    }

    fn doc_with_two_frames() -> (Document, NodeId, NodeId) {
        let mut d = Document::new("t");
        let page = d.add_page("Page 1");
        let a = d.create(page, NodeKind::Frame, "A", 0.0, 0.0, 200.0, 400.0).unwrap();
        let b = d.create(page, NodeKind::Frame, "B", 300.0, 0.0, 200.0, 400.0).unwrap();
        (d, a, b)
    }

    #[test]
    fn closed_overlays_animate_out_unless_instant() {
        let (_, a, b) = doc_with_two_frames();
        let mut p = Presenter::new(a);
        let ov = |transition, opened| OverlayState {
            id: b,
            position: OverlayPosition::Center,
            close_on_outside: true,
            background: None,
            anchor: None,
            offset: None,
            opened,
            transition,
            dur: 0.3,
            easing: Easing::EaseOut,
        };
        p.overlays.push(ov(Transition::Dissolve, 0.0));
        assert!(p.close_overlay(10.0));
        assert!(p.overlays.is_empty());
        assert_eq!(p.closing.len(), 1);
        assert_eq!(p.closing[0].0, 10.0, "fully open: the reverse starts now");
        // Closed half way through opening: it reverses from there.
        p.overlays.push(ov(Transition::Dissolve, 20.0));
        p.close_overlay(20.15);
        assert!((p.closing[1].0 - 20.0).abs() < 1e-9);
        // Instant overlays just go.
        p.overlays.push(ov(Transition::Instant, 30.0));
        p.close_overlay(31.0);
        assert_eq!(p.closing.len(), 2);
        assert!(!p.close_overlay(32.0), "nothing left to close");
    }

    #[test]
    fn matching_by_name_path() {
        let (mut d, a, b) = doc_with_two_frames();
        let ga = d.create(a, NodeKind::Group, "Card", 0.0, 0.0, 10.0, 10.0).unwrap();
        let ra = d.create(ga, NodeKind::Rectangle, "Box", 0.0, 0.0, 10.0, 10.0).unwrap();
        let ra2 = d.create(ga, NodeKind::Rectangle, "Box", 0.0, 0.0, 10.0, 10.0).unwrap();
        let only_a = d.create(a, NodeKind::Rectangle, "Gone", 0.0, 0.0, 10.0, 10.0).unwrap();
        let gb = d.create(b, NodeKind::Group, "Card", 0.0, 0.0, 10.0, 10.0).unwrap();
        let rb = d.create(gb, NodeKind::Rectangle, "Box", 0.0, 0.0, 10.0, 10.0).unwrap();
        let rb2 = d.create(gb, NodeKind::Rectangle, "Box", 0.0, 0.0, 10.0, 10.0).unwrap();
        // Same name, different parent: no match.
        let _other = d.create(b, NodeKind::Rectangle, "Box", 0.0, 0.0, 10.0, 10.0).unwrap();
        let m = match_layers(&d, a, &d, b);
        assert_eq!(m, vec![(ga, gb), (ra, rb), (ra2, rb2)]);
        assert!(!m.iter().any(|(f, _)| *f == only_a));
        assert_eq!(name_path(&d, a, ra).as_deref(), Some("Card\u{1f}Box"));
        assert_eq!(name_path(&d, b, ra), None);
    }

    #[test]
    fn smart_interpolate_tweens_matched_and_fades_the_rest() {
        let (mut d, a, b) = doc_with_two_frames();
        let ra = d.create(a, NodeKind::Rectangle, "Box", 0.0, 0.0, 20.0, 20.0).unwrap();
        let rb = d.create(b, NodeKind::Rectangle, "Box", 100.0, 200.0, 60.0, 40.0).unwrap();
        {
            let n = d.get_mut(ra).unwrap();
            n.fills = vec![Paint::solid(Color::new(1.0, 0.0, 0.0, 1.0))];
            n.corners.radii = [0.0; 4];
            n.opacity = 1.0;
        }
        {
            let n = d.get_mut(rb).unwrap();
            n.fills = vec![Paint::solid(Color::new(0.0, 0.0, 1.0, 1.0))];
            n.corners.radii = [20.0; 4];
            n.opacity = 0.5;
            // Rotate 90° about its centre.
            let c = (130.0, 220.0);
            n.transform = Affine::translate(c) * Affine::rotate(std::f64::consts::FRAC_PI_2) * Affine::translate((-30.0, -20.0));
        }
        let only_b = d.create(b, NodeKind::Rectangle, "New", 0.0, 0.0, 5.0, 5.0);
        let only_a = d.create(a, NodeKind::Rectangle, "Old", 0.0, 0.0, 5.0, 5.0).unwrap();
        let mid = smart_interpolate(&d, a, &d, b, 0.5);
        let n = mid.node(rb).unwrap();
        assert!((n.width - 40.0).abs() < 1e-9 && (n.height - 30.0).abs() < 1e-9);
        assert!((n.opacity - 0.75).abs() < 1e-6);
        assert!((n.corners.radii[0] - 10.0).abs() < 1e-9);
        let c = n.fills[0].solid_color().unwrap();
        assert!((c.r - 0.5).abs() < 0.02 && (c.b - 0.5).abs() < 0.02, "{c:?}");
        // Centre halfway between (10, 10) and (130, 220) in frame space; rotated 45°.
        let ctr = mid.world_transform(b).inverse() * mid.world_transform(rb) * Point::new(n.width / 2.0, n.height / 2.0);
        assert!((ctr.x - 70.0).abs() < 1e-6 && (ctr.y - 115.0).abs() < 1e-6, "{ctr:?}");
        let [aa, bb, ..] = n.transform.as_coeffs();
        assert!((bb.atan2(aa) - std::f64::consts::FRAC_PI_4).abs() < 1e-6);
        if let Ok(ob) = only_b {
            assert!((mid.node(ob).unwrap().opacity - 0.5).abs() < 1e-6);
        }
        // The source-only layer is copied into the destination frame, half faded.
        let copies: Vec<_> = mid.node(b).unwrap().children.iter().filter_map(|c| mid.node(*c)).filter(|n| n.name == "Old").collect();
        assert_eq!(copies.len(), 1);
        assert!((copies[0].opacity - 0.5).abs() < 1e-6);
        assert_ne!(copies[0].id, only_a);
        // At the end it is exactly the destination.
        let end = smart_interpolate(&d, a, &d, b, 1.0);
        assert_eq!(end.node(rb), d.node(rb));
        assert_eq!(end.node(b).unwrap().children, d.node(b).unwrap().children);
        // At the start the matched layer sits where the source was.
        let start = smart_interpolate(&d, a, &d, b, 0.0);
        let s = start.world_bounds(rb).unwrap();
        let f = start.world_bounds(b).unwrap();
        assert!((s.x0 - f.x0).abs() < 1e-6 && (s.y0 - f.y0).abs() < 1e-6 && (s.width() - 20.0).abs() < 1e-6);
    }

    #[test]
    fn scroll_offsets_fixed_and_sticky() {
        let (mut d, a, _) = doc_with_two_frames();
        d.get_mut(a).unwrap().overflow = Overflow::Vertical;
        let long = d.create(a, NodeKind::Rectangle, "Long", 0.0, 0.0, 200.0, 1000.0).unwrap();
        let fixed = d.create(a, NodeKind::Rectangle, "Bar", 0.0, 350.0, 200.0, 50.0).unwrap();
        let sticky = d.create(a, NodeKind::Rectangle, "Header", 0.0, 100.0, 200.0, 40.0).unwrap();
        d.get_mut(fixed).unwrap().scroll_position = ScrollPosition::Fixed;
        d.get_mut(sticky).unwrap().scroll_position = ScrollPosition::Sticky;
        assert_eq!(scroll_range(&d, a), (0.0, 600.0));
        d.get_mut(a).unwrap().overflow = Overflow::Horizontal;
        assert_eq!(scroll_range(&d, a), (0.0, 0.0));
        d.get_mut(a).unwrap().overflow = Overflow::Vertical;
        let mut s = HashMap::new();
        s.insert(a, (0.0, 50.0));
        let out = apply_scroll(&d, &s);
        let y = |doc: &Document, id| doc.world_bounds(id).unwrap().y0;
        assert_eq!(y(&out, long), -50.0);
        assert_eq!(y(&out, fixed), 350.0);
        assert_eq!(y(&out, sticky), 50.0);
        s.insert(a, (0.0, 300.0));
        let out = apply_scroll(&d, &s);
        assert_eq!(y(&out, long), -300.0);
        assert_eq!(y(&out, sticky), 0.0);
    }

    /// A page presented on an iPhone 16 (393×852): a tall screen A that scrolls vertically, with a
    /// tab bar fixed to its bottom, a nested scrolling list and a layer at its end; screen B.
    fn device_doc() -> (Document, NodeId, NodeId, NodeId, NodeId, NodeId) {
        let mut d = Document::new("t");
        let page = d.add_page("Page 1");
        if let NodeKind::Page { prototype, .. } = &mut d.get_mut(page).unwrap().kind {
            prototype.device = "iPhone 16".into();
        }
        let a = d.create(page, NodeKind::Frame, "A", 0.0, 0.0, 393.0, 2000.0).unwrap();
        let b = d.create(page, NodeKind::Frame, "B", 500.0, 0.0, 393.0, 1500.0).unwrap();
        for f in [a, b] {
            d.get_mut(f).unwrap().overflow = Overflow::Vertical;
        }
        let bar = d.create(a, NodeKind::Rectangle, "Tab bar", 0.0, 1940.0, 393.0, 60.0).unwrap();
        let n = d.get_mut(bar).unwrap();
        n.scroll_position = ScrollPosition::Fixed;
        n.constraints.vertical = VConstraint::Bottom;
        let list = d.create(a, NodeKind::Frame, "List", 0.0, 1200.0, 393.0, 300.0).unwrap();
        d.get_mut(list).unwrap().overflow = Overflow::Vertical;
        let item = d.create(list, NodeKind::Rectangle, "Item", 0.0, 700.0, 393.0, 100.0).unwrap();
        (d, a, b, bar, list, item)
    }

    fn player_on(d: Document, start: NodeId) -> Presenter {
        let mut p = Presenter::new(start);
        p.sync(&Arc::new(d), 0.0);
        p
    }

    #[test]
    fn device_screen_clips_the_frame_and_it_scrolls_through_its_height() {
        let (mut d, a, _, bar, list, _) = device_doc();
        assert_eq!(viewport(&d, a), Some((393.0, 852.0)));
        // On the device the whole frame scrolls; at its own size nothing overflows it.
        assert_eq!(scroll_range_in(&d, a, viewport(&d, a)), (0.0, 1148.0));
        assert_eq!(scroll_range(&d, a), (0.0, 0.0));
        // Nested frames scroll within themselves.
        assert_eq!(scroll_range(&d, list), (0.0, 500.0));
        // The display copy is sized to the screen; the fixed tab bar moves to the screen's bottom.
        let mut p = player_on(d.clone(), a);
        let disp = p.display().unwrap();
        assert_eq!((disp.node(a).unwrap().width, disp.node(a).unwrap().height), (393.0, 852.0));
        assert_eq!(disp.world_bounds(bar).unwrap().y0, 792.0);
        // Scrolled, it stays there while the content moves.
        p.scroll_to(0.0, 400.0);
        let disp = p.display().unwrap();
        assert_eq!(disp.world_bounds(bar).unwrap().y0, 792.0);
        assert_eq!(disp.world_bounds(list).unwrap().y0, 800.0);
        p.scroll_to(0.0, 99_999.0);
        assert_eq!(p.scroll[&a], (0.0, 1148.0), "clamped to the range");
        // Without scrolling the screen only clips; without a device the frame shows whole.
        d.get_mut(a).unwrap().overflow = Overflow::None;
        assert_eq!(scroll_range_in(&d, a, viewport(&d, a)), (0.0, 0.0));
        if let Some(pg) = d.page_of(a)
            && let NodeKind::Page { prototype, .. } = &mut d.get_mut(pg).unwrap().kind
        {
            prototype.device = "None".into();
        }
        assert_eq!(viewport(&d, a), None);
        // A frame wider than the screen scrolls sideways when it scrolls both ways.
        let (mut d, a, ..) = device_doc();
        let n = d.get_mut(a).unwrap();
        n.width = 800.0;
        n.overflow = Overflow::Both;
        assert_eq!(scroll_range_in(&d, a, viewport(&d, a)), (407.0, 1148.0));
    }

    #[test]
    fn scroll_to_moves_every_scrolling_frame_around_the_layer() {
        let (d, a, _, _, list, item) = device_doc();
        let mut p = player_on(d.clone(), a);
        assert!(p.perform(&Action::ScrollTo { dest: item, offset: [0.0, 0.0] }, item, Transition::Instant, 0.0, Easing::Linear, None, 1.0));
        // The list brings the item to its top (as far as it scrolls), the screen brings the list.
        assert_eq!(p.scroll[&list], (0.0, 500.0));
        assert_eq!(p.scroll[&a], (0.0, 1148.0));
        // Animated, it gets there over the duration.
        let mut p = player_on(d, a);
        assert!(p.perform(&Action::ScrollTo { dest: list, offset: [0.0, 0.0] }, list, Transition::Dissolve, 0.3, Easing::Linear, None, 1.0));
        let anim = p.scroll_anim.as_ref().unwrap();
        assert_eq!(anim.frames, vec![(a, (0.0, 0.0), (0.0, 1148.0))]);
    }

    #[test]
    fn navigating_resets_scroll_unless_it_preserves_it() {
        let (d, a, b, ..) = device_doc();
        let mut p = player_on(d, a);
        p.scroll_to(0.0, 600.0);
        assert!(p.perform(&Action::Navigate { dest: b, preserve_scroll: false }, a, Transition::Instant, 0.0, Easing::Linear, None, 1.0));
        assert_eq!(p.scroll.get(&b), None, "starts at the top");
        p.restart(a, 2.0);
        p.scroll_to(0.0, 1000.0);
        assert!(p.perform(&Action::Navigate { dest: b, preserve_scroll: true }, a, Transition::Instant, 0.0, Easing::Linear, None, 3.0));
        assert_eq!(p.scroll[&b], (0.0, 648.0), "kept, as far as B scrolls");
        // Back starts at the top again.
        assert!(p.perform(&Action::Back, b, Transition::Instant, 0.0, Easing::Linear, None, 4.0));
        assert_eq!((p.current, p.scroll.get(&a)), (a, None));
    }

    #[test]
    fn sync_follows_edits_and_keeps_the_screen() {
        let (mut d, a, b, _, list, _) = device_doc();
        let mut p = player_on(d.clone(), a);
        p.go(b, Transition::Instant, 0.0, Easing::Linear, 1.0, true, false);
        p.scroll_to(0.0, 600.0);
        // The same document again changes nothing.
        let same = Arc::new(d.clone());
        p.sync(&same, 2.0);
        let rev = p.rev;
        p.sync(&same, 2.0);
        assert_eq!(p.rev, rev);
        // An edit is copied; the screen and its scroll stay (clamped to the new range).
        d.get_mut(b).unwrap().height = 1000.0;
        d.get_mut(b).unwrap().name = "B2".into();
        p.sync(&Arc::new(d.clone()), 3.0);
        assert_eq!(p.current, b);
        assert_eq!(p.inspect()["name"], "B2");
        assert_eq!(p.scroll[&b], (0.0, 148.0));
        assert_eq!(p.history.len(), 1);
        // The screen deleted: back to the start.
        d.remove(b).unwrap();
        p.sync(&Arc::new(d.clone()), 4.0);
        assert_eq!(p.current, a);
        assert!(p.history.is_empty() && p.scroll.is_empty());
        // Junk scroll offsets never panic and stay in range.
        p.scroll.insert(list, (f64::NAN, f64::INFINITY));
        p.sync(&Arc::new(d), 5.0);
        assert_eq!(p.scroll[&list], (0.0, 0.0));
    }

    #[test]
    fn transitions_move_the_right_way() {
        let left = transition_frames(Transition::MoveIn { direction: Direction::Left }, 0.0);
        assert_eq!(left.to_off, (1.0, 0.0));
        assert_eq!(transition_frames(Transition::MoveIn { direction: Direction::Left }, 1.0).to_off, (0.0, 0.0));
        let push = transition_frames(Transition::Push { direction: Direction::Top }, 0.5);
        assert_eq!(push.from_off, (0.0, -0.5));
        assert_eq!(push.to_off, (0.0, 0.5));
        let out = transition_frames(Transition::MoveOut { direction: Direction::Right }, 0.25);
        assert!(out.from_on_top);
        assert_eq!(out.from_off, (0.25, 0.0));
        assert_eq!(transition_frames(Transition::Dissolve, 0.3).to_alpha, 0.3);
        assert_eq!(transition_frames(Transition::Instant, 0.0).from_alpha, 0.0);
        let r = Transition::SlideIn { direction: Direction::Bottom }.reversed();
        assert_eq!(r, Transition::SlideOut { direction: Direction::Top });
    }

    #[test]
    fn overlay_positions() {
        let base = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 800.0));
        let size = vec2(200.0, 100.0);
        assert_eq!(overlay_rect(OverlayPosition::Center, base, size, None, None).min, pos2(100.0, 350.0));
        assert_eq!(overlay_rect(OverlayPosition::BottomCenter, base, size, None, None).min, pos2(100.0, 700.0));
        assert_eq!(overlay_rect(OverlayPosition::TopRight, base, size, None, None).min, pos2(200.0, 0.0));
        let anchor = Rect::from_min_size(pos2(30.0, 40.0), vec2(50.0, 20.0));
        assert_eq!(overlay_rect(OverlayPosition::Manual, base, size, Some(anchor), None).min, pos2(30.0, 60.0));
    }

    /// Smart animate renders a temporary document per animation frame; keep that cheap.
    #[test]
    fn smart_animate_frame_cost() {
        let (mut d, a, b) = doc_with_two_frames();
        for (f, dx) in [(a, 0.0), (b, 40.0)] {
            for i in 0..40 {
                let r = d.create(f, NodeKind::Rectangle, &format!("Row {i}"), dx + 10.0, 10.0 + f64::from(i) * 9.0, 120.0, 8.0).unwrap();
                d.get_mut(r).unwrap().corners.radii = [4.0; 4];
            }
        }
        let t0 = std::time::Instant::now();
        let n = 10;
        for k in 0..n {
            let tmp = smart_interpolate(&d, a, &d, b, f64::from(k) / f64::from(n));
            assert!(figcraft_render::render_node(&tmp, b, 2.0).is_some());
        }
        let per = t0.elapsed().as_secs_f64() * 1000.0 / f64::from(n);
        eprintln!("smart animate: {per:.2} ms per frame (80 layers, 400x800 px)");
    }
}
