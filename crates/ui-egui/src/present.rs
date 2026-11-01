//! The prototype player: presents a frame full-window ([`show`]) or in the preview pane floating
//! over the canvas ([`show_pane`]), and follows its interactions. Both run the same [`player`].
//!
//! The player works on its own copy of the document (interactive components, variables and scroll
//! offsets change it without touching the user's file) and copies the editor's document again
//! whenever it changes, so the preview pane follows edits live. On a device, the presented frame
//! is sized to the device's screen ([`fit_viewport`]): a frame bigger than the screen scrolls
//! through its whole size (as in Figma, with no overflow setting), while content past the frame's
//! own edges scrolls only as its overflow allows and is otherwise clipped. A responsive prototype's frame is instead resized to the screen
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
    Action, Device, DeviceColor, Direction, Easing, KeyMods, Overflow, OverlayAnchor, OverlayPosition, PrototypeSettings, ScrollPosition, Transition,
    Trigger,
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

mod actions;
mod chrome;
mod device;
mod draw;
mod input;
mod layout;
mod navigate;
mod player;
mod scroll;
mod transition;

#[cfg(test)]
use chrome::choose_device;
pub use chrome::{Pane, PresentTab, pane_id, show, show_pane};
pub use layout::Place;
#[cfg(test)]
use layout::{FULL, layout};
#[cfg(test)]
use navigate::next_delay;
pub use scroll::{apply_scroll, fit_viewport, fling_axis, page_settings, raise_fixed, rubber_band, scroll_range, scroll_range_in, viewport};
pub use transition::{FramePlacement, direction_vec, lerp_transform, match_layers, name_path, overlay_rect, smart_interpolate, transition_frames};

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
    /// The layer that opened it, in page units of the presented frame (so it follows the frame
    /// when the window resizes or the view pans).
    pub anchor: Option<figcraft_geom::Rect>,
    /// Manual position: the overlay's top-left from the anchor's (page units).
    pub offset: Option<[f64; 2]>,
    pub anchor_to: OverlayAnchor,
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
    /// The links the last frame's actions opened, whether the viewer's own click, press, drag or
    /// key fired each, and whether it opens in a new tab; the frame-end queues them behind the
    /// notice, but for one the viewer fired while choosing to skip the notice (#1096).
    urls: Vec<(String, bool, bool)>,
    /// A link waiting for the viewer to open it ([`link_notice`]), whether "Don't show this
    /// again" is ticked (`None` when the notice doesn't offer it: the viewer didn't fire the link,
    /// so skipping the notice wouldn't cover it, or they skip it already), and whether it opens in
    /// a new tab.
    link_notice: Option<(String, Option<bool>, bool)>,
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
    /// The frame the presentation started from (Restart goes back there).
    pub fn start(&self) -> NodeId {
        self.start
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
            link_notice: None,
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

    /// Screen rect `r` in page units of the frame shown last.
    fn page_rect(&self, r: Option<Rect>) -> Option<figcraft_geom::Rect> {
        let (r, (_, place, _)) = (r?, self.shown?);
        let (a, b) = (place.page(r.min), place.page(r.max));
        Some(figcraft_geom::Rect::new(a.x, a.y, b.x, b.y))
    }
}

/// If no layer on the screen opened the overlay, the screen places it.
pub fn anchored_overlay_rect(
    pos: OverlayPosition,
    anchor_to: OverlayAnchor,
    base: Rect,
    size: Vec2,
    anchor: Option<Rect>,
    offset: Option<Vec2>,
) -> Rect {
    let on_trigger =
        anchor.and_then(|a| anchor_to.offset(pos, [f64::from(a.width()), f64::from(a.height())], [f64::from(size.x), f64::from(size.y)]));
    match on_trigger {
        Some([x, y]) => overlay_rect(OverlayPosition::Manual, base, size, anchor, Some(vec2(x as f32, y as f32))),
        None => overlay_rect(pos, base, size, anchor, offset),
    }
}

#[cfg(test)]
mod variables_tests;

#[cfg(test)]
mod behaviour_tests;

#[cfg(test)]
mod tail_tests;

#[cfg(test)]
mod delay_tests;

#[cfg(test)]
mod overlay_anchor_tests;

#[cfg(test)]
mod tests;
