//! Pointer tools. The UI (or an agent through the control channel) feeds [`PointerEvent`]s in page
//! coordinates; tools turn gestures into document edits through interactions (one undo step per
//! gesture) and publish overlay hints (marquee, guides, size label, cursor) for the canvas.

use std::collections::BTreeMap;

use figcraft_doc::geometry::select_target;
use figcraft_doc::*;
use figcraft_geom::{Affine, Point, Rect, Vec2};
use serde::Serialize;
use serde_json::json;

use crate::arc_handles::{ArcHandle, ArcHint};
use crate::cmd::arrange::move_world;
use crate::cmd::create::frame_at;
use crate::cmd::edit::roots;
use crate::shape_handles::{ShapeHandle, ShapeHint};
use crate::snap::{Guide, Targets};
use crate::{Result, Session, textedit};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Default)]
pub enum ToolId {
    #[default]
    Move,
    Scale,
    Hand,
    Frame,
    Section,
    Slice,
    Rectangle,
    Line,
    Arrow,
    Ellipse,
    Polygon,
    Star,
    Pen,
    Pencil,
    Text,
    Comment,
    Eyedropper,
    ShapeBuilder,
    // Board tools.
    Sticky,
    BoardShape,
    Connector,
    Marker,
    Highlighter,
    Eraser,
    Stamp,
}

impl ToolId {
    pub const ALL: [ToolId; 25] = [
        ToolId::Move,
        ToolId::Scale,
        ToolId::Hand,
        ToolId::Frame,
        ToolId::Section,
        ToolId::Slice,
        ToolId::Rectangle,
        ToolId::Line,
        ToolId::Arrow,
        ToolId::Ellipse,
        ToolId::Polygon,
        ToolId::Star,
        ToolId::Pen,
        ToolId::Pencil,
        ToolId::Text,
        ToolId::Comment,
        ToolId::Eyedropper,
        ToolId::ShapeBuilder,
        ToolId::Sticky,
        ToolId::BoardShape,
        ToolId::Connector,
        ToolId::Marker,
        ToolId::Highlighter,
        ToolId::Eraser,
        ToolId::Stamp,
    ];
    pub fn id(self) -> &'static str {
        match self {
            ToolId::Move => "move",
            ToolId::Scale => "scale",
            ToolId::Hand => "hand",
            ToolId::Frame => "frame",
            ToolId::Section => "section",
            ToolId::Slice => "slice",
            ToolId::Rectangle => "rectangle",
            ToolId::Line => "line",
            ToolId::Arrow => "arrow",
            ToolId::Ellipse => "ellipse",
            ToolId::Polygon => "polygon",
            ToolId::Star => "star",
            ToolId::Pen => "pen",
            ToolId::Pencil => "pencil",
            ToolId::Text => "text",
            ToolId::Comment => "comment",
            ToolId::Eyedropper => "eyedropper",
            ToolId::ShapeBuilder => "shapeBuilder",
            ToolId::Sticky => "sticky",
            ToolId::BoardShape => "shape",
            ToolId::Connector => "connector",
            ToolId::Marker => "marker",
            ToolId::Highlighter => "highlighter",
            ToolId::Eraser => "eraser",
            ToolId::Stamp => "stamp",
        }
    }
    pub fn from_id(s: &str) -> Option<ToolId> {
        ToolId::ALL.into_iter().find(|t| t.id().eq_ignore_ascii_case(s))
    }
    pub fn label(self) -> &'static str {
        match self {
            ToolId::Move => "Move",
            ToolId::Scale => "Scale",
            ToolId::Hand => "Hand tool",
            ToolId::Frame => "Frame",
            ToolId::Section => "Section",
            ToolId::Slice => "Slice",
            ToolId::Rectangle => "Rectangle",
            ToolId::Line => "Line",
            ToolId::Arrow => "Arrow",
            ToolId::Ellipse => "Ellipse",
            ToolId::Polygon => "Polygon",
            ToolId::Star => "Star",
            ToolId::Pen => "Pen",
            ToolId::Pencil => "Pencil",
            ToolId::Text => "Text",
            ToolId::Comment => "Comment",
            ToolId::Eyedropper => "Pick color",
            ToolId::ShapeBuilder => "Shape builder",
            ToolId::Sticky => "Sticky note",
            ToolId::BoardShape => "Shapes",
            ToolId::Connector => "Connector",
            ToolId::Marker => "Marker",
            ToolId::Highlighter => "Highlighter",
            ToolId::Eraser => "Eraser",
            ToolId::Stamp => "Stamp",
        }
    }

    /// The tools of a board's toolbar (a design file's toolbar has the others).
    pub fn on_boards(self) -> bool {
        matches!(
            self,
            ToolId::Sticky | ToolId::BoardShape | ToolId::Connector | ToolId::Marker | ToolId::Highlighter | ToolId::Eraser | ToolId::Stamp
        )
    }
    /// The tool's key (from [`crate::keymap`]; empty when it has none).
    pub fn shortcut(self) -> &'static str {
        let id = self.id();
        crate::keymap::KEYS
            .iter()
            .find(|b| if self == ToolId::Eyedropper { b.id == "edit.pickColor" } else { b.id.strip_prefix("tool.") == Some(id) })
            .and_then(|b| b.keys(crate::keymap::Platform::current()).first().copied())
            .unwrap_or("")
    }

    /// The size a click (or Enter, `tool.place`) makes with a shape or frame tool.
    pub fn click_size(self) -> (f64, f64) {
        match self {
            ToolId::Line | ToolId::Arrow => (100.0, 0.0),
            ToolId::Section => (400.0, 300.0),
            _ => (100.0, 100.0),
        }
    }

    pub fn draws_shape(self) -> bool {
        matches!(
            self,
            ToolId::Frame
                | ToolId::Section
                | ToolId::Slice
                | ToolId::Rectangle
                | ToolId::Ellipse
                | ToolId::Polygon
                | ToolId::Star
                | ToolId::Line
                | ToolId::Arrow
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum PointerKind {
    Down,
    Move,
    Up,
    /// Pointer moved without a button.
    Hover,
    DoubleClick,
}

/// A pointer event in page coordinates (and screen coordinates for panning).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PointerEvent {
    pub kind: PointerKind,
    pub x: f64,
    pub y: f64,
    /// Screen position (logical pixels).
    pub sx: f64,
    pub sy: f64,
    pub shift: bool,
    pub alt: bool,
    /// Cmd on macOS, Ctrl elsewhere.
    pub cmd: bool,
    pub ctrl: bool,
    /// Screen pixels per page unit.
    pub zoom: f64,
    /// Pen pressure, 0..1, where the device reports it (`None`: a mouse, or a platform that doesn't).
    pub pressure: Option<f32>,
}

/// The pointer is still within [`DRAG_SLOP_PX`] of where it was pressed.
fn within_slop(ev: &PointerEvent, start: Point) -> bool {
    (ev.p() - start).hypot() * ev.zoom.max(1e-6) < DRAG_SLOP_PX
}

impl PointerEvent {
    pub fn at(kind: PointerKind, x: f64, y: f64, zoom: f64) -> Self {
        Self { kind, x, y, sx: x * zoom, sy: y * zoom, shift: false, alt: false, cmd: false, ctrl: false, zoom, pressure: None }
    }
    pub(crate) fn p(&self) -> Point {
        Point::new(self.x, self.y)
    }
    fn tol(&self) -> f64 {
        1.0 / self.zoom.max(1e-6)
    }
}

/// Cursor hint for the UI.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Default)]
pub enum Cursor {
    #[default]
    Default,
    Crosshair,
    Move,
    /// Resize along an angle (degrees, 0 = horizontal).
    Resize(f64),
    /// Rotate, turned to the corner it is near: the direction from the box's centre to the corner
    /// (while rotating: to the pointer), in degrees, 0 = right and 90 = down.
    Rotate(f64),
    Grab,
    Grabbing,
    Text,
    Pen(PenCursor),
    /// A drag here makes a copy (Alt).
    Copy,
    /// Over a clickable part of a widget.
    Pointer,
    Eyedropper,
    Comment,
    /// What the gesture is doing will be undone when it ends (an edit only a main component can
    /// make).
    NotAllowed,
}

/// What a click with the pen would do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Default)]
pub enum PenCursor {
    /// A new point.
    #[default]
    New,
    /// On a point: closes the path, connects to the point or goes on from it.
    Close,
    /// On a segment: adds a point there.
    Add,
    /// Bends the segment or the point's handles (the Bend tool; Cmd/Ctrl in vector edit mode).
    Bend,
}

impl ToolId {
    /// The tool's cursor over empty canvas.
    pub fn cursor(self) -> Cursor {
        match self {
            ToolId::Hand => Cursor::Grab,
            ToolId::Pen => Cursor::Pen(PenCursor::New),
            ToolId::Comment => Cursor::Comment,
            ToolId::Eyedropper => Cursor::Eyedropper,
            ToolId::Move | ToolId::Scale => Cursor::Default,
            // The other tools make something where they are pressed: shapes, frames, slices, pencil
            // strokes, a new text (over a text the Text tool shows the I-beam instead).
            _ => Cursor::Crosshair,
        }
    }
}

/// Selection handle kinds: corners (0 tl, 1 tr, 2 br, 3 bl), edges (0 t, 1 r, 2 b, 3 l) and the
/// rotation zones outside the corners.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum Handle {
    Corner(u8),
    Edge(u8),
    Rotate(u8),
}

/// What the canvas should draw on top of the scene.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Overlay {
    pub marquee: Option<[f64; 4]>,
    pub guides: Vec<Guide>,
    pub cursor: Cursor,
    /// Pen tool: the in-progress path's anchor points and the rubber band to the pointer.
    pub pen: Vec<[f64; 2]>,
    pub rubber: Option<[f64; 4]>,
    /// Pen tool: the handles shaping the segments at the pen (anchor x, y, handle x, y): the last
    /// anchor's in and out handles and the one before's out handle.
    pub pen_handles: Vec<[f64; 4]>,
    /// Pen tool: where the last anchor's out handle ends, when it has one: the rubber band is the
    /// curve a click would make, leaving along it.
    pub rubber_handle: Option<[f64; 2]>,
    /// Frame/section the pointer is drawing or dropping into (highlighted).
    pub drop_target: Option<NodeId>,
    /// Auto layout insertion indicator (x0, y0, x1, y1).
    pub insert_line: Option<[f64; 4]>,
    /// The instance an Alt-drop of a component from the Assets panel would swap (highlighted).
    pub swap_target: Option<NodeId>,
    /// Auto layout grid cell a drag would drop into (x0, y0, x1, y1).
    pub drop_cell: Option<[f64; 4]>,
    /// Vector edit mode: what the pointer is over, the lasso being drawn and the closed area the
    /// paint bucket would fill (page-space polygons).
    pub vector_hover: Option<crate::vectoredit::VHit>,
    /// The edited layer `vector_hover` is on (several layers can be in edit mode).
    pub vector_hover_on: Option<NodeId>,
    /// Vector edit mode, Variable width tool: the width point, handle or path under the pointer.
    pub width_hover: Option<crate::vectoredit::WHit>,
    pub lasso: Vec<[f64; 2]>,
    /// The eraser brush under the pointer (x, y, radius in page units).
    pub brush: Option<[f64; 3]>,
    pub face: Vec<Vec<[f64; 2]>>,
    /// Shape builder: the regions under the pointer or collected by the drag (indexes into
    /// [`crate::shapebuilder::current`]) and whether releasing removes them.
    pub builder_regions: Vec<usize>,
    pub builder_remove: bool,
    /// Measuring (Alt): the box of the layer or frame measured to (x0, y0, x1, y1), outlined.
    pub measure_box: Option<[f64; 4]>,
    /// Scale tool drag: the factor so far and the point that stays put.
    pub scale: Option<ScaleHint>,
    /// The selected ellipse's arc handle under the pointer, and the readout while one is dragged
    /// (see [`crate::arc_handles`]).
    pub arc_hover: Option<ArcHandle>,
    pub arc: Option<ArcHint>,
    /// The selected shape's corner radius or star ratio handle under the pointer, and the readout
    /// while one is dragged (see [`crate::shape_handles`]).
    pub shape_hover: Option<ShapeHandle>,
    pub shape: Option<ShapeHint>,
}

/// The Scale tool's live readout while dragging a handle.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ScaleHint {
    pub factor: f64,
    pub pivot: [f64; 2],
}

#[derive(Clone, Debug)]
pub(crate) struct ResizeStart {
    /// Node id → (world transform, w, h) at the start.
    nodes: Vec<(NodeId, Affine, f64, f64)>,
    /// The handle box: page transform of its top-left corner (axes along the box) and its size.
    frame: (Affine, f64, f64),
    /// One layer (resized in place) rather than several (mapped from the old box to the new).
    single: bool,
    handle: Handle,
}

#[derive(Clone, Debug, Default)]
pub(crate) enum Gesture {
    #[default]
    None,
    Marquee {
        start: Point,
        /// Shift: toggle what it touches in the selection it started from (`base`).
        additive: bool,
        base: Vec<NodeId>,
        within: Option<NodeId>,
    },
    Move {
        start: Point,
        ids: Vec<NodeId>,
        targets: Targets,
        duplicate: bool,
        moved: bool,
        /// A Shift-press on this selected layer: it leaves the selection on release, unless the
        /// press became a drag.
        deselect: Option<NodeId>,
    },
    Resize {
        start: Point,
        r: Box<ResizeStart>,
        scale_tool: bool,
        moved: bool,
    },
    Rotate {
        center: Point,
        start_angle: f64,
        ids: Vec<NodeId>,
    },
    Draw {
        start: Point,
        id: Option<NodeId>,
        parent: NodeId,
        targets: Targets,
    },
    Pan {
        sx: f64,
        sy: f64,
        cx: f64,
        cy: f64,
    },
    Pencil {
        points: Vec<Point>,
        /// The pen pressure at each point, where the device reports it.
        pressure: Vec<Option<f32>>,
    },
    TextSelect {
        id: NodeId,
    },
    /// A new text from the Text tool, still pressed: a drag makes it a box the size of the drag
    /// (at least `line`, the height of its first line, tall).
    TextBox {
        id: NodeId,
        start: Point,
        line: f64,
    },
    PenDrag {
        anchor: Point,
    },
    /// Vector edit mode gestures (handled in `vectoredit`).
    Vector(crate::vectoredit::VGesture),
    /// Dragging an ellipse's arc handle: the arc so far (see [`crate::arc_handles`]).
    Arc {
        id: NodeId,
        handle: ArcHandle,
        arc: crate::arc_handles::Arc,
    },
    /// Dragging a shape's corner radius or star ratio handle (see [`crate::shape_handles`]).
    Shape {
        id: NodeId,
        handle: ShapeHandle,
    },
    /// Dragging the rotation origin handle (see [`crate::rotation_origin`]).
    Origin,
    /// The connector tool: dragging from `from` (pressed at `start`).
    Connect {
        from: figcraft_doc::End,
        start: Point,
    },
    /// The eraser: the strokes it took so far.
    Erase {
        gone: Vec<NodeId>,
    },
    /// Shape builder: regions collected so far (handled in `shapebuilder`).
    ShapeBuild {
        picked: Vec<usize>,
        remove: bool,
        start: Point,
        last: Point,
    },
}

/// Where Shift puts the Pen's next point: as far from `last` as `p` is, at the nearest 45° step.
fn at_45_degrees(last: Point, p: Point) -> Point {
    let v = p - last;
    let ang = (v.atan2() / (std::f64::consts::PI / 4.0)).round() * std::f64::consts::PI / 4.0;
    last + Vec2::from_angle(ang) * v.hypot()
}

/// The handles a pen path shows while it is drawn ([`Overlay::pen_handles`]): the last anchor's in
/// and out handles and the one before's out handle, those that aren't zero.
fn pen_handles(points: &[(Point, Vec2, Vec2)]) -> Vec<[f64; 4]> {
    let last = points.last().into_iter().flat_map(|(p, i, o)| [(p, i), (p, o)]);
    let before = points.len().checked_sub(2).and_then(|k| points.get(k)).into_iter().map(|(p, _, o)| (p, o));
    before.chain(last).filter(|(_, h)| h.hypot() > 0.0).map(|(p, h)| [p.x, p.y, p.x + h.x, p.y + h.y]).collect()
}

/// The pen tool's in-progress path (page coordinates).
#[derive(Clone, Debug, Default)]
pub struct PenPath {
    pub id: Option<NodeId>,
    pub parent: Option<NodeId>,
    /// Anchors with (in, out) handle offsets.
    pub points: Vec<(Point, Vec2, Vec2)>,
    pub closed: bool,
    /// The path ends connected to this anchor of its own (a vector network).
    pub join: Option<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct ToolState {
    pub tool: ToolId,
    pub(crate) gesture: Gesture,
    pub overlay: Overlay,
    pub pen: Option<PenPath>,
    /// Tool to return to after a temporary tool (space → hand).
    pub temp_from: Option<ToolId>,
    /// Last click (for double-click detection by agents that only send downs).
    pub last_hover: Option<NodeId>,
    /// Shape builder: the cached arrangement of the selection, and Remove mode (a plain click
    /// removes; Alt merges).
    pub shape_builder: Option<std::sync::Arc<crate::shapebuilder::Arrangement>>,
    pub builder_remove: bool,
    /// Dev Mode: the canvas inspects. Hovering shows distances from the selection without Alt, and
    /// presses select, pan and comment but never edit the design (no moving, resizing, drawing,
    /// or editing text or vectors).
    pub dev_mode: bool,
    /// Pen snap targets, kept while hovering (see `vectoredit::point_snap`).
    pub(crate) pen_targets: Option<crate::vectoredit::point_snap::PenTargets>,
    /// The Scale tool's anchor: column and row (0 start, 1 middle, 2 end) of the point of the
    /// selection box that stays put (`tool.scaleAnchor`).
    pub scale_anchor: (u8, u8),
    /// The last drag move handled (a held button repeats it every frame).
    pub(crate) last_move: Option<PointerEvent>,
    /// The name labels of top-level frames and sections as the canvas last drew them (each label's
    /// corners on the page, turned with its frame): the Move and Scale tools treat a press or
    /// hover on one as on its frame.
    pub labels: Vec<(NodeId, [Point; 4])>,
    /// The brush the Pencil draws with (`tool.pencilBrush`; `None`: a plain stroke).
    pub pencil_brush: Option<figcraft_doc::Brush>,
    /// The board tools' colours and choices.
    pub board: crate::board::BoardTool,
    /// The eyedropper samples for a colour picker (`edit.pickColor` with `apply: false`): the
    /// colour goes to `picked`, for the picker to write to the paint it edits, not to the fill.
    pub pick_only: bool,
    /// The colour the eyedropper sampled for a colour picker, until the picker takes it.
    pub picked: Option<figcraft_color::Color>,
}

/// Pen pressure along drawn samples as width profile points `[t, scale]`: `t` by length, at most
/// 32 points, medium pressure (0.5) at the stroke weight, from a tenth to twice it. `None` when no
/// sample has pressure.
pub(crate) fn pressure_profile(points: &[Point], pressure: &[Option<f32>]) -> Option<Vec<[f64; 2]>> {
    if pressure.iter().all(Option::is_none) {
        return None;
    }
    let mut lengths = vec![0.0];
    for w in points.windows(2) {
        let (Some(a), Some(b)) = (w.first(), w.get(1)) else { continue };
        lengths.push(lengths.last().copied().unwrap_or(0.0) + (*b - *a).hypot());
    }
    let total = lengths.last().copied().unwrap_or(0.0);
    if !(total.is_finite() && total > 0.0) {
        return None;
    }
    let at = |i: usize| f64::from(pressure.get(i).copied().flatten().filter(|p| p.is_finite()).unwrap_or(0.5).clamp(0.0, 1.0));
    let n = points.len().clamp(2, 32);
    let profile = (0..n).map(|k| {
        let t = k as f64 / (n - 1) as f64;
        let s = t * total;
        let i = lengths.partition_point(|l| *l < s).min(lengths.len().saturating_sub(1));
        let p = match (i.checked_sub(1).and_then(|j| lengths.get(j)), lengths.get(i)) {
            (Some(a), Some(b)) if b > a => at(i - 1) + (at(i) - at(i - 1)) * ((s - a) / (b - a)),
            _ => at(i),
        };
        [t, (p / 0.5).clamp(0.1, 2.0)]
    });
    Some(profile.collect())
}

/// A frame or section drawn around layers takes those of its siblings that lie fully inside it
/// (visible and unlocked ones, in their order; a frame never takes a section). Not in auto layout,
/// where the new frame is one more item of the flow.
fn take_enclosed(doc: &mut Document, drawn: NodeId) -> Result<()> {
    let Some(n) = doc.node(drawn) else { return Ok(()) };
    let section = matches!(n.kind, NodeKind::Section { .. });
    let (Some(parent), Some(area)) = (n.parent.and_then(|p| doc.node(p)), doc.world_bounds(drawn)) else { return Ok(()) };
    if parent.layout.is_some() {
        return Ok(());
    }
    let inside: Vec<NodeId> = parent
        .children
        .iter()
        .copied()
        .filter(|c| *c != drawn && doc.node(*c).is_some_and(|k| k.visible && !k.locked && (section || !matches!(k.kind, NodeKind::Section { .. }))))
        .filter(|c| doc.world_bounds(*c).is_some_and(|b| area.contains(b.origin()) && area.contains(Point::new(b.x1, b.y1))))
        .collect();
    for c in inside {
        doc.reparent(c, drawn, None)?;
    }
    Ok(())
}

/// The frame whose name label (see [`ToolState::labels`]) is under `p` on `page`: the topmost
/// one drawn, if it can be selected (still there, on the page, visible and unlocked).
pub fn label_at(doc: &Document, page: NodeId, labels: &[(NodeId, [Point; 4])], p: Point) -> Option<NodeId> {
    labels
        .iter()
        .rev()
        .find(|(id, q)| in_quad(q, p) && doc.page_of(*id) == Some(page) && doc.node(*id).is_some_and(|n| n.visible && !n.locked))
        .map(|(id, _)| *id)
}

/// Whether `p` is inside the convex quadrilateral `q` (either winding; edges included).
fn in_quad(q: &[Point; 4], p: Point) -> bool {
    let (mut left, mut right) = (false, false);
    for (a, b) in q.iter().zip(q.iter().cycle().skip(1)) {
        let side = (*b - *a).cross(p - *a);
        if !side.is_finite() {
            return false;
        }
        left |= side < 0.0;
        right |= side > 0.0;
    }
    // On one side of every edge (a point can't be on all four edges of a quad with an area).
    left != right
}

/// An anchor name (`tl`, `t`, `tr`, `l`, `c`, `r`, `bl`, `b`, `br`) as column and row (0 start,
/// 1 middle, 2 end).
pub fn anchor_of(name: &str) -> Option<(u8, u8)> {
    ANCHORS.iter().position(|a| a.eq_ignore_ascii_case(name.trim())).map(|i| ((i % 3) as u8, (i / 3) as u8))
}

/// The name of anchor `(column, row)`.
pub fn anchor_name(a: (u8, u8)) -> &'static str {
    ANCHORS.get(usize::from(a.1.min(2)) * 3 + usize::from(a.0.min(2))).copied().unwrap_or("tl")
}

/// The fill new text takes to be readable where it is drawn, in `parent`: white on a dark
/// background (None: the model's default, black). The background is the nearest layer from `parent`
/// out whose top visible fill is a mostly opaque solid colour, else the page (#362). For text made
/// in the app: by the Text tool, or by a command that names no fill while the interface is up.
pub(crate) fn readable_text_fills(doc: &Document, parent: NodeId) -> Option<Vec<figcraft_color::Paint>> {
    let dark = std::iter::once(parent).chain(doc.ancestors(parent)).filter_map(|id| doc.node(id)).find_map(|n| match &n.kind {
        NodeKind::Page { background, .. } => Some(background.is_dark()),
        _ => n.fills.iter().rev().find(|p| p.visible).and_then(|p| p.solid_color()).filter(|c| c.a >= 0.5).map(|c| c.is_dark()),
    });
    dark.unwrap_or(false).then(|| vec![figcraft_color::Paint::solid(figcraft_color::Color::WHITE)])
}

const ANCHORS: [&str; 9] = ["tl", "t", "tr", "l", "c", "r", "bl", "b", "br"];

impl ToolState {
    pub fn busy(&self) -> bool {
        !matches!(self.gesture, Gesture::None)
    }
}

const HANDLE_PX: f64 = 5.0;
const ROTATE_PX: f64 = 18.0;
/// Snap distance in screen pixels.
pub(crate) const SNAP_PX: f64 = 5.0;
/// Screen pixels the pointer travels after a press before a move or resize starts: a click or a
/// double-click made with an unsteady hand doesn't drag what it lands on.
const DRAG_SLOP_PX: f64 = 3.0;

/// Corners of the selection's handle box in page coordinates (tl, tr, br, bl).
pub fn selection_box(doc: &Document, sel: &[NodeId]) -> Option<[Point; 4]> {
    let (f, w, h) = selection_frame(doc, sel)?;
    Some([f * Point::ZERO, f * Point::new(w, 0.0), f * Point::new(w, h), f * Point::new(0.0, h)])
}

/// The selection's handle box: the page transform of its top-left corner (axes along the box) and
/// its size. One layer: its own box; several sharing a rotation: a box turned with them;
/// otherwise the page-aligned bounds.
pub fn selection_frame(doc: &Document, sel: &[NodeId]) -> Option<(Affine, f64, f64)> {
    let ids = roots(doc, sel);
    if let [id] = ids.as_slice() {
        let n = doc.node(*id)?;
        return Some((doc.world_transform(*id), n.width, n.height));
    }
    let Some(angle) = shared_rotation(doc, &ids) else {
        let b = doc.bounds_of(&ids)?;
        return Some((Affine::translate((b.x0, b.y0)), b.width(), b.height()));
    };
    let rot = Affine::rotate(angle);
    let inv = rot.inverse();
    let b = ids
        .iter()
        .filter_map(|i| Some(figcraft_geom::transform_rect_bbox(inv * doc.world_transform(*i), doc.node(*i)?.local_rect())))
        .reduce(|a, b| a.union(b))?;
    Some((rot * Affine::translate((b.x0, b.y0)), b.width(), b.height()))
}

/// The rotation (radians) every layer in `ids` shares, unless it's a multiple of 90 degrees (the
/// page-aligned box is then the same box). Flipped layers count by their axis, not its direction.
fn shared_rotation(doc: &Document, ids: &[NodeId]) -> Option<f64> {
    use std::f64::consts::{FRAC_PI_2, PI};
    const EPS: f64 = 1e-4;
    let mut angles = ids.iter().map(|i| {
        let c = doc.world_transform(*i).as_coeffs();
        c[1].atan2(c[0])
    });
    let a = angles.next()?;
    let same = |b: f64| {
        let d = (b - a).rem_euclid(PI);
        d.min(PI - d) < EPS
    };
    let axis_aligned = (a - (a / FRAC_PI_2).round() * FRAC_PI_2).abs() < EPS;
    (a.is_finite() && !axis_aligned && angles.all(same)).then_some(a)
}

/// Whether layer `id` is locked or inside a locked layer: it can be selected in the layers panel
/// but not moved or resized on the canvas.
pub fn locked_in(doc: &Document, id: NodeId) -> bool {
    std::iter::once(id).chain(doc.ancestors(id)).any(|a| doc.node(a).is_some_and(|n| n.locked))
}

/// Which handle (if any) is under `p`. A selection with a locked layer has none.
pub fn handle_at(doc: &Document, sel: &[NodeId], p: Point, zoom: f64) -> Option<Handle> {
    if sel.is_empty() || sel.iter().any(|i| locked_in(doc, *i)) {
        return None;
    }
    let corners = selection_box(doc, sel)?;
    // Lines (zero height) only get end handles.
    let is_line = sel.len() == 1 && sel.first().and_then(|i| doc.node(*i)).is_some_and(|n| matches!(n.kind, NodeKind::Line));
    handle_at_corners(&corners, p, zoom, is_line)
}

/// Half the side of the square the canvas draws on each corner of the handle box (screen pixels,
/// upright whatever the box's rotation).
const CORNER_SQUARE_PX: f64 = 4.0;

/// Whether `p` is on a corner square of the selection's handle box as drawn: there the corner
/// resizes, although a corner-radius handle 10 px in reaches that far too.
fn on_corner_square(doc: &Document, sel: &[NodeId], p: Point, zoom: f64) -> bool {
    let reach = CORNER_SQUARE_PX / zoom.max(1e-6);
    matches!(handle_at(doc, sel, p, zoom), Some(Handle::Corner(_)))
        && selection_box(doc, sel).is_some_and(|c| c.iter().any(|c| (c.x - p.x).abs() <= reach && (c.y - p.y).abs() <= reach))
}

/// A box whose sides are both shorter than this on screen (pixels) would be covered by its
/// handles: the canvas draws just its outline (the handles still answer just outside it).
pub const SMALL_BOX_PX: f64 = 16.0;

/// The stroke the Pencil draws: its colour and weight (the canvas previews it the same).
pub const PENCIL_STROKE: (figcraft_color::Color, f64) = (figcraft_color::Color::BLACK, 2.0);

/// The shorter and longer sides, in screen pixels, of the box with `corners` (tl, tr, br, bl).
pub fn box_sides_px(corners: &[Point; 4], zoom: f64) -> (f64, f64) {
    let [a, b, _, d] = corners;
    let (w, h) = ((*b - *a).hypot() * zoom, (*d - *a).hypot() * zoom);
    (w.min(h), w.max(h))
}

/// Which handle of the box with `corners` (tl, tr, br, bl in page coordinates) is under `p`;
/// `is_line` keeps only the two end handles.
pub fn handle_at_corners(corners: &[Point; 4], p: Point, zoom: f64, is_line: bool) -> Option<Handle> {
    let tol = HANDLE_PX / zoom.max(1e-6);
    // Inside the box, handles reach in at most a quarter of its shorter side, so that a press in
    // the middle of a small layer moves it; outside it they reach as far as ever.
    let inner = if !is_line && in_quad(corners, p) { box_sides_px(corners, zoom).0 / 4.0 / zoom.max(1e-6) } else { f64::INFINITY };
    let (corner_reach, edge_reach) = ((tol * 1.6).min(inner), tol.min(inner));
    // The nearest corner, then the nearest edge, in reach (a small box's reaches overlap).
    let nearest = |hits: &mut dyn Iterator<Item = (f64, Handle)>| hits.min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, h)| h);
    let corner = nearest(
        &mut corners
            .iter()
            .enumerate()
            .filter(|(i, _)| !(is_line && *i >= 2))
            .map(|(i, c)| ((*c - p).hypot(), Handle::Corner(i as u8)))
            .filter(|(d, _)| *d <= corner_reach),
    );
    if corner.is_some() {
        return corner;
    }
    if !is_line {
        use figcraft_geom::kurbo::ParamCurveNearest;
        let edge = nearest(&mut (0..4).filter_map(|i| {
            let (a, b) = (corners.get(i)?, corners.get((i + 1) % 4)?);
            let near = figcraft_geom::kurbo::Line::new(*a, *b).nearest(p, 0.001);
            let d = near.distance_sq.sqrt();
            (d <= edge_reach && near.t > 0.05 && near.t < 0.95).then_some((d, Handle::Edge(i as u8)))
        }));
        if edge.is_some() {
            return edge;
        }
    }
    // Rotation zones: just outside each corner.
    let center = Point::new(corners.iter().map(|c| c.x).sum::<f64>() / 4.0, corners.iter().map(|c| c.y).sum::<f64>() / 4.0);
    let rot = ROTATE_PX / zoom.max(1e-6);
    for (i, c) in corners.iter().enumerate() {
        let d = (*c - p).hypot();
        let outward = (p - *c).dot(*c - center) > 0.0;
        if d <= rot && d > tol && outward {
            return Some(Handle::Rotate(i as u8));
        }
    }
    None
}

fn resize_cursor(doc: &Document, sel: &[NodeId], h: Handle) -> Cursor {
    selection_box(doc, sel).map_or(Cursor::Default, |c| handle_cursor(&c, h))
}

/// The cursor over handle `h` of the box with corners `c`.
pub(crate) fn handle_cursor(c: &[Point; 4], h: Handle) -> Cursor {
    let angle = |a: Point, b: Point| (b - a).atan2().to_degrees();
    match h {
        Handle::Corner(i) => {
            let a = c.get(i as usize).copied().unwrap_or_default();
            let b = c.get((i as usize + 2) % 4).copied().unwrap_or_default();
            // Along the box's own axes at 45°, not the diagonal, so wide or tall boxes still get a
            // diagonal cursor. A box with no width or height falls back to the diagonal.
            let unit = |v: figcraft_geom::kurbo::Vec2| {
                let len = v.hypot();
                (len.is_finite() && len > 1e-9).then(|| v / len)
            };
            let next = c.get((i as usize + 1) % 4).copied().unwrap_or_default();
            let prev = c.get((i as usize + 3) % 4).copied().unwrap_or_default();
            match (unit(next - a), unit(prev - a)) {
                (Some(u), Some(v)) => Cursor::Resize((u + v).atan2().to_degrees()),
                _ => Cursor::Resize(angle(a, b)),
            }
        }
        Handle::Edge(i) => {
            let a = c.get(i as usize).copied().unwrap_or_default();
            let b = c.get((i as usize + 1) % 4).copied().unwrap_or_default();
            Cursor::Resize(angle(a, b) + 90.0)
        }
        Handle::Rotate(i) => {
            let centre = Point::new(c.iter().map(|p| p.x).sum::<f64>() / 4.0, c.iter().map(|p| p.y).sum::<f64>() / 4.0);
            Cursor::Rotate(angle(centre, c.get(i as usize).copied().unwrap_or_default()))
        }
    }
}

/// The box (x0, y0, x1, y1) a `w0`×`h0` handle box becomes when `handle` is dragged to `q`, all in
/// the box's own axes: `keep` constrains proportions, `from_center` resizes about the centre (the
/// opposite side mirrors the dragged one). Edges that cross over come back reversed (x1 < x0 or
/// y1 < y0: a flip).
pub(crate) fn resized_box(handle: Handle, q: Point, w0: f64, h0: f64, keep: bool, from_center: bool) -> (f64, f64, f64, f64) {
    let (mut x0, mut y0, mut x1, mut y1) = (0.0, 0.0, w0, h0);
    // The dragged sides: x0 or x1 (`Some(false)` / `Some(true)`), y0 or y1.
    let (sx, sy) = match handle {
        Handle::Corner(0) => (Some(false), Some(false)),
        Handle::Corner(1) => (Some(true), Some(false)),
        Handle::Corner(2) => (Some(true), Some(true)),
        Handle::Corner(_) => (Some(false), Some(true)),
        Handle::Edge(0) => (None, Some(false)),
        Handle::Edge(1) => (Some(true), None),
        Handle::Edge(2) => (None, Some(true)),
        Handle::Edge(_) => (Some(false), None),
        Handle::Rotate(_) => (None, None),
    };
    match sx {
        Some(false) => (x0, x1) = (q.x, if from_center { w0 - q.x } else { w0 }),
        Some(true) => (x0, x1) = (if from_center { w0 - q.x } else { 0.0 }, q.x),
        None => {}
    }
    match sy {
        Some(false) => (y0, y1) = (q.y, if from_center { h0 - q.y } else { h0 }),
        Some(true) => (y0, y1) = (if from_center { h0 - q.y } else { 0.0 }, q.y),
        None => {}
    }
    if keep && w0 > 1e-9 && h0 > 1e-9 && !matches!(handle, Handle::Rotate(_)) {
        // One factor for both axes: the dragged axis's (a corner: the one that moved the most),
        // about the point that stays put (the opposite handle, or the centre).
        let (kx, ky) = ((x1 - x0) / w0, (y1 - y0) / h0);
        let k = match (sx, sy) {
            (None, _) => ky,
            (_, None) => kx,
            _ if kx.abs() > ky.abs() => kx,
            _ => ky,
        };
        let pivot = if from_center { Point::new(w0 / 2.0, h0 / 2.0) } else { handle_point(opposite(handle), w0, h0) };
        (x0, x1) = (pivot.x * (1.0 - k), pivot.x + (w0 - pivot.x) * k);
        (y0, y1) = (pivot.y * (1.0 - k), pivot.y + (h0 - pivot.y) * k);
    }
    (x0, y0, x1, y1)
}

/// A handle's point in its box's axes (`w0`×`h0`): a corner or an edge's middle.
fn handle_point(h: Handle, w0: f64, h0: f64) -> Point {
    match h {
        Handle::Corner(0) | Handle::Rotate(0) => Point::new(0.0, 0.0),
        Handle::Corner(1) | Handle::Rotate(1) => Point::new(w0, 0.0),
        Handle::Corner(2) | Handle::Rotate(2) => Point::new(w0, h0),
        Handle::Corner(_) | Handle::Rotate(_) => Point::new(0.0, h0),
        Handle::Edge(0) => Point::new(w0 / 2.0, 0.0),
        Handle::Edge(1) => Point::new(w0, h0 / 2.0),
        Handle::Edge(2) => Point::new(w0 / 2.0, h0),
        Handle::Edge(_) => Point::new(0.0, h0 / 2.0),
    }
}

/// Anchor `(column, row)` of a `w`×`h` box, in its axes.
fn anchor_in_box(anchor: (u8, u8), w: f64, h: f64) -> Point {
    Point::new(w * f64::from(anchor.0.min(2)) / 2.0, h * f64::from(anchor.1.min(2)) / 2.0)
}

/// The Scale tool's anchor point on the selection's handle box, in page coordinates.
pub fn scale_anchor_point(doc: &Document, sel: &[NodeId], anchor: (u8, u8)) -> Option<Point> {
    let (f, w, h) = selection_frame(doc, sel)?;
    Some(f * anchor_in_box(anchor, w, h))
}

/// The handle across the box from `h`.
fn opposite(h: Handle) -> Handle {
    match h {
        Handle::Corner(i) => Handle::Corner((i + 2) % 4),
        Handle::Edge(i) => Handle::Edge((i + 2) % 4),
        Handle::Rotate(i) => Handle::Rotate((i + 2) % 4),
    }
}

/// The Scale tool's factor and pivot (in the box's axes) for dragging `handle` of a `w0`×`h0` box
/// to `q`. The pivot is the anchor point (`anchor`: column, row), or the opposite handle when the
/// anchor lies on the side being dragged, or the centre (`from_center`). Corners follow the axis
/// that moved the most; a negative factor flips. None when the box has no size to scale by.
pub(crate) fn scale_factor(handle: Handle, q: Point, w0: f64, h0: f64, anchor: (u8, u8), from_center: bool) -> Option<(f64, Point)> {
    const EPS: f64 = 1e-9;
    if !(q.x.is_finite() && q.y.is_finite() && w0.is_finite() && h0.is_finite()) || matches!(handle, Handle::Rotate(_)) {
        return None;
    }
    let h = handle_point(handle, w0, h0);
    let anchor_pt = anchor_in_box(anchor, w0, h0);
    // The axes the handle moves along.
    let (ax_x, ax_y) = match handle {
        Handle::Edge(0) | Handle::Edge(2) => (false, true),
        Handle::Edge(_) => (true, false),
        _ => (true, true),
    };
    let on_side = (ax_x && (anchor_pt.x - h.x).abs() < EPS) || (ax_y && (anchor_pt.y - h.y).abs() < EPS);
    let pivot = if from_center {
        Point::new(w0 / 2.0, h0 / 2.0)
    } else if on_side {
        handle_point(opposite(handle), w0, h0)
    } else {
        anchor_pt
    };
    let ratio = |on: bool, to: f64, from: f64, p: f64| (on && (from - p).abs() > EPS).then(|| (to - p) / (from - p));
    let kx = ratio(ax_x, q.x, h.x, pivot.x);
    let ky = ratio(ax_y, q.y, h.y, pivot.y);
    let k = match (kx, ky) {
        (Some(a), Some(b)) => {
            if a.abs() > b.abs() {
                a
            } else {
                b
            }
        }
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => return None,
    };
    if !k.is_finite() {
        return None;
    }
    // Never collapse to nothing (or blow up past a usable size); 0 counts as positive.
    Some((k.signum() * k.abs().clamp(0.01, 1000.0), pivot))
}

impl Session {
    /// What a marquee starting at page point `p` selects inside: the top-level frame it starts in
    /// (its children), or the focus view's frame.
    pub(crate) fn marquee_within(&self, p: Point) -> Result<Option<NodeId>> {
        let (doc, page) = (self.doc()?, self.page()?);
        let within = frame_at(doc, page, p, &[]).filter(|f| doc.node(*f).is_some_and(|n| n.parent == Some(page)));
        let within = within.filter(|f| doc.world_bounds(*f).is_some_and(|b| b.contains(p)));
        Ok(self.ds()?.focused().or(within))
    }

    /// The selection a marquee over page rect `r` makes from `base` (what was selected before):
    /// the layers it touches (`deep`: nested ones too); `additive` (Shift) toggles them.
    pub(crate) fn marquee_selection(&self, r: Rect, within: Option<NodeId>, deep: bool, base: &[NodeId], additive: bool) -> Result<Vec<NodeId>> {
        let (doc, page) = (self.doc()?, self.page()?);
        let mut hits =
            if deep { figcraft_doc::geometry::marquee_deep(doc, page, r) } else { figcraft_doc::geometry::marquee_select(doc, page, r, within) };
        if let Some(f) = self.ds()?.focused() {
            hits.retain(|h| *h != f && doc.is_ancestor(f, *h));
        }
        // A marquee over a frame from outside selects the frame only when it covers it fully.
        if within.is_none() && !deep {
            hits.retain(|h| {
                let full = doc.world_bounds(*h).is_some_and(|b| r.contains(b.origin()) && r.contains(Point::new(b.x1, b.y1)));
                full || !doc.node(*h).is_some_and(|n| n.kind.is_frame_like() || matches!(n.kind, NodeKind::Section { .. }))
            });
        }
        // Shift toggles: what was selected and is touched again comes out.
        let mut sel: Vec<NodeId> = base.iter().copied().filter(|b| !(additive && hits.contains(b))).collect();
        for h in hits {
            if !base.contains(&h) && !sel.contains(&h) {
                sel.push(h);
            }
        }
        Ok(sel)
    }

    pub fn set_tool(&mut self, t: ToolId) {
        if self.tool.tool == ToolId::Pen && t != ToolId::Pen {
            let _ = self.pen_finish();
        }
        if !matches!(t, ToolId::Move | ToolId::Scale | ToolId::Hand) && self.ds().is_ok_and(|d| d.vector_edit.is_some()) {
            let _ = crate::vectoredit::end(self);
        }
        self.tool.tool = t;
        self.tool.gesture = Gesture::None;
        self.tool.pick_only = false;
        // The tool's cursor shows before the pointer moves.
        self.tool.overlay =
            Overlay { builder_remove: self.tool.builder_remove && t == ToolId::ShapeBuilder, cursor: t.cursor(), ..Overlay::default() };
        self.tool.shape_builder = None;
        if t == ToolId::ShapeBuilder {
            // Regions show straight away, before the pointer moves.
            let sel = self.selection();
            let _ = crate::shapebuilder::arrangement(self, &sel);
        }
        if let Ok(ds) = self.ds_mut() {
            if t != ToolId::Text && t != ToolId::Move {
                ds.text_edit = None;
            }
            ds.revision += 1;
        }
    }

    /// The tool pointer events go to: in Dev Mode every tool but the hand and comments selects.
    fn acting_tool(&self) -> ToolId {
        match self.tool.tool {
            t if self.tool.dev_mode && !matches!(t, ToolId::Hand | ToolId::Comment) => ToolId::Move,
            t => t,
        }
    }

    /// Drop the gesture under way (a drag, a resize, a shape being drawn), restoring the document
    /// as Escape does; the UI calls it when Dev Mode starts, where nothing may move.
    pub fn drop_gesture(&mut self) -> Result<()> {
        self.tool.gesture = Gesture::None;
        self.cancel_interaction()
    }

    /// Feed one pointer event to the current tool.
    pub fn pointer(&mut self, ev: &PointerEvent) -> Result<()> {
        // A held button sends the same move every frame: running the gesture again would only
        // rebuild the same preview (a new document, and a canvas render, every frame).
        if ev.kind != PointerKind::Move {
            self.tool.last_move = None;
        } else if self.tool.busy() {
            if self.tool.last_move.as_ref() == Some(ev) {
                return Ok(());
            }
            self.tool.last_move = Some(ev.clone());
        }
        let tool = self.acting_tool();
        let edits = matches!(tool, ToolId::Move | ToolId::Scale) && !self.tool.dev_mode;
        if edits && self.origin_pointer(ev)? {
            return Ok(());
        }
        if edits && self.ds().is_ok_and(|d| d.vector_edit.is_some()) && self.vector_pointer(ev)? {
            return Ok(());
        }
        if tool == ToolId::ShapeBuilder {
            return self.shape_builder_pointer(ev);
        }
        match ev.kind {
            PointerKind::Hover => self.hover(ev),
            PointerKind::Down => match tool {
                ToolId::Move | ToolId::Scale => {
                    // A click on the canvas deselects a ruler guide (the UI selects guides itself).
                    self.ds_mut()?.guide = None;
                    self.move_down(ev)
                }
                ToolId::Hand => {
                    let v = &self.ds()?.view;
                    self.tool.gesture = Gesture::Pan { sx: ev.sx, sy: ev.sy, cx: v.cx, cy: v.cy };
                    self.tool.overlay.cursor = Cursor::Grabbing;
                    Ok(())
                }
                ToolId::Text => self.text_down(ev),
                ToolId::Pen => self.pen_down(ev),
                ToolId::Pencil => {
                    self.tool.gesture = Gesture::Pencil { points: vec![ev.p()], pressure: vec![ev.pressure] };
                    Ok(())
                }
                ToolId::Comment => {
                    self.request_ui("comment.new", json!({"x": ev.x, "y": ev.y}));
                    Ok(())
                }
                ToolId::Eyedropper => self.eyedrop(ev),
                t if t.on_boards() => self.board_down(ev),
                t if t.draws_shape() => self.draw_down(ev),
                _ => Ok(()),
            },
            PointerKind::Move => self.drag(ev),
            PointerKind::Up => self.up(ev),
            PointerKind::DoubleClick => self.double_click(ev),
        }
    }

    /// The rotation origin handle while it shows: a press drags it, a double-click puts it back
    /// at the centre. True when the event was the handle's.
    fn origin_pointer(&mut self, ev: &PointerEvent) -> Result<bool> {
        let dragging = matches!(self.tool.gesture, Gesture::Origin);
        if !dragging && !self.ds()?.on_origin_handle(ev.p(), ev.zoom) {
            return Ok(false);
        }
        match ev.kind {
            PointerKind::Down if !dragging => self.tool.gesture = Gesture::Origin,
            PointerKind::Move if dragging => self.drag_origin_to(ev.p(), ev.zoom)?,
            PointerKind::Up if dragging => self.tool.gesture = Gesture::None,
            PointerKind::DoubleClick => self.set_rotation_origin(None)?,
            PointerKind::Hover => {}
            _ => return Ok(false),
        }
        self.tool.overlay.cursor = Cursor::Move;
        Ok(true)
    }

    fn hover(&mut self, ev: &PointerEvent) -> Result<()> {
        let tool = self.acting_tool();
        let ds = self.ds()?;
        let (doc, sel, page) = (ds.doc.clone(), ds.selection.clone(), ds.page);
        let mut cursor = tool.cursor();
        // The text being edited, and with the Text tool any text, takes the I-beam: a click there
        // puts the caret.
        let on_edited = ds
            .text_edit
            .as_ref()
            .is_some_and(|te| doc.world_bounds(te.id).is_some_and(|b| b.inflate(ev.tol() * 4.0, ev.tol() * 4.0).contains(ev.p())));
        let on_text =
            |s: &Self| s.hit(&doc, page, ev.p(), ev.tol() * 2.0).and_then(|l| doc.node(l)).is_some_and(|n| matches!(n.kind, NodeKind::Text(_)));
        if tool == ToolId::Text && (on_edited || on_text(self)) {
            cursor = Cursor::Text;
        }
        let mut hover = None;
        let mut guides = Vec::new();
        let mut measure_box = None;
        let arc_hover = if self.arc_handles_shown() { crate::arc_handles::hit(&doc, &sel, ev.p(), ev.zoom).map(|(_, h)| h) } else { None };
        self.tool.overlay.arc_hover = arc_hover;
        let shape_hover = if self.arc_handles_shown() && !on_corner_square(&doc, &sel, ev.p(), ev.zoom) {
            crate::shape_handles::hit(&doc, &sel, ev.p(), ev.zoom).map(|(_, h)| h)
        } else {
            None
        };
        self.tool.overlay.shape_hover = shape_hover;
        if matches!(tool, ToolId::Move | ToolId::Scale) && on_edited {
            cursor = Cursor::Text;
        } else if matches!(tool, ToolId::Move | ToolId::Scale) && arc_hover.is_none() && shape_hover.is_none() {
            if let Some(h) = handle_at(&doc, &sel, ev.p(), ev.zoom).filter(|_| !self.tool.dev_mode) {
                cursor = resize_cursor(&doc, &sel, h);
            } else {
                let leaf = self.hit(&doc, page, ev.p(), ev.tol() * 2.0);
                if leaf.and_then(|l| crate::widgets::target_at(&doc, l, ev.p())).is_some() {
                    cursor = Cursor::Pointer;
                }
                hover = label_at(&doc, page, &self.tool.labels, ev.p()).or_else(|| leaf.map(|leaf| select_target(&doc, leaf, &sel, ev.cmd)));
                // Alt over a selected layer: a drag from here duplicates it.
                if ev.alt && !self.tool.dev_mode && tool == ToolId::Move && hover.is_some_and(|h| sel.contains(&h) && !locked_in(&doc, h)) {
                    cursor = Cursor::Copy;
                }
                if ev.alt || self.tool.dev_mode {
                    use crate::distance::{Hover, guide_at, hover_lines, padding_frame, parent_of};
                    // Shift+Alt over an auto layout frame: its padding and gaps. Otherwise from the
                    // selection to a ruler guide, the hovered layer or (over empty canvas) the parent.
                    let around: Vec<NodeId> = sel.iter().copied().chain(hover).collect();
                    let what = hover.filter(|_| ev.alt && ev.shift).and_then(|t| padding_frame(&doc, t)).map(Hover::Padding).or_else(|| {
                        if sel.is_empty() {
                            return None;
                        }
                        let guide = guide_at(&doc, page, &around, ev.p(), ev.tol() * 4.0).map(|(vertical, at)| Hover::Guide { vertical, at });
                        guide.or_else(|| hover.or_else(|| parent_of(&doc, page, &sel)).map(Hover::Layer))
                    });
                    if let Some(what) = what {
                        let (g, b) = hover_lines(&doc, page, &sel, what);
                        guides = g;
                        measure_box = b.map(|b| [b.x0, b.y0, b.x1, b.y1]);
                    }
                }
            }
        }
        self.tool.overlay.measure_box = measure_box;
        self.tool.overlay.cursor = cursor;
        if tool == ToolId::Pen {
            // Where a click would land (snapped), with its guides.
            let (to, g) = if ev.shift { (ev.p(), Vec::new()) } else { self.snap_pen_tool(ev)? };
            guides = g;
            self.tool.overlay.cursor = Cursor::Pen(self.pen_cursor(&doc, &sel, to, ev.zoom));
            if let Some((last, _, out)) = self.tool.pen.as_ref().and_then(|pen| pen.points.last()) {
                // With Shift the band ends where a click puts the point (#970).
                let to = if ev.shift { at_45_degrees(*last, to) } else { to };
                self.tool.overlay.rubber = Some([last.x, last.y, to.x, to.y]);
                self.tool.overlay.rubber_handle = (out.hypot() > 0.0).then_some([last.x + out.x, last.y + out.y]);
            }
        }
        self.tool.overlay.guides = guides;
        let ds = self.ds_mut()?;
        if ds.hover != hover {
            ds.hover = hover;
            ds.revision += 1;
        }
        Ok(())
    }

    /// What a click with the Pen tool at `p` (where it snaps) would do: close the path on its first
    /// point, connect to another of its points, go on from a selected open path's end, or add a
    /// new point (the same tests as [`Self::pen_down`]).
    fn pen_cursor(&self, doc: &Document, sel: &[NodeId], p: Point, zoom: f64) -> PenCursor {
        let near = |q: &Point| (*q - p).hypot() * zoom < 8.0;
        let on_point = match &self.tool.pen {
            None => selected_end_at(doc, sel, p, 8.0 / zoom.max(1e-6)).is_some(),
            Some(pen) => {
                let n = pen.points.len();
                (n > 2 && pen.points.first().is_some_and(|(q, _, _)| near(q)))
                    || (1..n.saturating_sub(1)).any(|k| pen.points.get(k).is_some_and(|(q, _, _)| near(q)))
            }
        };
        if on_point { PenCursor::Close } else { PenCursor::New }
    }

    /// Whether what the gesture did to `ids` so far is an edit only a main component can make
    /// (`moved`: their place among the layers counts too): it is undone when the gesture ends.
    fn refused(&self, ids: &[NodeId], moved: bool) -> bool {
        let Ok(ds) = self.ds() else { return false };
        ds.interaction.as_ref().is_some_and(|it| {
            ids.iter().any(|m| {
                (moved && !crate::components::move_allowed(&it.doc, &ds.doc, *m))
                    || crate::components::refused_override(&it.doc, &ds.doc, *m).is_some()
            })
        })
    }

    fn move_down(&mut self, ev: &PointerEvent) -> Result<()> {
        let ds = self.ds()?;
        let (doc, sel, page) = (ds.doc.clone(), ds.selection.clone(), ds.page);
        // Clicking outside the text being edited ends editing.
        if let Some(te) = ds.text_edit.clone() {
            let inside = doc.world_bounds(te.id).is_some_and(|b| b.inflate(ev.tol() * 4.0, ev.tol() * 4.0).contains(ev.p()));
            if inside {
                return self.text_click(te.id, ev, ev.shift);
            }
            textedit::end(self)?;
        }
        // Handles first: a selected ellipse's arc handles (drawn over the box), then the box's.
        let edits = !self.tool.dev_mode;
        if self.arc_handles_shown()
            && let Some((id, handle)) = crate::arc_handles::hit(&doc, &sel, ev.p(), ev.zoom)
            && let Some(arc) = crate::arc_handles::arc_of(&doc, id)
        {
            self.begin_interaction("Arc")?;
            self.tool.gesture = Gesture::Arc { id, handle, arc };
            self.tool.overlay.arc =
                crate::arc_handles::handle_at(&doc, id, handle, arc, ev.zoom).map(|at| ArcHint { handle, label: arc.label(handle), at });
            return Ok(());
        }
        if self.arc_handles_shown()
            && !on_corner_square(&doc, &sel, ev.p(), ev.zoom)
            && let Some((id, handle)) = crate::shape_handles::hit(&doc, &sel, ev.p(), ev.zoom)
        {
            self.begin_interaction("Corner radius")?;
            self.tool.gesture = Gesture::Shape { id, handle };
            self.tool.overlay.shape = shape_hint(&doc, id, handle, ev.zoom);
            return Ok(());
        }
        if let Some(h) = handle_at(&doc, &sel, ev.p(), ev.zoom).filter(|_| edits) {
            let ids = roots(&doc, &sel);
            if let Handle::Rotate(_) = h {
                let Some(b) = selection_box(&doc, &ids) else { return Ok(()) };
                let center = self
                    .ds()?
                    .moved_origin()
                    .unwrap_or_else(|| Point::new(b.iter().map(|c| c.x).sum::<f64>() / 4.0, b.iter().map(|c| c.y).sum::<f64>() / 4.0));
                self.begin_interaction("Rotate")?;
                self.tool.gesture = Gesture::Rotate { center, start_angle: (ev.p() - center).atan2(), ids };
                return Ok(());
            }
            let nodes: Vec<(NodeId, Affine, f64, f64)> =
                ids.iter().filter_map(|i| doc.node(*i).map(|n| (*i, doc.world_transform(*i), n.width, n.height))).collect();
            let Some(frame) = selection_frame(&doc, &ids) else { return Ok(()) };
            let single = ids.len() == 1;
            self.begin_interaction(if self.tool.tool == ToolId::Scale { "Scale" } else { "Resize" })?;
            self.tool.gesture = Gesture::Resize {
                start: ev.p(),
                r: Box::new(ResizeStart { nodes, frame, single, handle: h }),
                scale_tool: self.tool.tool == ToolId::Scale,
                moved: false,
            };
            return Ok(());
        }
        // A frame's name label picks the frame itself. The background of a top-level frame with
        // layers in it is like the canvas (a press starts a marquee inside it) unless the frame is
        // selected; an empty frame (or a widget, or a board object such as a sticky) is picked and
        // dragged.
        let board_background = |leaf: NodeId| {
            !sel.contains(&leaf)
                && doc.node(leaf).is_some_and(|n| {
                    n.parent == Some(page) && n.kind.is_frame_like() && !n.children.is_empty() && !crate::widgets::is_widget(n) && !n.is_board_atom()
                })
        };
        let label = label_at(&doc, page, &self.tool.labels, ev.p());
        let target = label.or_else(|| {
            self.hit(&doc, page, ev.p(), ev.tol() * 2.0)
                .filter(|leaf| ev.cmd || !board_background(*leaf))
                .map(|leaf| select_target(&doc, leaf, &sel, ev.cmd))
        });
        // A press inside the selection's box that reaches nothing above it (an unfilled shape's
        // inside, or the frame or canvas behind it) drags the selection.
        let behind_sel = |t: Option<NodeId>| t.is_none_or(|t| sel.iter().any(|s| *s != t && doc.is_ancestor(t, *s)));
        let target = match sel.iter().copied().find(|s| !locked_in(&doc, *s)) {
            Some(first) if label.is_none() && !ev.shift && behind_sel(target) && selection_box(&doc, &sel).is_some_and(|q| in_quad(&q, ev.p())) => {
                Some(first)
            }
            _ => target,
        };
        match target {
            Some(target) => {
                // Clicking inside the current selection's box keeps it (drag them all).
                let in_sel_box = sel.len() > 1 && doc.bounds_of(&sel).is_some_and(|b| b.contains(ev.p())) && !ev.shift;
                // Shift on a selected layer takes it out on release if the press doesn't become a
                // drag; a drag moves the selection with it. Where no drag can start, it goes now.
                let deselect = (ev.shift && sel.contains(&target)).then_some(target);
                let without_target = || sel.iter().copied().filter(|x| *x != target).collect::<Vec<_>>();
                let new_sel = if in_sel_box && (sel.contains(&target) || sel.iter().any(|s| doc.is_ancestor(*s, target))) {
                    sel.clone()
                } else if ev.shift && deselect.is_none() {
                    let mut s2 = sel.clone();
                    s2.push(target);
                    s2
                } else if sel.contains(&target) {
                    sel.clone()
                } else {
                    vec![target]
                };
                self.set_selection(new_sel.clone());
                // Dev Mode only selects (Shift-click still deselects): nothing moves.
                if !edits {
                    if deselect.is_some() {
                        self.set_selection(without_target());
                    }
                    return Ok(());
                }
                // Locked layers selected in the layers panel stay put.
                let ids: Vec<NodeId> = roots(&doc, &new_sel).into_iter().filter(|i| !locked_in(&doc, *i)).collect();
                if new_sel.is_empty() || doc.node(target).is_some_and(|n| n.locked) || ids.is_empty() {
                    if deselect.is_some() {
                        self.set_selection(without_target());
                    }
                    return Ok(());
                }
                let mut targets = Targets::gather(&doc, page, &ids);
                // A copy dragged out with Alt snaps to the layers it came from too (they stay put).
                if ev.alt
                    && let Some(b) = targets.moving
                {
                    targets.add_box(b);
                }
                if let Err(e) = self.begin_interaction(if ev.alt { "Duplicate" } else { "Move" }) {
                    if deselect.is_some() {
                        self.set_selection(without_target());
                    }
                    return Err(e);
                }
                self.tool.gesture = Gesture::Move { start: ev.p(), ids, targets, duplicate: ev.alt, moved: false, deselect };
                self.tool.overlay.cursor = if ev.alt { Cursor::Copy } else { Cursor::Default };
            }
            None => {
                let within = self.marquee_within(ev.p())?;
                if !ev.shift {
                    self.set_selection(vec![]);
                }
                self.tool.gesture = Gesture::Marquee { start: ev.p(), additive: ev.shift, base: if ev.shift { sel } else { vec![] }, within };
            }
        }
        Ok(())
    }

    fn drag(&mut self, ev: &PointerEvent) -> Result<()> {
        let g = std::mem::take(&mut self.tool.gesture);
        let res = match &g {
            Gesture::None => return self.hover(ev),
            Gesture::Marquee { start, additive, base, within } => {
                let r = Rect::from_points(*start, ev.p());
                self.tool.overlay.marquee = Some([r.x0, r.y0, r.x1, r.y1]);
                // Cmd/Ctrl: the nested layers it touches.
                let sel = self.marquee_selection(r, *within, ev.cmd, base, *additive)?;
                self.set_selection(sel);
                Ok(())
            }
            Gesture::Move { start, moved: false, .. } | Gesture::Resize { start, moved: false, .. } if within_slop(ev, *start) => Ok(()),
            Gesture::Move { start, ids, targets, duplicate, .. } => {
                let mut d = ev.p() - *start;
                if ev.shift {
                    if d.x.abs() > d.y.abs() {
                        d.y = 0.0;
                    } else {
                        d.x = 0.0;
                    }
                }
                // Cmd/Ctrl (and Control on macOS) held while dragging suspends snapping.
                let snap_on = self.ds().map(|ds| ds.view.snap_objects).unwrap_or(true) && !ev.cmd && !ev.ctrl;
                let mut guides = Vec::new();
                if snap_on {
                    let (c, g2) = targets.snap_move(d, SNAP_PX / ev.zoom.max(1e-6));
                    d += c;
                    guides = g2;
                }
                if self.ds().map(|ds| ds.view.snap_pixel).unwrap_or(true) {
                    // Keep the box's origin on whole pixels (a rotated parent frame's, inside one).
                    d = targets.round_move(d);
                }
                let ids = ids.clone();
                let dup = *duplicate;
                let page = self.page()?;
                let mut drop_cell = None;
                let mut drop_target = None;
                let mut in_flow = false;
                // Placed in a row or column: (is it a column, its gap) for the insertion line.
                let mut slot_axis: Option<(bool, f64)> = None;
                self.preview(|doc, sel| {
                    let mut moving = ids.clone();
                    if dup {
                        let mut copies = Vec::new();
                        for i in &ids {
                            let parent = doc.get(*i)?.parent.unwrap_or(page);
                            let idx = doc.index_in_parent(*i).map(|x| x + 1);
                            copies.push(crate::components::copy_in_file(doc, *i, parent, idx)?);
                        }
                        // Connectors copied with what they join join the copies.
                        let map = crate::board::copy_map(doc, &ids, &copies);
                        crate::board::rejoin(doc, &crate::board::copied_items(doc, &map), &map, false);
                        moving = copies;
                        *sel = moving.clone();
                    }
                    let p = ev.p();
                    let home = moving.first().and_then(|i| doc.node(*i)).and_then(|n| n.parent);
                    // Absolute children of an auto layout frame stay in it, and so do the layers of
                    // groups, boolean groups and instances ([`crate::drop::keeps_layers`]); the
                    // rest may change parent.
                    let flow = moving.iter().all(|i| doc.node(*i).is_some_and(|n| !n.child_layout.absolute))
                        && !home.is_some_and(|h| crate::drop::keeps_layers(doc, h));
                    let target = if flow { crate::drop::drag_parent(doc, page, p, &moving) } else { home.unwrap_or(page) };
                    let mode = doc.node(target).and_then(|n| n.layout.as_ref()).map(|l| l.mode);
                    if let (true, Some(mode)) = (flow, mode) {
                        // Into (or within) an auto layout frame: the layers take the slot (a grid: the
                        // cell) under the pointer, as the frame shows it now. A frame other than the
                        // one they come from is outlined too.
                        let cell = if mode == LayoutMode::Grid { crate::drop::grid_cell_at(doc, target, p) } else { None };
                        let idx =
                            if mode == LayoutMode::Grid { 0 } else { crate::drop::auto_layout_slot(doc, target, p, &moving).map_or(0, |(i, _)| i) };
                        for m in &moving {
                            if doc.get(*m)?.parent != Some(target) {
                                doc.reparent(*m, target, None)?;
                            }
                        }
                        if let (Some(first), Some((col, row, rect))) = (moving.first().copied(), cell) {
                            let cl = &mut doc.get_mut(first)?.child_layout;
                            cl.grid_column = Some(col);
                            cl.grid_row = Some(row);
                            drop_cell = Some(rect);
                        } else if mode != LayoutMode::Grid {
                            let mut kids: Vec<NodeId> = doc.get(target)?.children.iter().copied().filter(|c| !moving.contains(c)).collect();
                            for (k, m) in moving.iter().enumerate() {
                                kids.insert((idx + k).min(kids.len()), *m);
                            }
                            let gap = doc.node(target).and_then(|n| n.layout.as_ref()).map_or(0.0, |l| l.gap);
                            doc.get_mut(target)?.children = kids;
                            slot_axis = Some((mode == LayoutMode::Vertical, gap));
                        }
                        drop_target = (Some(target) != home).then_some(target);
                        in_flow = true;
                        return Ok(());
                    }
                    // A free move. Layers dragged out of an auto layout flow leave it (onto the page
                    // while dragging; the drop puts them in the frame under the pointer).
                    for m in &moving {
                        let in_al = doc.node(*m).and_then(|n| n.parent).and_then(|p| doc.node(p)).is_some_and(|p| p.layout.is_some());
                        if flow && in_al {
                            doc.reparent(*m, page, None)?;
                        }
                        move_world(doc, *m, d.x, d.y)?;
                    }
                    drop_target = (flow && target != page).then_some(target);
                    Ok(())
                })?;
                let placed = if dup { self.selection() } else { ids };
                // Snapping guides mean nothing while the layout places the layers; with Alt held,
                // live distances to the neighbours replace them.
                self.tool.overlay.guides = if in_flow {
                    Vec::new()
                } else if ev.alt {
                    let doc = self.doc()?;
                    doc.bounds_of(&placed).map(|b| crate::distance::neighbours(doc, page, &placed, b)).unwrap_or_default()
                } else {
                    guides
                };
                // The preview already shows the layers in their slot: the line marks it there, in
                // the middle of the gap before them (clear of the selection box).
                self.tool.overlay.insert_line = slot_axis.and_then(|(vertical, gap)| {
                    let off = if gap.is_finite() { (gap / 2.0).clamp(3.0, 24.0) } else { 3.0 };
                    self.doc().ok()?.bounds_of(&placed).map(|b| crate::drop::line_before(b, vertical, off))
                });
                self.tool.overlay.drop_cell = drop_cell;
                self.tool.overlay.drop_target = drop_target;
                self.tool.overlay.cursor = if self.refused(&placed, true) {
                    Cursor::NotAllowed
                } else if dup {
                    Cursor::Copy
                } else {
                    Cursor::Default
                };
                self.tool.gesture = g.clone();
                if let Gesture::Move { moved, .. } = &mut self.tool.gesture {
                    *moved = true;
                }
                return Ok(());
            }
            Gesture::Resize { start, r, scale_tool, .. } => {
                let res = self.resize_drag(ev, *start, r, *scale_tool);
                self.tool.gesture = Gesture::Resize { start: *start, r: r.clone(), scale_tool: *scale_tool, moved: true };
                if self.refused(&self.selection(), false) {
                    self.tool.overlay.cursor = Cursor::NotAllowed;
                }
                return res;
            }
            Gesture::Rotate { center, start_angle, ids } => {
                let mut a = ((ev.p() - *center).atan2() - start_angle).to_degrees();
                if ev.shift {
                    a = (a / 15.0).round() * 15.0;
                }
                let (c, ids) = (*center, ids.clone());
                self.preview(|doc, _| {
                    for i in &ids {
                        crate::cmd::arrange::rotate_world(doc, *i, c, -a)?;
                    }
                    Ok(())
                })?;
                // The cursor turns with the pointer about the centre.
                self.tool.overlay.cursor =
                    if self.refused(&ids, false) { Cursor::NotAllowed } else { Cursor::Rotate((ev.p() - c).atan2().to_degrees()) };
                Ok(())
            }
            Gesture::Draw { start, id, parent, targets } => {
                let (start, id, parent, targets) = (*start, *id, *parent, targets.clone());
                self.tool.gesture = g;
                return self.draw_drag(ev, start, id, parent, &targets);
            }
            Gesture::TextBox { id, start, line } => {
                let (id, start, line) = (*id, *start, *line);
                self.tool.gesture = g;
                return self.text_box_drag(ev, id, start, line);
            }
            Gesture::Arc { id, handle, arc } => {
                let (id, handle, arc) = (*id, *handle, *arc);
                self.tool.gesture = g;
                return self.arc_drag(ev, id, handle, arc);
            }
            Gesture::Shape { id, handle } => {
                let (id, handle) = (*id, *handle);
                let doc = self.doc()?.clone();
                if let Some(props) = crate::shape_handles::drag(&doc, id, handle, ev.p(), ev.alt) {
                    self.preview(|doc, _| crate::cmd::props::apply("node.set", doc, id, &props))?;
                }
                self.tool.overlay.shape = shape_hint(self.doc()?, id, handle, ev.zoom);
                self.tool.overlay.shape_hover = Some(handle);
                Ok(())
            }
            Gesture::Pan { sx, sy, cx, cy } => {
                let ds = self.ds_mut()?;
                let z = ds.view.zoom.max(1e-6);
                ds.view.cx = cx - (ev.sx - sx) / z;
                ds.view.cy = cy - (ev.sy - sy) / z;
                ds.revision += 1;
                Ok(())
            }
            Gesture::Pencil { points, pressure } => {
                let (mut pts, mut pressure) = (points.clone(), pressure.clone());
                if pts.last().is_none_or(|l| (*l - ev.p()).hypot() > 0.5 / ev.zoom.max(1e-6)) {
                    pts.push(ev.p());
                    pressure.push(ev.pressure);
                }
                self.tool.overlay.pen = pts.iter().map(|p| [p.x, p.y]).collect();
                self.tool.gesture = Gesture::Pencil { points: pts, pressure };
                if let Ok(ds) = self.ds_mut() {
                    ds.revision += 1;
                }
                return Ok(());
            }
            Gesture::TextSelect { id } => {
                let id = *id;
                let idx = self.text_index_at(id, ev.p())?;
                textedit::set_caret(self, idx, true)?;
                Ok(())
            }
            Gesture::Vector(_) | Gesture::ShapeBuild { .. } | Gesture::Origin => Ok(()),
            Gesture::Connect { from, start } => {
                let (from, start) = (*from, *start);
                self.connect_drag(ev, from, start);
                return Ok(());
            }
            Gesture::Erase { gone } => {
                self.tool.gesture = Gesture::Erase { gone: gone.clone() };
                return self.erase_at(ev);
            }
            Gesture::PenDrag { anchor } => {
                // A click's wobble pulls out no handle.
                let out = if within_slop(ev, *anchor) { Vec2::ZERO } else { ev.p() - *anchor };
                if let Some(pen) = self.tool.pen.as_mut()
                    && let Some(last) = pen.points.last_mut()
                {
                    last.2 = out;
                    last.1 = -out;
                }
                self.pen_update()?;
                Ok(())
            }
        };
        self.tool.gesture = g;
        res
    }

    /// An arc handle drag: the new arc, written through `node.set`'s keys.
    fn arc_drag(&mut self, ev: &PointerEvent, id: NodeId, handle: ArcHandle, arc: crate::arc_handles::Arc) -> Result<()> {
        let doc = self.doc()?.clone();
        let Some(next) = crate::arc_handles::drag(&doc, id, handle, arc, ev.p(), ev.shift, ev.zoom) else { return Ok(()) };
        let props = crate::arc_handles::props(next);
        self.preview(|doc, _| crate::cmd::props::apply("node.set", doc, id, &props))?;
        self.tool.gesture = Gesture::Arc { id, handle, arc: next };
        let at = crate::arc_handles::handle_at(self.doc()?, id, handle, next, ev.zoom);
        self.tool.overlay.arc = at.map(|at| ArcHint { handle, label: next.label(handle), at });
        self.tool.overlay.arc_hover = Some(handle);
        Ok(())
    }

    fn resize_drag(&mut self, ev: &PointerEvent, start: Point, r: &ResizeStart, scale_tool: bool) -> Result<()> {
        let page = self.page()?;
        let snap = self.ds().map(|d| d.view.snap_objects).unwrap_or(true) && !ev.cmd && !ev.ctrl;
        let ids: Vec<NodeId> = r.nodes.iter().map(|n| n.0).collect();
        let targets = if snap { Some(Targets::gather(self.doc()?, page, &ids)) } else { None };
        let mut p = ev.p();
        let mut guides = Vec::new();
        if let Some(t) = &targets {
            let (dx, dy, g) = t.snap_point(p.x, p.y, SNAP_PX / ev.zoom.max(1e-6));
            p.x += dx;
            p.y += dy;
            guides = g;
        }
        let _ = start;
        // In the handle box's own axes (one layer: its local space).
        let (frame, w0, h0) = r.frame;
        // The pixel grid: an upright box's dragged sides land on whole pixels (the Scale tool
        // scales by the factor as dragged).
        if !scale_tool && self.ds().map(|d| d.view.snap_pixel).unwrap_or(true) && crate::guides::axis_aligned(frame) {
            p = Point::new(p.x.round(), p.y.round());
        }
        let q = frame.inverse() * p;
        let is_line = r.single && ids.first().and_then(|i| self.doc().ok()?.node(*i).map(|n| matches!(n.kind, NodeKind::Line))).unwrap_or(false);
        if scale_tool {
            return self.scale_drag(ev, r, q, guides);
        }
        if is_line {
            // Line end handles: move the end point freely (length + angle).
            let (a, b) = match r.handle {
                Handle::Corner(0) => (p, frame * Point::new(w0, 0.0)),
                _ => (frame * Point::new(0.0, 0.0), p),
            };
            let id = ids.first().copied().unwrap_or(NodeId(0));
            self.preview(|doc, _| {
                let mut v = b - a;
                if ev.shift {
                    let ang = (v.atan2() / (std::f64::consts::PI / 4.0)).round() * std::f64::consts::PI / 4.0;
                    v = Vec2::from_angle(ang) * v.hypot();
                }
                let len = v.hypot();
                let world = Affine::translate(a.to_vec2()) * Affine::rotate(v.atan2());
                doc.set_world_transform(id, world)?;
                figcraft_layout::set_size(doc, id, len, 0.0);
                Ok(())
            })?;
            self.tool.overlay.guides = guides;
            return Ok(());
        }
        // Constrain proportions (Shift, or the node's lock).
        let keep = ev.shift || r.single && ids.first().and_then(|i| self.doc().ok()?.node(*i).map(|n| n.constrain_proportions)).unwrap_or(false);
        let (x0, y0, x1, y1) = resized_box(r.handle, q, w0, h0, keep, ev.alt);
        let (flip_x, flip_y) = (x1 < x0, y1 < y0);
        let (nx0, ny0) = (x0.min(x1), y0.min(y1));
        let (nw, nh) = ((x1 - x0).abs(), (y1 - y0).abs());
        let nodes = r.nodes.clone();
        let single = r.single;
        let handle = r.handle;
        // What's in a section stays where it is on the page, and so does what's in a frame resized
        // with Cmd/Ctrl held (its constraints are ignored).
        let keeps = |doc: &Document, id: NodeId| {
            doc.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Section { .. }) || (ev.cmd && single && n.kind.is_frame_like()))
        };
        self.preview(|doc, _| {
            if single {
                let Some((id, world, _, _)) = nodes.first() else { return Ok(()) };
                let kept = if keeps(doc, *id) { crate::cmd::props::children_worlds(doc, *id) } else { Vec::new() };
                let mut t = *world * Affine::translate((nx0, ny0));
                if flip_x {
                    t *= Affine::new([-1.0, 0.0, 0.0, 1.0, nw, 0.0]);
                }
                if flip_y {
                    t *= Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, nh]);
                }
                doc.set_world_transform(*id, t)?;
                if kept.is_empty() {
                    crate::cmd::props::resize_node(doc, *id, nw.max(0.01), nh.max(0.01))?;
                } else {
                    figcraft_layout::set_size(doc, *id, nw.max(0.01), nh.max(0.01));
                    crate::cmd::props::restore_worlds(doc, &kept)?;
                }
                let n = doc.get_mut(*id)?;
                if matches!(handle, Handle::Corner(_) | Handle::Edge(1) | Handle::Edge(3)) {
                    n.child_layout.sizing_h = Sizing::Fixed;
                }
                if matches!(handle, Handle::Corner(_) | Handle::Edge(0) | Handle::Edge(2)) {
                    n.child_layout.sizing_v = Sizing::Fixed;
                }
                if let NodeKind::Text(t) = &mut n.kind {
                    t.auto_resize = match handle {
                        Handle::Edge(1) | Handle::Edge(3) => TextAutoResize::Height,
                        _ => TextAutoResize::None,
                    };
                }
                return Ok(());
            }
            // Several layers: map the old box onto the new one, in the box's axes. Each layer's
            // centre follows the box (mirrored when it flips); each keeps its rotation and scales
            // along its own axes (see `axis_factors`), turning over with the box.
            let (sx, sy) = (if w0 > 1e-9 { (x1 - x0) / w0 } else { 1.0 }, if h0 > 1e-9 { (y1 - y0) / h0 } else { 1.0 });
            let mirror = Affine::new([sx.signum(), 0.0, 0.0, sy.signum(), 0.0, 0.0]);
            let inv = frame.inverse();
            for (id, world, w, h) in &nodes {
                let local = inv * *world;
                let center = local * Point::new(w / 2.0, h / 2.0);
                let (kw, kh) = figcraft_layout::axis_factors(local, sx, sy);
                let (w2, h2) = (w * kw, h * kh);
                let lin = (mirror * local).with_translation(Vec2::ZERO);
                let origin = Point::new(x0 + center.x * sx, y0 + center.y * sy) - lin * Point::new(w2 / 2.0, h2 / 2.0);
                let kept = if keeps(doc, *id) { crate::cmd::props::children_worlds(doc, *id) } else { Vec::new() };
                doc.set_world_transform(*id, frame * lin.with_translation(origin))?;
                crate::cmd::props::resize_node(doc, *id, w2, h2)?;
                crate::cmd::props::restore_worlds(doc, &kept)?;
            }
            Ok(())
        })?;
        self.tool.overlay.guides = guides;
        Ok(())
    }

    /// Scale tool handle drag (`q`: the snapped pointer in the handle box's axes): everything
    /// scales by one factor about the anchor point (Alt: the centre), strokes, effects, radii and
    /// text included, children by the same factor whatever their constraints.
    fn scale_drag(&mut self, ev: &PointerEvent, r: &ResizeStart, q: Point, guides: Vec<Guide>) -> Result<()> {
        let (frame, w0, h0) = r.frame;
        let Some((k, pivot)) = scale_factor(r.handle, q, w0, h0, self.tool.scale_anchor, ev.alt) else { return Ok(()) };
        let nodes = r.nodes.clone();
        let inv = frame.inverse();
        // A layer's place in the box scales about the pivot; a negative factor turns it over.
        let about = frame * Affine::translate(pivot.to_vec2() * (1.0 - k));
        let sign = k.signum();
        self.preview(|doc, _| {
            for (id, world, _, _) in &nodes {
                let c = (inv * *world).as_coeffs();
                doc.set_world_transform(*id, about * Affine::new([sign * c[0], sign * c[1], sign * c[2], sign * c[3], k * c[4], k * c[5]]))?;
                crate::cmd::arrange::scale_node(doc, *id, k.abs())?;
            }
            Ok(())
        })?;
        let p = frame * pivot;
        self.tool.overlay.scale = Some(ScaleHint { factor: k, pivot: [p.x, p.y] });
        self.tool.overlay.guides = guides;
        Ok(())
    }

    fn draw_down(&mut self, ev: &PointerEvent) -> Result<()> {
        let ds = self.ds()?;
        let doc = ds.doc.clone();
        let page = ds.page;
        if ds.text_edit.is_some() {
            textedit::end(self)?;
        }
        // Frames drawn inside frames nest; sections only at the top or in sections.
        let parent = match self.tool.tool {
            ToolId::Section => {
                frame_at(&doc, page, ev.p(), &[]).filter(|f| doc.node(*f).is_some_and(|n| matches!(n.kind, NodeKind::Section { .. }))).unwrap_or(page)
            }
            _ => frame_at(&doc, page, ev.p(), &[]).unwrap_or(page),
        };
        let targets = Targets::gather(&doc, page, &[]);
        self.begin_interaction(self.tool.tool.label())?;
        self.tool.gesture = Gesture::Draw { start: ev.p(), id: None, parent, targets };
        Ok(())
    }

    fn draw_drag(&mut self, ev: &PointerEvent, start: Point, id: Option<NodeId>, parent: NodeId, targets: &Targets) -> Result<()> {
        let tool = self.tool.tool;
        let mut p = ev.p();
        let mut guides = Vec::new();
        if self.ds().map(|d| d.view.snap_objects).unwrap_or(true) && !ev.cmd && !ev.ctrl {
            let (dx, dy, g) = targets.snap_point(p.x, p.y, SNAP_PX / ev.zoom.max(1e-6));
            p.x += dx;
            p.y += dy;
            guides = g;
        }
        let pixel = self.ds().map(|d| d.view.snap_pixel).unwrap_or(true);
        let mut created = id;
        let page = self.page()?;
        self.preview(|doc, sel| {
            let (mut a, mut b) = (start, p);
            if pixel {
                a = Point::new(a.x.round(), a.y.round());
                b = Point::new(b.x.round(), b.y.round());
            }
            let pw = doc.world_transform(parent);
            let (kind, base) = match tool {
                ToolId::Frame => (NodeKind::Frame, "Frame"),
                ToolId::Section => (NodeKind::Section { contents_hidden: false }, "Section"),
                ToolId::Slice => (NodeKind::Slice, "Slice"),
                ToolId::Rectangle => (NodeKind::Rectangle, "Rectangle"),
                ToolId::Ellipse => (NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "Ellipse"),
                ToolId::Polygon => (NodeKind::Polygon { count: 3 }, "Polygon"),
                ToolId::Star => (NodeKind::Star { count: 5, inner_ratio: 0.38 }, "Star"),
                ToolId::Line => (NodeKind::Line, "Line"),
                ToolId::Arrow => (NodeKind::Line, "Arrow"),
                _ => return Ok(()),
            };
            let name = match tool {
                ToolId::Frame | ToolId::Section | ToolId::Slice => doc.next_name(base),
                _ => base.to_string(),
            };
            let nid = doc.alloc_id();
            let mut n = if tool == ToolId::Section { Node::new_section(nid, name, 0.0, 0.0) } else { Node::new(nid, kind, name, 0.0, 0.0) };
            if tool == ToolId::Arrow {
                n.stroke.cap_end = StrokeCap::LineArrow;
            }
            if matches!(tool, ToolId::Line | ToolId::Arrow) {
                let mut v = b - a;
                if ev.shift {
                    let ang = (v.atan2() / (std::f64::consts::PI / 4.0)).round() * std::f64::consts::PI / 4.0;
                    v = Vec2::from_angle(ang) * v.hypot();
                }
                // Alt draws from the centre, as for the other shapes.
                let (a, v) = if ev.alt { (a - v, v * 2.0) } else { (a, v) };
                n.width = v.hypot();
                n.transform = pw.inverse() * Affine::translate(a.to_vec2()) * Affine::rotate(v.atan2());
            } else {
                let mut d = b - a;
                if ev.shift {
                    let m = d.x.abs().max(d.y.abs());
                    d = Vec2::new(m * d.x.signum(), m * d.y.signum());
                }
                let (x0, y0, x1, y1) = if ev.alt { (a.x - d.x, a.y - d.y, a.x + d.x, a.y + d.y) } else { (a.x, a.y, a.x + d.x, a.y + d.y) };
                let r = Rect::new(x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1));
                n.width = r.width();
                n.height = r.height();
                n.transform = pw.inverse() * Affine::translate((r.x0, r.y0));
            }
            doc.insert(n, parent, None)?;
            *sel = vec![nid];
            created = Some(nid);
            let _ = page;
            Ok(())
        })?;
        self.tool.overlay.guides = guides;
        self.tool.gesture = Gesture::Draw { start, id: created, parent, targets: targets.clone() };
        Ok(())
    }

    fn up(&mut self, ev: &PointerEvent) -> Result<()> {
        let g = std::mem::take(&mut self.tool.gesture);
        self.tool.overlay.marquee = None;
        self.tool.overlay.guides.clear();
        self.tool.overlay.scale = None;
        self.tool.overlay.arc = None;
        self.tool.overlay.shape = None;
        self.tool.overlay.measure_box = None;
        self.tool.overlay.insert_line = None;
        self.tool.overlay.drop_cell = None;
        let drop_target = self.tool.overlay.drop_target.take();
        match g {
            Gesture::Move { ids, moved, duplicate, start, deselect, .. } => {
                if moved {
                    // A free move lands in the frame under the pointer (or the page) where it was
                    // dropped; layers in an auto layout frame already sit in their slot.
                    let page = self.page()?;
                    let sel = self.selection();
                    let sources = if duplicate { ids.clone() } else { Vec::new() };
                    let moving = if duplicate { sel } else { ids };
                    let doc = self.doc()?.clone();
                    let in_al =
                        moving.iter().any(|m| doc.node(*m).and_then(|n| n.parent).and_then(|p| doc.node(p)).is_some_and(|p| p.layout.is_some()));
                    let target = drop_target.unwrap_or(page);
                    let current = moving.first().and_then(|m| doc.node(*m)).and_then(|n| n.parent);
                    if !in_al && current != Some(target) {
                        let ds = self.ds_mut()?;
                        let before = ds.doc.clone();
                        let d = ds.doc_mut();
                        for m in &moving {
                            // Group, boolean group and instance layers stay put: they only move.
                            let from = d.node(*m).and_then(|n| n.parent);
                            if from.is_some_and(|f| crate::drop::keeps_layers(d, f)) || crate::components::structure_locked(d, target) {
                                continue;
                            }
                            // A layer can't go into itself; it stays where it is.
                            let _ = d.reparent(*m, target, None);
                        }
                        crate::normalize(ds.doc_mut(), &before);
                    }
                    // Reordering or dragging layers out of an instance (outside its slots), or
                    // moving layers inside one, are edits only its main can make: the gesture is
                    // undone.
                    if self.refused(&moving, true) {
                        self.cancel_interaction()?;
                    } else {
                        self.commit_interaction()?;
                        // Duplicate repeats a free Alt-drag's step (an auto layout places its own).
                        if duplicate && !in_al && moving.len() == sources.len() {
                            self.ds_mut()?.last_duplicate = Some(crate::cmd::edit::Duplicated { sources, copies: moving });
                        }
                    }
                } else {
                    self.cancel_interaction()?;
                    if let Some(d) = deselect {
                        let sel = self.selection().into_iter().filter(|x| *x != d).collect();
                        self.set_selection(sel);
                    }
                    if deselect.is_none() && !(ev.shift || ev.cmd || duplicate) {
                        self.widget_click(start, ev.tol() * 2.0)?;
                    }
                }
            }
            Gesture::Resize { .. } | Gesture::Rotate { .. } | Gesture::Arc { .. } | Gesture::Shape { .. } => {
                // Layers inside an instance keep their main's size and rotation.
                if self.refused(&self.selection(), false) {
                    self.cancel_interaction()?;
                } else {
                    self.commit_interaction()?;
                }
            }
            Gesture::Draw { start, id, parent, .. } => {
                let tiny = (ev.p() - start).hypot() * ev.zoom < 3.0;
                if tiny || id.is_none() {
                    // Click without drag: Figma makes a 100×100 shape (frames 100×100 too).
                    self.cancel_interaction()?;
                    self.begin_interaction(self.tool.tool.label())?;
                    let (w, h) = self.tool.tool.click_size();
                    let target = start + Vec2::new(w, h);
                    let mut fake = ev.clone();
                    fake.x = target.x;
                    fake.y = target.y;
                    fake.shift = false;
                    fake.alt = false;
                    self.tool.gesture = Gesture::None;
                    let targets = Targets::default();
                    self.draw_drag(&fake, start, None, parent, &targets)?;
                    self.tool.gesture = Gesture::None;
                }
                if matches!(self.tool.tool, ToolId::Frame | ToolId::Section)
                    && let Some(drawn) = self.selection().first().copied()
                {
                    let ds = self.ds_mut()?;
                    let before = ds.doc.clone();
                    take_enclosed(ds.doc_mut(), drawn)?;
                    crate::normalize(ds.doc_mut(), &before);
                }
                self.commit_interaction()?;
                if !self.prefs.keep_tool {
                    self.set_tool(ToolId::Move);
                }
            }
            Gesture::Pan { .. } => {
                self.tool.overlay.cursor = Cursor::Grab;
                if let Some(t) = self.tool.temp_from.take() {
                    self.tool.tool = t;
                }
            }
            Gesture::Pencil { mut points, mut pressure } => {
                self.tool.overlay.pen.clear();
                // The stroke ends where the button came up, even without a move to get there.
                if points.last().is_none_or(|l| (*l - ev.p()).hypot() > 0.5 / ev.zoom.max(1e-6)) {
                    points.push(ev.p());
                    pressure.push(ev.pressure);
                }
                self.pencil_finish(&points, &pressure)?;
            }
            Gesture::Connect { from, start } => self.connect_up(ev, from, start)?,
            Gesture::Erase { .. } => self.erase_up()?,
            Gesture::TextSelect { .. } | Gesture::TextBox { .. } | Gesture::Marquee { .. } | Gesture::None => {}
            Gesture::PenDrag { .. } | Gesture::ShapeBuild { .. } | Gesture::Origin => {}
            Gesture::Vector(_) => self.commit_interaction()?,
        }
        if let Ok(ds) = self.ds_mut() {
            ds.revision += 1;
        }
        Ok(())
    }

    fn double_click(&mut self, ev: &PointerEvent) -> Result<()> {
        let ds = self.ds()?;
        let (doc, sel, page) = (ds.doc.clone(), ds.selection.clone(), ds.page);
        if self.tool.tool == ToolId::Pen {
            return self.pen_finish();
        }
        let edits = !self.tool.dev_mode;
        // A frame's or section's name label: rename it there (the canvas puts a field over it).
        // Labels come before widgets, as on a press; the second press began a move: undo it.
        if edits
            && matches!(self.tool.tool, ToolId::Move | ToolId::Scale)
            && !(ev.shift || ev.cmd || ev.alt)
            && let Some(id) = label_at(&doc, page, &self.tool.labels, ev.p())
        {
            self.tool.gesture = Gesture::None;
            self.cancel_interaction()?;
            if sel != [id] {
                self.set_selection(vec![id]);
            }
            self.request_ui("canvas.renameLabel", serde_json::json!({"id": id}));
            return Ok(());
        }
        // Quick clicks on a widget's clickable part are clicks, one by one.
        if edits
            && matches!(self.tool.tool, ToolId::Move | ToolId::Scale)
            && !(ev.shift || ev.cmd || ev.alt)
            && self.hit(&doc, page, ev.p(), ev.tol() * 2.0).and_then(|hit| crate::widgets::target_at(&doc, hit, ev.p())).is_some()
        {
            self.tool.gesture = Gesture::None;
            self.cancel_interaction()?;
            self.widget_click(ev.p(), ev.tol() * 2.0)?;
            return Ok(());
        }
        // The second press began a move, a resize, a shape or a marquee: a double-click isn't one.
        if matches!(self.tool.gesture, Gesture::Move { .. } | Gesture::Resize { .. } | Gesture::Draw { .. } | Gesture::Marquee { .. }) {
            self.tool.gesture = Gesture::None;
            self.tool.overlay.marquee = None;
            self.cancel_interaction()?;
        }
        let Some(leaf) = self.hit(&doc, page, ev.p(), ev.tol() * 2.0) else { return Ok(()) };
        if matches!(self.tool.tool, ToolId::Move | ToolId::Scale) && self.board_double_click(&doc, leaf, &sel)? {
            return Ok(());
        }
        let target = select_target(&doc, leaf, &sel, true);
        // Text: edit at the click.
        if edits
            && doc.node(target).is_some_and(|n| matches!(n.kind, NodeKind::Text(_)))
            && (sel.contains(&target) || select_target(&doc, leaf, &sel, false) == target)
        {
            // Entering a text selects all of it; in the text being edited, the word.
            let editing = self.ds()?.text_edit.as_ref().is_some_and(|te| te.id == target);
            let idx = self.text_index_at(target, ev.p())?;
            textedit::begin(self, target, Some(idx), false)?;
            if editing {
                textedit::select_word(self, idx)?;
            } else {
                textedit::key(self, "SelectAll", false, false, false)?;
            }
            return Ok(());
        }
        // Shapes and vectors: enter vector edit mode (every selected shape, the clicked one active).
        if edits && doc.node(target).is_some_and(|n| n.kind.is_shape()) && sel.contains(&target) {
            let mut ids = crate::vectoredit::enter_targets(&doc, &sel).unwrap_or_default();
            ids.retain(|i| *i != target);
            ids.insert(0, target);
            return crate::vectoredit::begin_many(self, &ids);
        }
        // Groups, frames and boolean groups: select one level deeper (the layer under the pointer).
        let deeper = figcraft_doc::geometry::enter_target(&doc, leaf, &sel);
        if deeper != target || !sel.contains(&deeper) {
            self.set_selection(vec![deeper]);
        }
        Ok(())
    }

    /// A plain click at `p` on a clickable part of a widget calls its plugin back (a failure is
    /// shown as a message).
    fn widget_click(&mut self, p: Point, tol: f64) -> Result<()> {
        let (doc, page) = (self.doc()?.clone(), self.page()?);
        let Some((id, target)) = self.hit(&doc, page, p, tol).and_then(|hit| crate::widgets::target_at(&doc, hit, p)) else { return Ok(()) };
        if let Err(e) = self.execute("widget.click", &serde_json::json!({"id": id, "target": target})) {
            self.message = Some(e.to_string());
        }
        Ok(())
    }

    /// The char index in text node `id` at page point `p`.
    pub fn text_index_at(&self, id: NodeId, p: Point) -> Result<usize> {
        let doc = self.doc()?;
        let n = doc.get(id)?;
        let local = doc.world_transform(id).inverse() * p;
        let Some((layout, dy)) = figcraft_text::layout_node(n) else { return Ok(0) };
        Ok(layout.hit(local.x, local.y - dy))
    }

    fn text_click(&mut self, id: NodeId, ev: &PointerEvent, extend: bool) -> Result<()> {
        let idx = self.text_index_at(id, ev.p())?;
        textedit::set_caret(self, idx, extend)?;
        self.tool.gesture = Gesture::TextSelect { id };
        Ok(())
    }

    fn text_down(&mut self, ev: &PointerEvent) -> Result<()> {
        let ds = self.ds()?;
        let (doc, page) = (ds.doc.clone(), ds.page);
        if let Some(te) = ds.text_edit.clone() {
            if doc.world_bounds(te.id).is_some_and(|b| b.contains(ev.p())) {
                return self.text_click(te.id, ev, ev.shift);
            }
            textedit::end(self)?;
        }
        // Clicking existing text edits it.
        if let Some(leaf) = self.hit(&doc, page, ev.p(), ev.tol() * 2.0)
            && doc.node(leaf).is_some_and(|n| matches!(n.kind, NodeKind::Text(_)))
        {
            let idx = self.text_index_at(leaf, ev.p())?;
            textedit::begin(self, leaf, Some(idx), false)?;
            self.tool.gesture = Gesture::TextSelect { id: leaf };
            return Ok(());
        }
        // New text at the click (Figma puts the baseline-ish top-left there).
        let parent = frame_at(&doc, page, ev.p(), &[]).unwrap_or(page);
        let ds = self.ds_mut()?;
        let d = ds.doc_mut();
        let id = d.alloc_id();
        let mut n = Node::new(id, NodeKind::Text(TextData::default()), "Text", 1.0, 14.0);
        let pw = d.world_transform(parent);
        let at = Point::new(ev.x.round(), (ev.y - 7.0).round());
        n.transform = pw.inverse() * Affine::translate(at.to_vec2());
        n.name = String::new();
        if let Some(fills) = readable_text_fills(d, parent) {
            n.fills = fills;
        }
        if let Some((w, h)) = figcraft_text::auto_size(&n) {
            n.width = w.max(1.0);
            n.height = h;
        }
        let line = n.height;
        d.insert(n, parent, None)?;
        ds.selection = vec![id];
        textedit::begin(self, id, Some(0), true)?;
        self.tool.gesture = Gesture::TextBox { id, start: ev.p(), line };
        if !self.prefs.keep_tool {
            self.tool.tool = ToolId::Move;
        }
        Ok(())
    }

    /// Drag out a new text box: past the drag slop it has the drag's size (at least one line
    /// tall) and its top-left, and that size is fixed.
    fn text_box_drag(&mut self, ev: &PointerEvent, id: NodeId, start: Point, line: f64) -> Result<()> {
        if within_slop(ev, start) {
            return Ok(());
        }
        let r = Rect::from_points(start, ev.p());
        let d = self.ds_mut()?.doc_mut();
        let pw = d.node(id).and_then(|n| n.parent).map(|p| d.world_transform(p)).unwrap_or_default();
        let n = d.get_mut(id)?;
        n.transform = pw.inverse() * Affine::translate((r.x0.round(), r.y0.round()));
        n.width = r.width().round().max(1.0);
        n.height = r.height().round().max(line);
        n.child_layout.sizing_h = Sizing::Fixed;
        n.child_layout.sizing_v = Sizing::Fixed;
        if let NodeKind::Text(t) = &mut n.kind {
            t.auto_resize = TextAutoResize::None;
        }
        self.ds_mut()?.revision += 1;
        Ok(())
    }

    fn eyedrop(&mut self, ev: &PointerEvent) -> Result<()> {
        let ds = self.ds()?;
        let doc = ds.doc.clone();
        let view = figcraft_render::View { origin: (ev.x, ev.y), scale: ev.zoom.max(0.01), width: 1, height: 1 };
        let Some(pix) = figcraft_render::render_view(&doc, ds.page, &view, &Default::default()) else { return Ok(()) };
        let rgba = figcraft_render::to_rgba8(&pix);
        let c = figcraft_color::Color::from_rgba8(
            rgba.first().copied().unwrap_or(0),
            rgba.get(1).copied().unwrap_or(0),
            rgba.get(2).copied().unwrap_or(0),
            255,
        );
        let sel = ds.selection.clone();
        if self.tool.pick_only {
            self.tool.picked = Some(c);
        } else if !sel.is_empty() {
            self.execute("node.set", &json!({"ids": sel, "props": {"fill": c.to_css_hex()}}))?;
        }
        self.recent_colors.retain(|x| *x != c);
        self.recent_colors.insert(0, c);
        self.recent_colors.truncate(16);
        self.message = Some(format!("Picked #{}", c.to_hex()));
        self.set_tool(ToolId::Move);
        Ok(())
    }

    /// The pen tool's snapped position for `ev`: its path's vector (or, before it has one, its first
    /// point) counts among the targets.
    fn snap_pen_tool(&mut self, ev: &PointerEvent) -> Result<(Point, Vec<Guide>)> {
        let (id, extra) = match &self.tool.pen {
            Some(pen) if pen.id.is_some() => (pen.id, Vec::new()),
            Some(pen) => (None, pen.points.iter().map(|(p, _, _)| *p).collect()),
            None => (None, Vec::new()),
        };
        self.snap_pen(ev, id, &extra)
    }

    fn pen_down(&mut self, ev: &PointerEvent) -> Result<()> {
        self.tool.overlay.guides.clear();
        // The segment being placed shows as itself while the press lasts.
        self.tool.overlay.rubber = None;
        self.tool.overlay.rubber_handle = None;
        let p = if ev.shift { ev.p() } else { self.snap_pen_tool(ev)?.0 };
        let (doc, page) = {
            let ds = self.ds()?;
            (ds.doc.clone(), ds.page)
        };
        // With an open path selected, a click on one of its end points continues it in edit mode.
        if self.tool.pen.is_none()
            && let Some((id, v)) = selected_end_at(&doc, &self.selection(), ev.p(), 8.0 / ev.zoom.max(1e-6))
        {
            return self.pen_edit(id, Some(v));
        }
        let pen = self.tool.pen.get_or_insert_with(PenPath::default);
        // Clicking the first anchor closes the path.
        if pen.points.len() > 2
            && let Some((first, _, _)) = pen.points.first()
            && (*first - p).hypot() * ev.zoom < 8.0
        {
            pen.closed = true;
            self.pen_update()?;
            return self.pen_finish();
        }
        // Clicking another of its own anchors (not the last) connects there and ends the path.
        let near = |q: &Point| (*q - p).hypot() * ev.zoom < 8.0;
        if let Some(k) = (1..pen.points.len().saturating_sub(1)).find(|k| pen.points.get(*k).is_some_and(|(q, _, _)| near(q))) {
            pen.join = Some(k);
            self.pen_update()?;
            return self.pen_finish();
        }
        if pen.parent.is_none() {
            pen.parent = Some(frame_at(&doc, page, p, &[]).unwrap_or(page));
        }
        let mut p = p;
        if ev.shift
            && let Some((last, _, _)) = pen.points.last()
        {
            p = at_45_degrees(*last, p);
        }
        pen.points.push((p, Vec2::ZERO, Vec2::ZERO));
        self.tool.gesture = Gesture::PenDrag { anchor: p };
        self.pen_update()
    }

    /// Rebuild the pen's vector node from its anchors.
    fn pen_update(&mut self) -> Result<()> {
        let Some(pen) = self.tool.pen.clone() else { return Ok(()) };
        self.tool.overlay.pen = pen.points.iter().map(|(p, _, _)| [p.x, p.y]).collect();
        self.tool.overlay.pen_handles = pen_handles(&pen.points);
        if pen.points.len() < 2 {
            if let Ok(ds) = self.ds_mut() {
                ds.revision += 1;
            }
            return Ok(());
        }
        let page = self.page()?;
        let parent = pen.parent.unwrap_or(page);
        if pen.id.is_none() {
            self.begin_interaction("Pen")?;
        }
        let ds = self.ds_mut()?;
        let doc = ds.doc_mut();
        let pw = doc.world_transform(parent);
        let inv = pw.inverse();
        // Anchors and handles into the parent's space.
        let pts: Vec<(Point, Point, Point)> = pen.points.iter().map(|(p, i, o)| (inv * *p, inv * (*p + *i), inv * (*p + *o))).collect();
        let mut path = figcraft_geom::BezPath::new();
        for (k, (p, _, _)) in pts.iter().enumerate() {
            if k == 0 {
                path.move_to(*p);
            } else if let (Some(prev), Some(cur)) = (pts.get(k - 1), pts.get(k)) {
                path.curve_to(prev.2, cur.1, *p);
            }
        }
        if pen.closed
            && let (Some(last), Some(first)) = (pts.last(), pts.first())
        {
            path.curve_to(last.2, first.1, first.0);
            path.close_path();
        }
        use figcraft_geom::Shape;
        let bb = path.bounding_box();
        path.apply_affine(Affine::translate((-bb.x0, -bb.y0)));
        let mut net = VectorNetwork::from_path(&path);
        // Straight segments stay straight.
        for s in &mut net.segments {
            if (s.ta.0.abs() + s.ta.1.abs()) < 1e-9 && (s.tb.0.abs() + s.tb.1.abs()) < 1e-9 {
                s.ta = (0.0, 0.0);
                s.tb = (0.0, 0.0);
            }
        }
        if !pen.closed {
            net.regions.clear();
        }
        // Anchor k is vertex k of the open path.
        if let (Some(k), Some((last, _, out))) = (pen.join, pts.last()) {
            let from = pts.len() - 1;
            crate::vectoredit::ops::connect(&mut net, from, k, (out.x - last.x, out.y - last.y), (0.0, 0.0));
        }
        let id = match pen.id {
            Some(id) if doc.contains(id) => id,
            _ => {
                let id = doc.alloc_id();
                let mut n = Node::new(id, NodeKind::Vector { network: VectorNetwork::default() }, "Vector", 0.0, 0.0);
                n.fills.clear();
                n.strokes = vec![figcraft_color::Paint::solid(figcraft_color::Color::BLACK)];
                n.stroke.align = StrokeAlign::Center;
                doc.insert(n, parent, None)?;
                id
            }
        };
        let n = doc.get_mut(id)?;
        n.kind = NodeKind::Vector { network: net };
        n.width = bb.width();
        n.height = bb.height();
        n.transform = Affine::translate((bb.x0, bb.y0));
        if pen.closed && n.fills.is_empty() {
            n.fills = vec![figcraft_color::Paint::solid(figcraft_color::Color::DEFAULT_SHAPE)];
        }
        ds.selection = vec![id];
        ds.revision += 1;
        if let Some(p) = self.tool.pen.as_mut() {
            p.id = Some(id);
        }
        Ok(())
    }

    /// Edit `id` as a vector with the pen, going on from point `anchor` (or, without one, starting
    /// a new path in the same layer at the next click).
    fn pen_edit(&mut self, id: NodeId, anchor: Option<usize>) -> Result<()> {
        self.set_tool(ToolId::Move);
        crate::vectoredit::begin(self, id)?;
        crate::vectoredit::set_tool(self, crate::vectoredit::VTool::Pen)?;
        crate::vectoredit::set_state(self, |e| {
            e.anchor = anchor;
            e.vertices = anchor.into_iter().collect();
        })
    }

    /// Enter/Escape while the pen draws: the path ends but its layer stays in edit mode with the
    /// pen, so the next path joins the same vector (another Escape leaves edit mode).
    pub fn pen_end(&mut self) -> Result<()> {
        let id = self.tool.pen.as_ref().and_then(|p| p.id);
        self.pen_finish()?;
        match id {
            Some(id) if self.doc()?.contains(id) => self.pen_edit(id, None),
            _ => Ok(()),
        }
    }

    /// Finish the pen path (and keep the pen tool: double-click, Delete, a tool switch).
    pub fn pen_finish(&mut self) -> Result<()> {
        let pen = self.tool.pen.take();
        self.tool.overlay.pen.clear();
        self.tool.overlay.pen_handles.clear();
        self.tool.overlay.rubber = None;
        self.tool.overlay.rubber_handle = None;
        self.tool.gesture = Gesture::None;
        if pen.as_ref().is_some_and(|p| p.id.is_some()) {
            self.commit_interaction()?;
        }
        Ok(())
    }

    fn pencil_finish(&mut self, points: &[Point], pressure: &[Option<f32>]) -> Result<()> {
        if points.len() < 2 {
            return Ok(());
        }
        // Smooth: fit a curve through the samples.
        let mut poly = figcraft_geom::BezPath::new();
        for (k, p) in points.iter().enumerate() {
            if k == 0 {
                poly.move_to(*p);
            } else {
                poly.line_to(*p);
            }
        }
        let fitted = figcraft_geom::kurbo::simplify::simplify_bezpath(poly.iter(), 0.75, &Default::default());
        let d = fitted.to_svg();
        if matches!(self.tool.tool, ToolId::Marker | ToolId::Highlighter) {
            return self.ink_finish(&d);
        }
        let page = self.page()?;
        let parent = frame_at(self.doc()?, page, points.first().copied().unwrap_or_default(), &[]).unwrap_or(page);
        let pw = self.doc()?.world_transform(parent);
        let mut local = fitted.clone();
        local.apply_affine(pw.inverse());
        let _ = d;
        let (color, weight) = PENCIL_STROKE;
        let mut props = json!({"fills": [], "strokes": [color.to_css_hex()], "strokeWeight": weight, "strokeCap": "round", "strokeJoin": "round", "strokeAlign": "center"});
        // Pen pressure becomes the stroke's width profile (brushes follow it too).
        if let Some(points) = pressure_profile(points, pressure) {
            props["widthProfile"] = json!(points);
        }
        if let Some(b) = &self.tool.pencil_brush {
            props["brush"] = serde_json::to_value(b).unwrap_or_default();
        }
        self.execute(
            "node.create",
            &json!({"type": "vector", "path": local.to_svg(), "closed": false, "parent": parent, "name": "Vector", "props": props}),
        )?;
        Ok(())
    }

    /// Keys the current mode handles before menu shortcuts (vector edit mode's tools and point
    /// keys, Enter to finish the shape builder). Returns whether the key was used.
    pub fn tool_key(&mut self, name: &str, shift: bool, alt: bool, cmd: bool) -> Result<bool> {
        // Dev Mode edits nothing: keys go to the commands (which it refuses in the app).
        if self.tool.dev_mode {
            return Ok(false);
        }
        if self.ds().is_ok_and(|d| d.vector_edit.is_some()) {
            return crate::vectoredit::key(self, name, shift, alt, cmd);
        }
        if !alt && !cmd && self.guide_key(name, shift)? {
            return Ok(true);
        }
        crate::shapebuilder::key(self, name)
    }

    /// Keys for a selected ruler guide: Delete removes it, arrows across it nudge it (Shift: the big
    /// nudge), Escape deselects it.
    fn guide_key(&mut self, name: &str, shift: bool) -> Result<bool> {
        let Some((owner, index)) = self.ds()?.selected_guide() else { return Ok(false) };
        // Backspace is Delete, `Left` is `ArrowLeft`, and so on.
        let name = crate::keymap::normalize(name).map_or(name, |(k, _)| k);
        let page = self.page()?;
        let at = |mut p: serde_json::Value| {
            if owner != page {
                p["frame"] = json!(owner);
            }
            p
        };
        let Some(g) = self.doc()?.node(owner).and_then(|n| n.guides().get(index).cloned()) else { return Ok(false) };
        let step = if shift { self.prefs.nudge_big } else { self.prefs.nudge_small };
        let d = match (name, g.vertical) {
            ("Delete", _) => {
                self.execute("guide.remove", &at(json!({"index": index})))?;
                return Ok(true);
            }
            ("Escape", _) => {
                self.execute("guide.select", &json!({}))?;
                return Ok(true);
            }
            ("ArrowLeft", true) | ("ArrowUp", false) => -step,
            ("ArrowRight", true) | ("ArrowDown", false) => step,
            _ => return Ok(false),
        };
        self.execute("guide.move", &at(json!({"index": index, "offset": g.offset + d})))?;
        Ok(true)
    }

    /// The layer under `p` for a click: the topmost on `page`, or in the focus view the focused
    /// frame's. Boolean groups are hit by their results ([`crate::booleans::result_outline`]); in
    /// outline mode shapes only on their outlines.
    pub fn hit(&self, doc: &Document, page: NodeId, p: Point, tol: f64) -> Option<NodeId> {
        let sel = self.selection();
        let result = crate::booleans::result_outline(doc, &sel);
        let focus = self.ds().ok().and_then(crate::DocState::focused);
        // Outline mode draws no fills: shapes are clicked on their outlines, reaching what's behind.
        if let Some(v) = self.ds().ok().map(|d| &d.view).filter(|v| v.outlines) {
            // What it draws besides the shapes can be clicked too.
            let also = figcraft_doc::geometry::OutlineHits { hidden: v.outline_hidden, bounds: v.outline_bounds };
            return figcraft_doc::geometry::hit_outlines(doc, focus.unwrap_or(page), p, tol, Some(&result), also);
        }
        match focus {
            Some(f) => figcraft_doc::geometry::hit_in(doc, f, p, tol, Some(&result)),
            None => doc.hit_test(page, p, tol, Some(&result)),
        }
    }

    /// Enter with a shape or frame tool (`tool.place`): what a click with it makes, centred on `at`
    /// (default the middle of the view) and selected.
    pub fn place(&mut self, at: Option<Point>) -> Result<()> {
        let tool = self.tool.tool;
        let refuse = |why: &str| Err(crate::EngineError::Disabled("tool.place".into(), why.into()));
        if !tool.draws_shape() {
            return refuse("choose a shape or frame tool first");
        }
        if self.tool.busy() {
            return refuse("finish the drag first");
        }
        let v = &self.ds()?.view;
        let (c, zoom) = (at.unwrap_or(Point::new(v.cx, v.cy)), v.zoom);
        let (w, h) = tool.click_size();
        let ev = |kind| PointerEvent::at(kind, c.x - w / 2.0, c.y - h / 2.0, zoom);
        let r = self.pointer(&ev(PointerKind::Down)).and_then(|()| self.pointer(&ev(PointerKind::Up)));
        if r.is_err() {
            self.tool.gesture = Gesture::None;
            let _ = self.cancel_interaction();
        }
        r
    }

    /// Keyboard Escape: cancel gestures, finish pens, leave text editing, then deselect (in the
    /// focus view, once the focused frame or nothing is selected, it ends the focus view).
    pub fn escape(&mut self) -> Result<()> {
        // Following someone in a live session: Escape stops that first.
        if self.live_unfollow() {
            return Ok(());
        }
        if self.tool.pen.is_some() {
            return self.pen_end();
        }
        if self.tool.busy() {
            self.tool.gesture = Gesture::None;
            self.tool.overlay = Overlay::default();
            return self.cancel_interaction();
        }
        if self.end_origin_edit() {
            return Ok(());
        }
        if self.ds()?.text_edit.is_some() {
            return textedit::end(self);
        }
        if self.ds()?.vector_edit.is_some() {
            return crate::vectoredit::escape(self);
        }
        if self.tool.tool != ToolId::Move {
            self.set_tool(ToolId::Move);
            return Ok(());
        }
        let sel = self.selection();
        if let Some(f) = self.ds()?.focused()
            && (sel.is_empty() || sel == [f])
        {
            self.ds_mut()?.focus = None;
            return Ok(());
        }
        // Escape selects the parent, then nothing.
        let doc = self.doc()?;
        let parents: Vec<NodeId> =
            sel.iter().filter_map(|i| doc.node(*i).and_then(|n| n.parent)).filter(|p| !doc.node(*p).is_some_and(Node::is_page)).collect();
        if !parents.is_empty() {
            let mut uniq = Vec::new();
            for p in parents {
                if !uniq.contains(&p) {
                    uniq.push(p);
                }
            }
            self.set_selection(uniq);
        } else {
            self.set_selection(vec![]);
        }
        Ok(())
    }

    /// Whether the Move tool shows arc and shape handles now (not while another gesture runs, nor
    /// in vector or text edit mode).
    fn arc_handles_shown(&self) -> bool {
        let arc_drag = matches!(self.tool.gesture, Gesture::Arc { .. } | Gesture::Shape { .. });
        self.tool.tool == ToolId::Move
            && !self.tool.dev_mode
            && (arc_drag || !self.tool.busy())
            && self.ds().is_ok_and(|ds| ds.vector_edit.is_none() && ds.text_edit.is_none())
    }

    /// The selected ellipse's arc handles as the canvas shows them at the current zoom.
    pub fn arc_handles(&self) -> Option<crate::arc_handles::ArcHandles> {
        let ds = self.ds().ok()?;
        self.arc_handles_shown().then(|| crate::arc_handles::handles(&ds.doc, &ds.selection, ds.view.zoom)).flatten()
    }

    /// The selected shape's corner radius and star ratio handles as the canvas shows them now.
    pub fn shape_handles(&self) -> Vec<(ShapeHandle, [f64; 2])> {
        let Ok(ds) = self.ds() else { return vec![] };
        if self.arc_handles_shown() { crate::shape_handles::handles(&ds.doc, &ds.selection, ds.view.zoom) } else { vec![] }
    }

    /// Snapshot of tool state for the UI / inspect.
    pub fn tool_info(&self) -> serde_json::Value {
        let shapes: Vec<serde_json::Value> = self.shape_handles().into_iter().map(|(h, at)| json!({"handle": h, "at": at})).collect();
        json!({"tool": self.tool.tool.id(), "busy": self.tool.busy(), "overlay": self.tool.overlay, "arcHandles": self.arc_handles(), "shapeHandles": shapes})
    }
}

/// The readout of a shape handle drag on `id`.
fn shape_hint(doc: &Document, id: NodeId, handle: ShapeHandle, zoom: f64) -> Option<ShapeHint> {
    Some(ShapeHint { handle, value: crate::shape_handles::value(doc, id, handle)?, at: crate::shape_handles::handle_at(doc, id, handle, zoom)? })
}

#[allow(dead_code)]
fn unused(_: BTreeMap<u8, u8>) {}

/// An end point of a selected open vector within `tol` of page point `p`: (layer, point).
fn selected_end_at(doc: &Document, sel: &[NodeId], p: Point, tol: f64) -> Option<(NodeId, usize)> {
    sel.iter().find_map(|id| {
        let net = crate::vectoredit::network(doc, *id)?;
        let w = doc.world_transform(*id);
        let near = |v: &usize| net.vertices.get(*v).is_some_and(|x| (w * x.point() - p).hypot() <= tol);
        (0..net.vertices.len()).filter(near).find(|v| net.incident(*v).len() == 1).map(|v| (*id, v))
    })
}
