//! Motion (Beta) mode: the animation timeline under the canvas.
//!
//! State shared by the timeline and its future inspector lives in [`MotionUi`]; the timeline itself
//! is [`timeline`]. The keyframe model is `figcraft_doc::anim::motion`.

pub mod clip_popover;
pub mod easing;
pub mod inspector;
pub mod timeline;

use figcraft_doc::NodeId;
use figcraft_doc::anim::motion::AnimProperty;
use std::collections::BTreeSet;

use crate::FigApp;

/// Opens or closes Motion mode, widening the right panel to the inspector while it is open.
pub fn set_open(app: &mut FigApp, open: bool) {
    if open == app.ui.motion_open {
        return;
    }
    if open {
        app.ui.motion_right_width = Some(app.ui.right_width);
        app.ui.right_width = inspector::WIDTH;
        app.ui.motion.playing = false;
    } else if let Some(w) = app.ui.motion_right_width.take() {
        app.ui.right_width = w;
    }
    app.ui.motion_open = open;
}

/// A keyframe the timeline has selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectedKey {
    pub node: NodeId,
    pub property: AnimProperty,
    pub time_ms: u32,
}

/// A property track the timeline has picked. The layer row then stops being the selection block,
/// and only this row keeps a background (the layer-panel behaviour: picking a child drops the
/// parent's block).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectedTrack {
    pub node: NodeId,
    pub property: AnimProperty,
}

/// One selectable item in the Motion timeline. The whole timeline selection is a set of these, so a
/// parent (layer) can carry all its children at once, re-clicking any item toggles just it off, and
/// Shift extends the selection instead of replacing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SelItem {
    /// A layer row — selecting it pulls in every track and clip underneath it.
    Layer(NodeId),
    /// A property (animation track) row.
    Track(NodeId, AnimProperty),
    /// A single keyframe diamond.
    Key(NodeId, AnimProperty, u32),
    /// A segment between two keyframes, identified by its start keyframe's time.
    Segment(NodeId, AnimProperty, u32),
    /// A clip bar.
    Clip(NodeId, u32),
}

impl SelItem {
    /// The layer this item belongs to.
    pub fn node(self) -> NodeId {
        match self {
            SelItem::Layer(n) | SelItem::Track(n, _) | SelItem::Key(n, _, _) | SelItem::Segment(n, _, _) | SelItem::Clip(n, _) => n,
        }
    }
}

/// Which part of a clip bar a drag grabbed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipEdge {
    /// The middle: slide the whole clip (delay only, length unchanged).
    Move,
    /// The left end: move the start, keep the end.
    TrimStart,
    /// The right end: move the end, keep the start.
    TrimEnd,
}

/// A clip drag in progress: the clip as it was when the drag began, and the end grabbed. The clip
/// is recomputed from `orig` plus the pointer's total delta, so frames never accumulate drift.
#[derive(Clone, Debug)]
pub struct ClipDrag {
    pub node: NodeId,
    pub id: u32,
    pub edge: ClipEdge,
    /// Pointer x when the drag began.
    pub origin_x: f32,
    pub orig: figcraft_doc::anim::motion::Clip,
}

/// A layer-summary drag: the layer's whole motion as it was when the drag began, its span, and the
/// end grabbed. `Move` slides everything; the ends time-stretch the clips and keys about the other.
#[derive(Clone, Debug)]
pub struct LayerDrag {
    pub node: NodeId,
    pub edge: ClipEdge,
    pub origin_x: f32,
    pub lo: u32,
    pub hi: u32,
    pub orig: figcraft_doc::anim::motion::NodeMotion,
}

/// What an evaluated preview was built from. The canvas preview is rebuilt only when one of these
/// changes, so the same frame keeps handing the canvas the same document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preview {
    /// The document's revision, which every edit bumps.
    pub revision: u64,
    pub page: NodeId,
    /// The playhead, as raw bits (a float has no `Eq`).
    pub playhead: u64,
}

/// Timeline state kept between frames.
#[derive(Default)]
pub struct MotionUi {
    /// Height of the docked timeline, dragged by its top edge (`timeline::HEIGHT` until then).
    pub timeline_h: f32,
    /// The playhead, in milliseconds (may sit between keyframes).
    pub playhead_ms: f64,
    pub playing: bool,
    /// The full timeline selection: every selectable item currently picked. A parent (layer) carries
    /// all its children, Shift adds to it, and re-clicking any member toggles just it off.
    pub selection: BTreeSet<SelItem>,
    pub selected: Option<SelectedKey>,
    /// The property row picked in the timeline, if one is.
    pub selected_track: Option<SelectedTrack>,
    /// Layers whose property and clip rows are folded away.
    pub folded: BTreeSet<NodeId>,
    /// Vertical scroll of the timeline rows.
    pub scroll_y: f32,
    /// The playhead is being dragged in the ruler (the canvas must keep previewing).
    pub scrubbing: bool,
    /// Live duration while its edge is dragged (committed on release).
    pub drag_duration: Option<u32>,
    /// Horizontal scroll of the track (px). Only the duration drag past the right edge moves it:
    /// the further out the pointer is, the faster it scrolls, so the axis keeps extending.
    pub scroll_x: f32,
    /// The clip row clicked in the timeline, shown in the inspector.
    pub selected_clip: Option<(NodeId, u32)>,
    /// The clip whose floating settings window is open, and the row it came from (the window
    /// opens against it, and a press on it toggles instead of closing). The settings are a
    /// popover over the canvas, not a section of the right panel.
    pub clip_popover: Option<(NodeId, u32)>,
    /// The window itself, kept between frames: its height is only known after a layout, and the
    /// popover places itself on the frame after that.
    pub clip_pop: Option<crate::popover::Popover>,
    pub clip_anchor: Option<egui::Rect>,
    /// The window opened this frame: the press that opened it must not close it again.
    pub clip_just_opened: bool,
    /// The clip drag in progress (recomputed from `orig` each frame).
    pub clip_drag: Option<ClipDrag>,
    /// The layer-summary drag in progress (recomputed from `orig` each frame).
    pub layer_drag: Option<LayerDrag>,
    /// The inspector shows the easing panel for the selected segment.
    pub easing_target: bool,
    /// The page evaluated at the playhead and what it was built from.
    pub preview: Option<(Preview, std::sync::Arc<figcraft_doc::Document>)>,
}

/// A square button drawn as a filled block: a hover/active backing over the whole rect and a
/// smaller block in the middle. Returns the click response.
pub fn block_button(ui: &mut egui::Ui, id: egui::Id, rect: egui::Rect, active: bool, tip: &str) -> egui::Response {
    let t = crate::theme::Tokens::get(ui.ctx());
    let resp = ui.interact(rect, id, egui::Sense::click()).on_hover_text(tip);
    crate::a11y::button(&resp, tip);
    if active || resp.hovered() {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(5), t.hover);
    }
    let inner = egui::Rect::from_center_size(rect.center(), egui::vec2(12.0, 12.0));
    ui.painter().rect_filled(inner, egui::CornerRadius::same(2), if active { t.accent_text } else { t.icon_secondary });
    resp
}
