//! Boards: files for thinking together, with sticky notes, shapes with text, connectors, ink and
//! stamps. Board objects are ordinary layers with a [`BoardItem`] on them; [`settle_text`] and
//! [`settle_routes`] keep what follows from it (a sticky's text size, a shape's text room,
//! connectors' routes and their labels) up to date in the normalize pass, so local edits, undo and
//! live-session edits from others all end in the same geometry.

use std::collections::{BTreeMap, BTreeSet};

use figcraft_color::{BlendMode, Color, Paint};
use figcraft_doc::{
    AutoLayout, BOARD_COLORS, BoardItem, BoardShape, Connector, Constraints, CrossAlign, DocError, Document, Effect, EffectKind, End, FileKind,
    HConstraint, LayoutMode, MainAlign, Node, NodeId, NodeKind, Routing, STAMP_SIZE, Shadow, Side, Sizing, Stamp, StrokeAlign, StrokeCap, StrokeJoin,
    TextAlignH, TextAutoResize, TextData, VConstraint, VectorNetwork, board_color,
};
use figcraft_geom::kurbo::PathEl;
use figcraft_geom::{Affine, BezPath, Point, Rect, Shape};
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::params::{boolean, color_of, get, id, id_of, num, num_or, opt_id, req_num, req_string, req_targets, string};
use crate::tools::{Gesture, PointerEvent, ToolId};
use crate::{EngineError, Result, Session, bad, spec, textedit};

mod connectors;
mod derived;
mod edit;
mod items;
mod reshape;
mod tools;

pub(crate) use connectors::end_handle_at;
pub use connectors::{END_HANDLE_PX, selected_connector_ends};
use connectors::{connector_cmd, reattach};
pub use derived::route;
pub(crate) use derived::{settle_routes, settle_stamps, settle_text, stamp_target};
pub use edit::label_of;
pub(crate) use edit::{copied_items, copy_map, rejoin};
use edit::{label_cmd, set_color, set_style};
pub(crate) use items::add_text;
pub use items::{CONNECTED_GAP, connected_place};
use items::{add_connected, ink_cmd, next_sticky, shape_cmd, stamp_cmd, sticky_cmd};
pub use items::{edit_text, owner_of_text, sticky_font_size, text_of};
pub(crate) use reshape::path_handle_at;
pub use reshape::selected_connector_handles;

/// Why a board command refuses on a design file.
pub const NEEDS_BOARD: &str = "this is for boards: make one with File ▸ New board";
/// Why prototyping and Dev Mode refuse on a board.
pub const NOT_ON_BOARDS: &str = "boards have no prototypes or Dev Mode: use a design file for those";

/// A sticky note's side (it grows taller for long notes).
pub const STICKY: f64 = 200.0;
const STICKY_PAD: f64 = 16.0;
/// The room between a sticky and the next one made beside it.
const STICKY_GAP: f64 = 24.0;
/// A new shape's size.
const SHAPE: f64 = 160.0;
/// Text on board objects, and the author line under a sticky's note.
pub(crate) const INK: Color = Color::from_rgb8_const(0x1E, 0x1E, 0x1E);
const AUTHOR_INK: Color = Color::from_rgb8_const(0x5C, 0x5A, 0x55);
/// Connectors' default colour and weight.
const LINE: Color = Color::from_rgb8_const(0x3A, 0x39, 0x36);
const LINE_WEIGHT: f64 = 2.0;
/// Marker and highlighter strokes: (weight, opacity).
const MARKER: (f64, f32) = (4.0, 1.0);
const HIGHLIGHTER: (f64, f32) = (20.0, 0.45);
/// How far the eraser reaches around the pointer (screen pixels).
const ERASER_PX: f64 = 8.0;
/// The text sizes the contextual bar offers.
pub const TEXT_SIZES: [(&str, f64); 4] = [("S", 12.0), ("M", 16.0), ("L", 24.0), ("XL", 36.0)];

/// The board tools' choices: the colour new stickies get, the shape and stamp the tools place,
/// how new connectors run and the marker's and highlighter's colours.
#[derive(Clone, Debug)]
pub struct BoardTool {
    pub sticky: Color,
    pub shape: BoardShape,
    pub stamp: Stamp,
    pub routing: Routing,
    pub marker: Color,
    pub highlighter: Color,
}

impl Default for BoardTool {
    fn default() -> Self {
        let first = BOARD_COLORS.first().map_or(Color::WHITE, |c| c.1);
        Self {
            sticky: first,
            shape: BoardShape::Rectangle,
            stamp: Stamp::ThumbsUp,
            routing: Routing::Elbow,
            marker: INK,
            highlighter: Color::from_rgb8_const(0xFF, 0xD2, 0x1F),
        }
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "board.sticky",
            "Sticky note",
            "{x, y (its top-left, in the parent), text?, color? (yellow|orange|red|pink|violet|blue|green|gray or hex; default the sticky tool's), author? (a name, or false for none; default yours), parent?} → {id, text}",
            true,
            sticky_cmd
        ),
        spec!(
            "board.nextSticky",
            "Next sticky note",
            "{id? (a sticky or shape; default the selected or edited one), side?: right|below|left|above (default right), edit? (start typing in it; default false)} → {id, text, connector?}  a sticky of the same colour beside a sticky; after a shape, the next shape joined to it by a connector, as board.addConnected",
            true,
            next_sticky
        ),
        spec!(
            "board.addConnected",
            "Add connected shape",
            "{id? (a sticky or shape; default the selected or edited one), side?: right|below|left|above (default right), edit? (start typing in it; default false)} → {id, text, connector}  the next one of the same kind, size and colour on that side, joined by a connector",
            true,
            add_connected
        ),
        spec!(
            "board.shape",
            "Shape with text",
            "{shape: rectangle|roundedRectangle|ellipse|diamond|triangle|triangleDown|parallelogramRight|parallelogramLeft|trapezoid|pentagon|hexagon|octagon|star|plus|arrowLeft|arrowRight|chevron|speechBubble|database|document|predefinedProcess|manualInput|internalStorage, x, y, width?, height? (default 160), text?, color?, parent?} → {id, text}",
            true,
            shape_cmd
        ),
        spec!(
            "board.connector",
            "Connector",
            "{from, to (each a layer id or {x, y} on the page), fromSide?, toSide? (auto|top|right|bottom|left), fromAt?, toAt? ([x, y] on the layer, fractions of its box), style?: straight|elbow|curved, startCap?, endCap? (none|lineArrow|triangleArrow|circleFilled|diamondFilled…; default an arrow at the end), label?} → {id, label?}  a line that follows the layers it joins",
            true,
            connector_cmd
        ),
        spec!(
            "board.reattach",
            "Reattach connector",
            "{id? (a connector; default the selection), end: from|to, target (a layer id, or {x, y} on the page to leave the end free), side? (auto|top|right|bottom|left), at? ([x, y] on the layer, fractions of its box)} → {id}  moves one end of a connector",
            true,
            reattach
        ),
        spec!(
            "board.reshape",
            "Reshape connector",
            "{id? (a connector; default the selection), portion (0 is the first run of an elbow, or a curve's middle), x, y (where it moves to on the page; a first or last run gets an extra run to stay on its end)} → {id}",
            true,
            reshape::reshape
        ),
        spec!("board.label", "Connector label", "{id (a connector; default the selection), text (empty removes it)} → {id, label}", true, label_cmd),
        spec!(
            "board.stamp",
            "Stamp",
            "{x, y (its centre on the page), stamp: thumbsUp|heart|plusOne|star|question|check} → {id}  a badge with your name on hover",
            true,
            stamp_cmd
        ),
        spec!(
            "board.ink",
            "Marker stroke",
            "{path (SVG path data on the page) | points [[x,y],..], highlighter? (wide and see-through), color?, select? (default true)} → {id}",
            true,
            ink_cmd
        ),
        spec!(
            "board.setColor",
            "Board color",
            "{ids?, color (a board colour name or hex)}  stickies and shapes take it as their fill, connectors and ink as their line",
            true,
            set_color
        ),
        spec!(
            "board.setStyle",
            "Board style",
            "{ids?, shape? (shapes with text), fontSize? (their text), style?: straight|elbow|curved, startCap?, endCap? (connectors)}",
            true,
            set_style
        ),
        spec!("tool.sticky", "Sticky note", "{color?} (a click places a sticky and starts typing in it)", false, |s, p| {
            if let Some(c) = color_param("tool.sticky", p)? {
                s.tool.board.sticky = c;
            }
            board_tool(s, ToolId::Sticky)
        }),
        spec!("tool.shape", "Shapes", "{shape?} (a click places a shape with text)", false, |s, p| {
            if let Some(v) = string(p, "shape") {
                s.tool.board.shape = shape_of("tool.shape", &v)?;
            }
            board_tool(s, ToolId::BoardShape)
        }),
        spec!("tool.connector", "Connector", "{style?: straight|elbow|curved} (drag from one object to another)", false, |s, p| {
            if let Some(v) = string(p, "style") {
                s.tool.board.routing = routing_of("tool.connector", &v)?;
            }
            board_tool(s, ToolId::Connector)
        }),
        spec!("tool.marker", "Marker", "{color?}", false, |s, p| {
            if let Some(c) = color_param("tool.marker", p)? {
                s.tool.board.marker = c;
            }
            board_tool(s, ToolId::Marker)
        }),
        spec!("tool.highlighter", "Highlighter", "{color?}", false, |s, p| {
            if let Some(c) = color_param("tool.highlighter", p)? {
                s.tool.board.highlighter = c;
            }
            board_tool(s, ToolId::Highlighter)
        }),
        spec!("tool.eraser", "Eraser", "{} (drag over marker and highlighter strokes to remove them)", false, |s, _| board_tool(s, ToolId::Eraser)),
        spec!("tool.stamp", "Stamp", "{stamp?} (a click places the stamp)", false, |s, p| {
            if let Some(v) = string(p, "stamp") {
                s.tool.board.stamp = stamp_of("tool.stamp", &v)?;
            }
            board_tool(s, ToolId::Stamp)
        }),
    ]
}

pub fn is_board(doc: &Document) -> bool {
    doc.kind == FileKind::Board
}

/// Refuse `cmd` unless the active file is a board.
pub(crate) fn require(s: &Session, cmd: &str) -> Result<()> {
    if s.doc().is_ok_and(is_board) { Ok(()) } else { Err(EngineError::Disabled(cmd.into(), NEEDS_BOARD.into())) }
}

/// Prototyping and Dev Mode: not on boards.
fn design_only(cmd: &str) -> bool {
    cmd.starts_with("proto.") || cmd.starts_with("dev.") || matches!(cmd, "view.prototype" | "view.designPrototype" | "view.devMode")
}

/// Refuse `cmd` on a board when it is prototyping or Dev Mode.
pub(crate) fn refuse_on_board(s: &Session, cmd: &str) -> Result<()> {
    if design_only(cmd) && s.doc().is_ok_and(is_board) { Err(EngineError::Disabled(cmd.into(), NOT_ON_BOARDS.into())) } else { Ok(()) }
}

fn board_tool(s: &mut Session, t: ToolId) -> Result<Value> {
    require(s, &format!("tool.{}", t.id()))?;
    crate::cmd::view::tool(s, t)
}

fn shape_of(cmd: &str, v: &str) -> Result<BoardShape> {
    BoardShape::from_id(v).ok_or_else(|| {
        let known: Vec<&str> = BoardShape::ALL.iter().map(|b| b.id()).collect();
        bad(cmd, format!("unknown shape `{v}` ({})", known.join("|")))
    })
}

fn stamp_of(cmd: &str, v: &str) -> Result<Stamp> {
    Stamp::from_id(v).ok_or_else(|| bad(cmd, format!("unknown stamp `{v}` (thumbsUp|heart|plusOne|star|question|check)")))
}

fn routing_of(cmd: &str, v: &str) -> Result<Routing> {
    Routing::from_id(v).ok_or_else(|| bad(cmd, format!("`style` is straight, elbow or curved, not `{v}`")))
}

fn cap_of(cmd: &str, p: &Value, key: &str) -> Result<Option<StrokeCap>> {
    match get(p, key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => crate::cmd::props::enum_of::<StrokeCap>(v).map(Some).ok_or_else(|| bad(cmd, format!("`{key}`: unknown cap"))),
    }
}

/// `color`: a board colour's name, or any colour.
fn color_param(cmd: &str, p: &Value) -> Result<Option<Color>> {
    match get(p, "color") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if board_color(s).is_some() => Ok(board_color(s)),
        Some(v) => color_of(v).map(Some).ok_or_else(|| bad(cmd, "`color` is a board colour (yellow, blue…) or a colour such as #FFE78A")),
    }
}

/// The deeper shade of a board colour lines are drawn in (light tints are too faint as lines).
fn line_shade(c: Color) -> Color {
    let palette = BOARD_COLORS.iter().any(|(_, b)| *b == c);
    if palette { Color::new(c.r * 0.55, c.g * 0.55, c.b * 0.55, c.a) } else { c }
}

/// Your name on what you make: the name from Preferences, else the one a live session gave you
/// (none for the default name outside a session).
pub(crate) fn author(s: &Session) -> String {
    let name = crate::live::display_name(&s.prefs);
    if name.is_empty() { s.live().info.as_ref().map(|i| i.name.clone()).unwrap_or_default() } else { name }
}

/// The parent a command places into, and that parent's page-to-local transform.
fn parent_of(cmd: &str, s: &Session, p: &Value) -> Result<NodeId> {
    let page = s.page()?;
    let parent = opt_id(cmd, p, "parent")?.unwrap_or(page);
    let doc = s.doc()?;
    let n = doc.get(parent)?;
    let takes = n.is_page() || matches!(n.kind, NodeKind::Section { .. }) || (n.kind.is_frame_like() && !n.is_board_atom());
    if !takes {
        return Err(bad(cmd, "`parent` is the page, a section or a frame"));
    }
    Ok(parent)
}
