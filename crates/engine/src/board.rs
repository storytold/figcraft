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
use figcraft_geom::{Affine, BezPath, Point, Rect, Shape, Vec2};
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::params::{boolean, color_of, get, id, id_of, num, num_or, opt_id, req_num, req_string, req_targets, string};
use crate::tools::{Gesture, PointerEvent, ToolId};
use crate::{EngineError, Result, Session, bad, spec, textedit};

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
const INK: Color = Color::from_rgb8_const(0x1E, 0x1E, 0x1E);
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
            "{id? (a sticky; default the selected or edited one), side?: right|below|left|above (default right), edit? (start typing in it; default false)} → {id, text}  a sticky of the same colour beside it",
            true,
            next_sticky
        ),
        spec!(
            "board.shape",
            "Shape with text",
            "{shape: rectangle|roundedRectangle|ellipse|diamond|triangle|speechBubble|chevron, x, y, width?, height? (default 160), text?, color?, parent?} → {id, text}",
            true,
            shape_cmd
        ),
        spec!(
            "board.connector",
            "Connector",
            "{from, to (each a layer id or {x, y} on the page), fromSide?, toSide? (auto|top|right|bottom|left), style?: straight|elbow|curved, startCap?, endCap? (none|lineArrow|triangleArrow|circleFilled|diamondFilled…; default an arrow at the end), label?} → {id, label?}  a line that follows the layers it joins",
            true,
            connector_cmd
        ),
        spec!(
            "board.reattach",
            "Reattach connector",
            "{id? (a connector; default the selection), end: from|to, target (a layer id, or {x, y} on the page to leave the end free), side? (auto|top|right|bottom|left)} → {id}  moves one end of a connector",
            true,
            reattach
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
    BoardShape::from_id(v)
        .ok_or_else(|| bad(cmd, format!("unknown shape `{v}` (rectangle|roundedRectangle|ellipse|diamond|triangle|speechBubble|chevron)")))
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
fn author(s: &Session) -> String {
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

/// A new layer at the top of `parent`, at `at` in it, made by `f`.
fn add(
    doc: &mut Document,
    parent: NodeId,
    kind: NodeKind,
    name: &str,
    at: (f64, f64),
    size: (f64, f64),
    f: impl FnOnce(&mut Node),
) -> Result<NodeId> {
    let id = doc.alloc_id();
    let mut n = Node::new(id, kind, name, size.0, size.1);
    n.transform = Affine::translate(at);
    f(&mut n);
    doc.insert(n, parent, None)?;
    Ok(id)
}

/// A soft shadow under stickies and stamps.
fn soft_shadow() -> Effect {
    Effect {
        kind: EffectKind::DropShadow(Shadow { color: Color::new(0.0, 0.0, 0.0, 0.16), offset: (0.0, 2.0), radius: 6.0, ..Shadow::default() }),
        visible: true,
    }
}

/// A text layer filling its frame's width (the frame's auto layout places it).
fn add_text(doc: &mut Document, parent: NodeId, text: &str, size: f64, color: Color, align: TextAlignH, width: f64) -> Result<NodeId> {
    let first: String = text.lines().next().unwrap_or("").chars().take(64).collect();
    let name = if first.trim().is_empty() { "Text" } else { first.as_str() };
    let mut td = TextData {
        characters: crate::cmd::props::normalize_line_endings(text),
        auto_resize: TextAutoResize::Height,
        align_h: align,
        ..TextData::default()
    };
    td.style.font_size = size;
    let id = add(doc, parent, NodeKind::Text(td), name, (0.0, 0.0), (width.max(1.0), size * 1.4), |n| {
        n.fills = vec![Paint::solid(color)];
        n.child_layout.sizing_h = Sizing::Fill;
    })?;
    if let Some(n) = doc.node(id)
        && let Some((w, h)) = figcraft_text::auto_size(n)
        && let Some(n) = doc.node_mut(id)
    {
        (n.width, n.height) = (w, h);
    }
    Ok(id)
}

/// The text size a sticky's note gets: smaller the more it holds (by characters and lines, so
/// every copy of the file picks the same size whatever fonts it has).
pub fn sticky_font_size(text: &str) -> f64 {
    let weight = text.chars().count() + 24 * text.matches('\n').count();
    match weight {
        0..=24 => 24.0,
        25..=64 => 20.0,
        65..=140 => 16.0,
        141..=280 => 13.0,
        _ => 11.0,
    }
}

/// A sticky note's layers: the frame, the note and (when known) its author.
fn make_sticky(doc: &mut Document, parent: NodeId, at: (f64, f64), text: &str, color: Color, author: &str) -> Result<(NodeId, NodeId)> {
    let id = add(doc, parent, NodeKind::Frame, "Sticky", at, (STICKY, STICKY), |n| {
        n.fills = vec![Paint::solid(color)];
        n.corners.radii = [2.0; 4];
        n.effects = vec![soft_shadow()];
        n.clip = false;
        n.layout = Some(AutoLayout {
            mode: LayoutMode::Vertical,
            gap: 8.0,
            padding: [STICKY_PAD; 4],
            main_align: MainAlign::SpaceBetween,
            ..AutoLayout::default()
        });
        n.child_layout.sizing_v = Sizing::Hug;
        n.child_layout.min_h = Some(STICKY);
        n.board = Some(BoardItem::Sticky { author: author.to_string() });
    })?;
    let inner = STICKY - 2.0 * STICKY_PAD;
    let note = add_text(doc, id, text, sticky_font_size(text), INK, TextAlignH::Left, inner)?;
    if !author.is_empty() {
        let a = add_text(doc, id, author, 10.0, AUTHOR_INK, TextAlignH::Left, inner)?;
        if let Some(n) = doc.node_mut(a) {
            n.name = "Author".into();
        }
    }
    Ok((id, note))
}

fn sticky_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.sticky";
    require(s, cmd)?;
    let color = color_param(cmd, p)?.unwrap_or(s.tool.board.sticky);
    let author = match get(p, "author") {
        Some(Value::String(a)) => a.trim().chars().filter(|c| !c.is_control()).take(64).collect(),
        Some(Value::Bool(false)) => String::new(),
        _ => author(s),
    };
    let parent = parent_of(cmd, s, p)?;
    let text = string(p, "text").unwrap_or_default();
    let at = (num_or(p, "x", 0.0), num_or(p, "y", 0.0));
    let (id, note) = make_sticky(s.doc_mut()?, parent, at, &text, color, &author)?;
    s.set_selection(vec![id]);
    Ok(json!({"id": id, "text": note}))
}

/// The first text layer inside board object `id` (its note or label).
pub fn text_of(doc: &Document, id: NodeId) -> Option<NodeId> {
    let n = doc.node(id)?;
    n.board.as_ref().filter(|b| b.has_text())?;
    n.children.iter().copied().find(|c| doc.node(*c).is_some_and(|k| matches!(k.kind, NodeKind::Text(_))))
}

/// The board object whose text `text` is, if any.
pub fn owner_of_text(doc: &Document, text: NodeId) -> Option<NodeId> {
    let p = doc.node(text)?.parent?;
    (text_of(doc, p) == Some(text)).then_some(p)
}

/// Start typing in board object `id`'s text (all of it selected). False when it has none.
pub fn edit_text(s: &mut Session, id: NodeId) -> Result<bool> {
    let Some(t) = s.doc().ok().and_then(|d| text_of(d, id)) else { return Ok(false) };
    textedit::begin(s, t, None, false)?;
    textedit::key(s, "SelectAll", false, false, false)?;
    Ok(true)
}

fn next_sticky(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.nextSticky";
    require(s, cmd)?;
    let editing = s.ds()?.text_edit.as_ref().map(|t| t.id);
    let doc = s.doc()?;
    let from = id(p, "id")
        .or_else(|| editing.and_then(|t| owner_of_text(doc, t)))
        .or_else(|| s.selection().first().copied().map(|i| owner_of_text(doc, i).unwrap_or(i)))
        .ok_or_else(|| EngineError::Disabled(cmd.into(), "select a sticky first".into()))?;
    let n = doc.get(from)?;
    if !matches!(n.board, Some(BoardItem::Sticky { .. })) {
        return Err(bad(cmd, "that isn't a sticky"));
    }
    let color = n.fills.iter().find_map(Paint::solid_color).unwrap_or(s.tool.board.sticky);
    let parent = n.parent.ok_or_else(|| bad(cmd, "that sticky has no parent"))?;
    let at = n.transform.translation();
    let (dx, dy) = match string(p, "side").as_deref().unwrap_or("right") {
        "right" => (n.width + STICKY_GAP, 0.0),
        "left" => (-(STICKY + STICKY_GAP), 0.0),
        "below" => (0.0, n.height + STICKY_GAP),
        "above" => (0.0, -(STICKY + STICKY_GAP)),
        other => return Err(bad(cmd, format!("`side` is right, below, left or above, not `{other}`"))),
    };
    let author = author(s);
    if editing.is_some() {
        textedit::end(s)?;
    }
    let (id, note) = make_sticky(s.doc_mut()?, parent, (at.x + dx, at.y + dy), "", color, &author)?;
    s.set_selection(vec![id]);
    if boolean(p, "edit") == Some(true) {
        // Typing goes on in the new sticky once this step is recorded.
        s.request_ui("board.editText", json!({"id": id}));
    }
    Ok(json!({"id": id, "text": note}))
}

/// A shape with text's layers: the frame, its outline and its text.
fn make_shape(
    doc: &mut Document,
    parent: NodeId,
    at: (f64, f64),
    size: (f64, f64),
    shape: BoardShape,
    text: &str,
    color: Color,
) -> Result<(NodeId, NodeId)> {
    let (w, h) = size;
    let id = add(doc, parent, NodeKind::Frame, shape.label(), at, size, |n| {
        n.fills.clear();
        n.clip = false;
        n.layout = Some(AutoLayout {
            mode: LayoutMode::Vertical,
            padding: shape_padding(shape, w, h),
            main_align: MainAlign::Center,
            cross_align: CrossAlign::Center,
            ..AutoLayout::default()
        });
        n.board = Some(BoardItem::Shape { shape });
    })?;
    add(doc, id, NodeKind::Vector { network: VectorNetwork::from_path(&shape.path(w, h)) }, "Outline", (0.0, 0.0), size, |n| {
        n.fills = vec![Paint::solid(color)];
        n.strokes = vec![Paint::solid(line_shade(color))];
        n.stroke.weight = 1.5;
        n.stroke.align = StrokeAlign::Inside;
        n.child_layout.absolute = true;
        n.constraints = Constraints { horizontal: HConstraint::Scale, vertical: VConstraint::Scale };
    })?;
    let [_, r, _, l] = shape_padding(shape, w, h);
    let label = add_text(doc, id, text, 16.0, INK, TextAlignH::Center, (w - l - r).max(1.0))?;
    Ok((id, label))
}

fn shape_padding(shape: BoardShape, w: f64, h: f64) -> [f64; 4] {
    let [t, r, b, l] = shape.text_insets();
    [t * h, r * w, b * h, l * w]
}

fn shape_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.shape";
    require(s, cmd)?;
    let shape = match string(p, "shape") {
        Some(v) => shape_of(cmd, &v)?,
        None => s.tool.board.shape,
    };
    let size = |k| {
        num(p, k)
            .map_or(Ok(SHAPE), |v| if (1.0..=100_000.0).contains(&v) { Ok(v) } else { Err(bad(cmd, format!("`{k}` is a size from 1 to 100000"))) })
    };
    let (w, h) = (size("width")?, size("height")?);
    let color = color_param(cmd, p)?.unwrap_or(s.tool.board.sticky);
    let parent = parent_of(cmd, s, p)?;
    let text = string(p, "text").unwrap_or_default();
    let at = (num_or(p, "x", 0.0), num_or(p, "y", 0.0));
    let (id, label) = make_shape(s.doc_mut()?, parent, at, (w, h), shape, &text, color)?;
    s.set_selection(vec![id]);
    Ok(json!({"id": id, "text": label}))
}

fn stamp_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.stamp";
    require(s, cmd)?;
    let stamp = match string(p, "stamp") {
        Some(v) => stamp_of(cmd, &v)?,
        None => s.tool.board.stamp,
    };
    let (x, y) = (req_num(cmd, p, "x")?, req_num(cmd, p, "y")?);
    let author = author(s);
    let page = s.page()?;
    let half = STAMP_SIZE / 2.0;
    let doc = s.doc_mut()?;
    let id = add(doc, page, NodeKind::Frame, stamp.label(), (x - half, y - half), (STAMP_SIZE, STAMP_SIZE), |n| {
        n.fills = vec![Paint::solid(stamp.color())];
        n.corners.radii = [half; 4];
        n.effects = vec![soft_shadow()];
        n.clip = false;
        n.board = Some(BoardItem::Stamp { stamp, author });
    })?;
    for part in stamp.mark() {
        let path = figcraft_geom::BezPath::from_svg(part.d).map_err(|e| EngineError::Other(format!("stamp mark: {e}")))?;
        add(doc, id, NodeKind::Vector { network: VectorNetwork::from_path(&path) }, "Mark", (0.0, 0.0), (STAMP_SIZE, STAMP_SIZE), |n| {
            if part.filled {
                n.fills = vec![Paint::solid(Color::WHITE)];
            } else {
                n.fills.clear();
                n.strokes = vec![Paint::solid(Color::WHITE)];
                n.stroke.weight = 3.0;
                (n.stroke.cap_start, n.stroke.cap_end) = (StrokeCap::Round, StrokeCap::Round);
                n.stroke.join = StrokeJoin::Round;
                n.stroke.align = StrokeAlign::Center;
            }
        })?;
    }
    s.set_selection(vec![id]);
    Ok(json!({"id": id}))
}

fn ink_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.ink";
    require(s, cmd)?;
    let high = boolean(p, "highlighter").unwrap_or(false);
    let color = color_param(cmd, p)?.unwrap_or(if high { s.tool.board.highlighter } else { s.tool.board.marker });
    let (weight, opacity) = if high { HIGHLIGHTER } else { MARKER };
    if get(p, "path").is_none() && get(p, "points").is_none() {
        return Err(bad(cmd, "`path` or `points` is required"));
    }
    let page = s.page()?;
    let select = boolean(p, "select").unwrap_or(true);
    let mut q = json!({"type": "vector", "parent": page, "closed": false, "select": select, "name": if high { "Highlight" } else { "Ink" }, "props": {
        "fills": [], "strokes": [color.to_css_hex()], "strokeWeight": weight, "strokeCap": "round", "strokeJoin": "round", "strokeAlign": "center",
    }});
    for k in ["path", "points"] {
        if let (Some(v), Some(o)) = (get(p, k), q.as_object_mut()) {
            o.insert(k.into(), v.clone());
        }
    }
    let made = crate::cmd::create::create(s, &q)?;
    let id = made.get("id").and_then(id_of).ok_or_else(|| EngineError::Other("no stroke made".into()))?;
    let n = s.doc_mut()?.get_mut(id)?;
    n.opacity = opacity;
    if high {
        n.blend = BlendMode::Multiply;
    }
    n.board = Some(BoardItem::Ink);
    Ok(json!({"id": id}))
}

/// One connector end from the params: a layer on this page, or a point.
fn end_param(cmd: &str, s: &Session, p: &Value, key: &str, side_key: &str) -> Result<End> {
    let side = match string(p, side_key) {
        Some(v) => Side::from_id(&v).ok_or_else(|| bad(cmd, format!("`{side_key}` is auto, top, right, bottom or left")))?,
        None => Side::Auto,
    };
    match get(p, key) {
        Some(Value::Object(o)) if o.contains_key("x") => {
            let (x, y) = (num(&Value::Object(o.clone()), "x"), num(&Value::Object(o.clone()), "y"));
            match (x, y) {
                (Some(x), Some(y)) if x.is_finite() && y.is_finite() => Ok(End::Point { x, y }),
                _ => Err(bad(cmd, format!("`{key}` needs finite x and y"))),
            }
        }
        Some(v) => {
            let node = id_of(v).ok_or_else(|| bad(cmd, format!("`{key}` is a layer id or {{x, y}}")))?;
            let doc = s.doc()?;
            let n = doc.get(node)?;
            if n.is_page() || doc.page_of(node) != Some(s.page()?) {
                return Err(bad(cmd, format!("`{key}` must be a layer on this page")));
            }
            if matches!(n.board, Some(BoardItem::Connector(_))) {
                return Err(bad(cmd, format!("`{key}` can't be another connector")));
            }
            Ok(End::Node { node, side })
        }
        None => Err(bad(cmd, format!("`{key}` is required"))),
    }
}

fn connector_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.connector";
    require(s, cmd)?;
    let from = end_param(cmd, s, p, "from", "fromSide")?;
    let to = end_param(cmd, s, p, "to", "toSide")?;
    if from.node().is_some() && from.node() == to.node() {
        return Err(bad(cmd, "connect two different layers"));
    }
    let routing = match string(p, "style") {
        Some(v) => routing_of(cmd, &v)?,
        None => s.tool.board.routing,
    };
    let start = cap_of(cmd, p, "startCap")?.unwrap_or(StrokeCap::None);
    let end = cap_of(cmd, p, "endCap")?.unwrap_or(StrokeCap::TriangleArrow);
    let c = Connector { from, to, routing };
    let page = s.page()?;
    let doc = s.doc_mut()?;
    let pts = route(doc, &c).ok_or_else(|| bad(cmd, "the ends can't be placed"))?;
    if pts.windows(2).all(|w| w.first() == w.get(1)) {
        return Err(bad(cmd, "the ends are in the same place"));
    }
    let id = add(doc, page, NodeKind::Vector { network: VectorNetwork::default() }, "Connector", (0.0, 0.0), (0.0, 0.0), |n| {
        n.fills.clear();
        n.strokes = vec![Paint::solid(LINE)];
        n.stroke.weight = LINE_WEIGHT;
        n.stroke.join = StrokeJoin::Round;
        n.stroke.cap_start = start;
        n.stroke.cap_end = end;
        n.board = Some(BoardItem::Connector(c));
    })?;
    place_route(doc, id, &pts, routing);
    let mut out = json!({"id": id});
    if let Some(text) = string(p, "label").filter(|t| !t.trim().is_empty()) {
        out["label"] = json!(set_label(doc, id, &text)?);
    }
    s.set_selection(vec![id]);
    Ok(out)
}

/// The connector `id` names: a connector, or the label on one.
fn connector_of(cmd: &str, doc: &Document, id: NodeId) -> Result<(NodeId, Connector)> {
    match doc.get(id)?.board.clone() {
        Some(BoardItem::Connector(c)) => Ok((id, c)),
        Some(BoardItem::Label { connector }) => match doc.get(connector)?.board.clone() {
            Some(BoardItem::Connector(c)) => Ok((connector, c)),
            _ => Err(bad(cmd, "that isn't a connector")),
        },
        _ => Err(bad(cmd, "that isn't a connector")),
    }
}

fn reattach(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.reattach";
    require(s, cmd)?;
    let id = req_targets(cmd, s, p)?.first().copied().ok_or_else(|| bad(cmd, "select a connector"))?;
    let (id, mut c) = connector_of(cmd, s.doc()?, id)?;
    let from = match string(p, "end").as_deref() {
        Some("from") => true,
        Some("to") => false,
        _ => return Err(bad(cmd, "`end` is from or to")),
    };
    let end = end_param(cmd, s, p, "target", "side")?;
    let other = if from { c.to } else { c.from };
    if end.node().is_some() && end.node() == other.node() {
        return Err(bad(cmd, "connect two different layers"));
    }
    *(if from { &mut c.from } else { &mut c.to }) = end;
    let doc = s.doc_mut()?;
    let pts = route(doc, &c).ok_or_else(|| bad(cmd, "the ends can't be placed"))?;
    if pts.windows(2).all(|w| w.first() == w.get(1)) {
        return Err(bad(cmd, "the ends are in the same place"));
    }
    doc.get_mut(id)?.board = Some(BoardItem::Connector(c));
    place_route(doc, id, &pts, c.routing);
    Ok(json!({"id": id}))
}

/// A connector end as `board.connector` and `board.reattach` take it.
fn end_json(e: End) -> Value {
    match e {
        End::Node { node, .. } => json!(node),
        End::Point { x, y } => json!({"x": x, "y": y}),
    }
}

/// The end of connector `id` that stays while its other one (`from`: its start) is dragged.
fn other_end(doc: &Document, id: NodeId, from: bool) -> Option<End> {
    match &doc.node(id)?.board {
        Some(BoardItem::Connector(c)) => Some(if from { c.to } else { c.from }),
        _ => None,
    }
}

/// The selected connector (one, alone and unlocked) and its two ends on the page as drawn.
pub fn selected_connector_ends(doc: &Document, sel: &[NodeId]) -> Option<(NodeId, [Point; 2])> {
    let [id] = sel else { return None };
    if !matches!(doc.node(*id)?.board, Some(BoardItem::Connector(_))) || crate::tools::locked_in(doc, *id) {
        return None;
    }
    let (a, b) = ends_now(doc, *id)?;
    Some((*id, [a, b]))
}

/// How near (screen pixels) a press must be to a selected connector's end to drag it.
pub const END_HANDLE_PX: f64 = 8.0;

/// The selected connector's end under `p` at `zoom`, nearest first (`true`: its start).
pub(crate) fn end_handle_at(doc: &Document, sel: &[NodeId], p: Point, zoom: f64) -> Option<(NodeId, bool)> {
    let (id, [a, b]) = selected_connector_ends(doc, sel)?;
    let (da, db) = ((a - p).hypot(), (b - p).hypot());
    (da.min(db) * zoom.max(1e-6) <= END_HANDLE_PX).then_some((id, da <= db))
}

/// The label of connector `id`, if it has one.
pub fn label_of(doc: &Document, id: NodeId) -> Option<NodeId> {
    let parent = doc.node(id)?.parent?;
    doc.node(parent)?.children.iter().copied().find(|c| doc.node(*c).is_some_and(|n| n.board == Some(BoardItem::Label { connector: id })))
}

/// Give connector `id` the label `text` (empty removes it); the label's id.
fn set_label(doc: &mut Document, id: NodeId, text: &str) -> Result<Option<NodeId>> {
    let existing = label_of(doc, id);
    if text.trim().is_empty() {
        if let Some(l) = existing {
            doc.remove(l)?;
        }
        return Ok(None);
    }
    if let Some(l) = existing {
        if let Some(t) = text_of(doc, l)
            && let Some(NodeKind::Text(td)) = doc.node_mut(t).map(|n| &mut n.kind)
        {
            let len = td.char_count();
            td.replace_range(0, len, text);
        }
        return Ok(Some(l));
    }
    let parent = doc.node(id).and_then(|n| n.parent).ok_or(DocError::NoNode(id))?;
    let l = add(doc, parent, NodeKind::Frame, "Label", (0.0, 0.0), (40.0, 24.0), |n| {
        n.fills = vec![Paint::solid(Color::WHITE)];
        n.corners.radii = [4.0; 4];
        n.clip = false;
        n.layout = Some(AutoLayout { padding: [3.0, 6.0, 3.0, 6.0], ..AutoLayout::default() });
        n.child_layout.sizing_h = Sizing::Hug;
        n.child_layout.sizing_v = Sizing::Hug;
        n.board = Some(BoardItem::Label { connector: id });
    })?;
    let t = add_text(doc, l, text, 12.0, INK, TextAlignH::Center, 40.0)?;
    if let Some(n) = doc.node_mut(t)
        && let NodeKind::Text(td) = &mut n.kind
    {
        // The label hugs its text: one line per paragraph.
        td.auto_resize = TextAutoResize::WidthAndHeight;
        n.child_layout.sizing_h = Sizing::Hug;
    }
    Ok(Some(l))
}

fn label_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.label";
    require(s, cmd)?;
    let id = req_targets(cmd, s, p)?.first().copied().ok_or_else(|| bad(cmd, "select a connector"))?;
    let (id, _) = connector_of(cmd, s.doc()?, id)?;
    let text = req_string(cmd, p, "text")?;
    let label = set_label(s.doc_mut()?, id, &text)?;
    Ok(json!({"id": id, "label": label}))
}

/// The board objects among `ids`, each with its item (a label counts as its connector's text).
fn items(doc: &Document, ids: &[NodeId]) -> Vec<(NodeId, BoardItem)> {
    ids.iter().filter_map(|i| doc.node(*i).and_then(|n| n.board.clone()).map(|b| (*i, b))).collect()
}

fn set_color(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.setColor";
    require(s, cmd)?;
    let ids = req_targets(cmd, s, p)?;
    let color = color_param(cmd, p)?.ok_or_else(|| bad(cmd, "`color` is required"))?;
    let doc = s.doc_mut()?;
    let mut changed = Vec::new();
    for (id, item) in items(doc, &ids) {
        match item {
            BoardItem::Sticky { .. } => doc.get_mut(id)?.fills = vec![Paint::solid(color)],
            BoardItem::Shape { .. } => {
                let Some(outline) = doc.get(id)?.children.first().copied() else { continue };
                let n = doc.get_mut(outline)?;
                n.fills = vec![Paint::solid(color)];
                n.strokes = vec![Paint::solid(line_shade(color))];
            }
            BoardItem::Connector(_) | BoardItem::Ink => doc.get_mut(id)?.strokes = vec![Paint::solid(line_shade(color))],
            BoardItem::Label { .. } | BoardItem::Stamp { .. } => continue,
        }
        changed.push(id);
    }
    if changed.is_empty() {
        return Err(bad(cmd, "select stickies, shapes, connectors or ink"));
    }
    Ok(json!({"ids": changed}))
}

/// What `board.setStyle` changes (each `None` leaves it).
struct Restyle {
    shape: Option<BoardShape>,
    routing: Option<Routing>,
    size: Option<f64>,
    start: Option<StrokeCap>,
    end: Option<StrokeCap>,
}

impl Restyle {
    fn of(cmd: &str, p: &Value) -> Result<Restyle> {
        let size = num(p, "fontSize")
            .map(|v| if v.is_finite() && (1.0..=1000.0).contains(&v) { Ok(v) } else { Err(bad(cmd, "`fontSize` is from 1 to 1000")) })
            .transpose()?;
        Ok(Restyle {
            shape: string(p, "shape").map(|v| shape_of(cmd, &v)).transpose()?,
            routing: string(p, "style").map(|v| routing_of(cmd, &v)).transpose()?,
            size,
            start: cap_of(cmd, p, "startCap")?,
            end: cap_of(cmd, p, "endCap")?,
        })
    }

    /// Restyle board object `id` (`item`); whether anything about it changed.
    fn apply(&self, doc: &mut Document, id: NodeId, item: BoardItem) -> Result<bool> {
        let changed = match item {
            BoardItem::Shape { shape } => match self.shape.filter(|n| *n != shape) {
                Some(new) => {
                    reshape(doc, id, new)?;
                    true
                }
                None => false,
            },
            BoardItem::Connector(c) => self.restyle_connector(doc, id, c)?,
            _ => false,
        };
        // Text sizes: shapes' and labels' text (stickies size theirs to fit).
        let sized = match (self.size, &item) {
            (Some(size), BoardItem::Shape { .. } | BoardItem::Label { .. }) => resize_text(doc, id, size),
            _ => false,
        };
        Ok(changed || sized)
    }

    fn restyle_connector(&self, doc: &mut Document, id: NodeId, mut c: Connector) -> Result<bool> {
        let n = doc.get_mut(id)?;
        if let Some(r) = self.routing {
            c.routing = r;
            n.board = Some(BoardItem::Connector(c));
        }
        if let Some(cap) = self.start {
            n.stroke.cap_start = cap;
        }
        if let Some(cap) = self.end {
            n.stroke.cap_end = cap;
        }
        Ok(self.routing.is_some() || self.start.is_some() || self.end.is_some())
    }
}

/// Shape with text `id` takes outline `new` (its text room too).
fn reshape(doc: &mut Document, id: NodeId, new: BoardShape) -> Result<()> {
    let n = doc.get_mut(id)?;
    n.board = Some(BoardItem::Shape { shape: new });
    n.name = new.label().into();
    let (w, h) = (n.width, n.height);
    if let Some(l) = n.layout.as_mut() {
        l.padding = shape_padding(new, w, h);
    }
    if let Some(o) = doc.get(id)?.children.first().copied()
        && let Some(on) = doc.node_mut(o)
    {
        on.kind = NodeKind::Vector { network: VectorNetwork::from_path(&new.path(w, h)) };
        (on.width, on.height, on.transform) = (w, h, Affine::IDENTITY);
    }
    Ok(())
}

/// Board object `id`'s text at `size`; whether it has text.
fn resize_text(doc: &mut Document, id: NodeId, size: f64) -> bool {
    let Some(t) = text_of(doc, id) else { return false };
    if let Some(NodeKind::Text(td)) = doc.node_mut(t).map(|n| &mut n.kind) {
        set_font_size(td, size);
        return true;
    }
    false
}

fn set_style(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.setStyle";
    require(s, cmd)?;
    let ids = req_targets(cmd, s, p)?;
    let restyle = Restyle::of(cmd, p)?;
    let doc = s.doc_mut()?;
    let mut changed = Vec::new();
    for (id, item) in items(doc, &ids) {
        if restyle.apply(doc, id, item)? {
            changed.push(id);
        }
    }
    if changed.is_empty() {
        return Err(bad(cmd, "nothing selected takes these (shapes with text, connectors, labels)"));
    }
    Ok(json!({"ids": changed}))
}

fn set_font_size(td: &mut TextData, size: f64) {
    td.style.font_size = size;
    for r in &mut td.runs {
        r.style.font_size = size;
    }
}

/// Each layer of `sources` (copied one by one, in order, as `copies`) and its copy: the whole
/// subtrees pair up when they have the same shape (an instance made of a main component may not;
/// then only the roots do).
pub(crate) fn copy_map(doc: &Document, sources: &[NodeId], copies: &[NodeId]) -> BTreeMap<NodeId, NodeId> {
    let mut map = BTreeMap::new();
    for (s, c) in sources.iter().zip(copies) {
        let (a, b) = (doc.subtree(*s), doc.subtree(*c));
        if a.len() == b.len() {
            map.extend(a.into_iter().zip(b));
        } else {
            map.insert(*s, *c);
        }
    }
    map
}

/// The connectors and labels among copied layers, as `(copy, the original's board item)`, from the
/// originals in `doc` (`map`: original → copy).
pub(crate) fn copied_items(doc: &Document, map: &BTreeMap<NodeId, NodeId>) -> Vec<(NodeId, BoardItem)> {
    map.iter().filter_map(|(o, c)| doc.node(*o).and_then(|n| n.board.clone()).map(|b| (*c, b))).collect()
}

/// After a copy (`map`: each copied original → its copy; `originals`: the copies' items as the
/// originals had them), copied connectors join the copies of what they joined and labels follow
/// the copies of their connectors. From another file (`foreign`), what wasn't copied isn't here: a
/// connector's end there stays where it is drawn, as a point, and a label without its connector
/// goes.
pub(crate) fn rejoin(doc: &mut Document, originals: &[(NodeId, BoardItem)], map: &BTreeMap<NodeId, NodeId>, foreign: bool) {
    for (copy, item) in originals {
        let mut item = item.clone();
        let copied = |n: NodeId| map.contains_key(&n);
        match &mut item {
            BoardItem::Connector(c) if foreign => {
                let ends = ends_now(doc, *copy);
                for (end, at) in [(&mut c.from, ends.map(|e| e.0)), (&mut c.to, ends.map(|e| e.1))] {
                    if let (End::Node { node, .. }, Some(p)) = (*end, at)
                        && !copied(node)
                    {
                        *end = End::Point { x: p.x, y: p.y };
                    }
                }
            }
            BoardItem::Label { connector } if foreign && !copied(*connector) => {
                let _ = doc.remove(*copy);
                continue;
            }
            BoardItem::Connector(_) | BoardItem::Label { .. } => {}
            _ => continue,
        }
        item.remap(map);
        if let Some(n) = doc.node_mut(*copy) {
            n.board = Some(item);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Derived data (the normalize pass).

/// Before text is measured: a sticky's note takes the size its length calls for, and a resized
/// shape keeps its text inside its outline.
pub(crate) fn settle_text(doc: &mut Document, before: &Document, touched: &BTreeSet<NodeId>) {
    if !is_board(doc) {
        return;
    }
    for &id in touched {
        let Some(n) = doc.node(id) else { continue };
        if let Some(owner) =
            owner_of_text(doc, id).or(Some(id)).filter(|o| doc.node(*o).is_some_and(|k| matches!(k.board, Some(BoardItem::Sticky { .. }))))
            && let Some(t) = text_of(doc, owner)
            && let Some(NodeKind::Text(td)) = doc.node(t).map(|k| &k.kind)
        {
            let size = sticky_font_size(&td.characters);
            let off = td.style.font_size != size || td.runs.iter().any(|r| r.style.font_size != size);
            if off && let Some(NodeKind::Text(td)) = doc.node_mut(t).map(|k| &mut k.kind) {
                set_font_size(td, size);
            }
            continue;
        }
        if let Some(BoardItem::Shape { shape }) = n.board
            && before.node(id).is_none_or(|b| (b.width, b.height) != (n.width, n.height))
        {
            let pad = shape_padding(shape, n.width, n.height);
            if n.layout.as_ref().is_some_and(|l| l.padding != pad)
                && let Some(l) = doc.node_mut(id).and_then(|k| k.layout.as_mut())
            {
                l.padding = pad;
            }
        }
    }
}

/// After layout: connectors run between their ends again (an end whose layer is gone stays where
/// it was), one moved on its own takes its free ends along, and labels sit at their middles. Only
/// the connectors and labels an edit can have moved are looked at: touched themselves, or with a
/// touched (or gone) end, connector or layer around them.
pub(crate) fn settle_routes(doc: &mut Document, before: &Document, touched: &BTreeSet<NodeId>) {
    if !is_board(doc) || touched.is_empty() {
        return;
    }
    let hit = |doc: &Document, id: NodeId| touched.contains(&id) || doc.ancestors(id).iter().any(|a| touched.contains(a));
    let (mut connectors, mut labels) = (Vec::new(), Vec::new());
    for n in doc.nodes() {
        match &n.board {
            Some(BoardItem::Connector(c)) => {
                let ends = [c.from.node(), c.to.node()].into_iter().flatten();
                if hit(doc, n.id) || ends.into_iter().any(|e| !doc.contains(e) || hit(doc, e)) {
                    connectors.push((n.id, *c));
                }
            }
            Some(BoardItem::Label { connector }) => labels.push((n.id, *connector)),
            _ => {}
        }
    }
    let rerouted: BTreeSet<NodeId> = connectors.iter().map(|(id, _)| *id).collect();
    for (id, mut c) in connectors {
        let changed = detach_gone_ends(doc, id, &mut c) | carry_free_ends(doc, before, touched, id, &mut c);
        if changed && let Some(n) = doc.node_mut(id) {
            n.board = Some(BoardItem::Connector(c));
        }
        if let Some(pts) = route(doc, &c) {
            place_route(doc, id, &pts, c.routing);
        }
    }
    for (label, connector) in labels {
        if rerouted.contains(&connector) || touched.contains(&connector) || hit(doc, label) {
            place_label(doc, label, connector);
        }
    }
}

/// The ends of connector `id` (as `c`) whose layer is gone or on another page stay where they are
/// drawn, as points. Whether any did.
fn detach_gone_ends(doc: &Document, id: NodeId, c: &mut Connector) -> bool {
    let (page, ends) = (doc.page_of(id), ends_now(doc, id));
    let mut detached = false;
    for (end, at) in [(&mut c.from, ends.map(|e| e.0)), (&mut c.to, ends.map(|e| e.1))] {
        if let (End::Node { node, .. }, Some(p)) = (*end, at)
            && (!doc.contains(node) || doc.page_of(node) != page)
        {
            *end = End::Point { x: p.x, y: p.y };
            detached = true;
        }
    }
    detached
}

/// Connector `id` (as `c`) moved by itself (a drag, a nudge: its place changed but not what it
/// joins): its free ends move with it. Whether they did.
fn carry_free_ends(doc: &Document, before: &Document, touched: &BTreeSet<NodeId>, id: NodeId, c: &mut Connector) -> bool {
    let (Some(b), Some(n)) = (before.node(id), doc.node(id)) else { return false };
    if !touched.contains(&id) || b.transform == n.transform || b.board != n.board {
        return false;
    }
    let Some(expected) = route(doc, c).and_then(|pts| finite_box(&route_path(&pts, c.routing))).map(|b| b.origin()) else { return false };
    let d = (doc.world_transform(id) * Point::ZERO) - expected;
    if d.hypot() <= 1e-9 {
        return false;
    }
    let mut moved = false;
    for end in [&mut c.from, &mut c.to] {
        if let End::Point { x, y } = end {
            (*x, *y) = (*x + d.x, *y + d.y);
            moved = true;
        }
    }
    moved
}

/// Label `label` at the middle of connector `connector`; gone with it when it is gone.
fn place_label(doc: &mut Document, label: NodeId, connector: NodeId) {
    let Some(mid) = doc.node(connector).and_then(|_| route_mid(doc, connector)) else {
        let _ = doc.remove(label);
        return;
    };
    let Some(n) = doc.node(label) else { return };
    let pw = n.parent.map_or(Affine::IDENTITY, |p| doc.world_transform(p));
    let local = pw.inverse() * mid;
    let t = Affine::translate((local.x - n.width / 2.0, local.y - n.height / 2.0));
    if n.transform != t
        && let Some(n) = doc.node_mut(label)
    {
        n.transform = t;
    }
}

/// Connector `id`'s first and last points on the page as it is drawn now.
fn ends_now(doc: &Document, id: NodeId) -> Option<(Point, Point)> {
    let NodeKind::Vector { network } = &doc.node(id)?.kind else { return None };
    let w = doc.world_transform(id);
    Some((w * network.vertices.first()?.point(), w * network.vertices.last()?.point()))
}

/// The middle of connector `id`'s route by length, on the page.
fn route_mid(doc: &Document, id: NodeId) -> Option<Point> {
    let NodeKind::Vector { network } = &doc.node(id)?.kind else { return None };
    let w = doc.world_transform(id);
    let mut pts: Vec<Point> = Vec::new();
    figcraft_geom::kurbo::flatten(network.stroke_path().iter(), 0.25, |el| {
        if let PathEl::MoveTo(p) | PathEl::LineTo(p) = el {
            pts.push(w * p);
        }
    });
    let total: f64 = pts.windows(2).map(|s| (s[1] - s[0]).hypot()).sum();
    let mut left = total / 2.0;
    for s in pts.windows(2) {
        let (a, b) = (s[0], s[1]);
        let l = (b - a).hypot();
        if l >= left && l > 0.0 {
            return Some(a.lerp(b, left / l));
        }
        left -= l;
    }
    pts.first().copied()
}

/// The path through a route's page points `pts` (see [`route`]): one curve for a curved
/// connector, straight runs otherwise.
fn route_path(pts: &[Point], routing: Routing) -> BezPath {
    let mut path = BezPath::new();
    match (routing, pts) {
        (Routing::Curved, [p0, c0, c1, p1]) => {
            path.move_to(*p0);
            path.curve_to(*c0, *c1, *p1);
        }
        _ => {
            for (i, p) in pts.iter().enumerate() {
                if i == 0 { path.move_to(*p) } else { path.line_to(*p) }
            }
        }
    }
    path
}

/// The box a path is drawn in, when it is finite.
fn finite_box(path: &BezPath) -> Option<Rect> {
    let b = path.bounding_box();
    (b.x0.is_finite() && b.y0.is_finite() && b.x1.is_finite() && b.y1.is_finite()).then_some(b)
}

/// Write the route `pts` (page points, see [`route`]) into connector `id`: its box, place and
/// path. Untouched when it already runs there, so a connector that didn't move isn't an edit.
fn place_route(doc: &mut Document, id: NodeId, pts: &[Point], routing: Routing) {
    let Some(n) = doc.node(id) else { return };
    let pw = n.parent.map_or(Affine::IDENTITY, |p| doc.world_transform(p));
    let local = pw.inverse() * route_path(pts, routing);
    let Some(b) = finite_box(&local) else { return };
    let rel = Affine::translate((-b.x0, -b.y0)) * local;
    let network = match routing {
        Routing::Curved => VectorNetwork::from_path(&rel),
        _ => VectorNetwork::from_points(&rel.elements().iter().filter_map(PathEl::end_point).collect::<Vec<_>>(), false),
    };
    let t = Affine::translate((b.x0, b.y0));
    let same =
        n.transform == t && n.width == b.width() && n.height == b.height() && matches!(&n.kind, NodeKind::Vector { network: old } if *old == network);
    if same {
        return;
    }
    if let Some(n) = doc.node_mut(id) {
        n.kind = NodeKind::Vector { network };
        n.transform = t;
        (n.width, n.height) = (b.width(), b.height());
    }
}

/// How far an elbow connector going around the objects it joins keeps from them.
const ROOM: f64 = 24.0;

/// The page points connector `c` runs through: from the middle of the side of each attached layer
/// facing the other end (or the side it names), straight, in right angles, or (curved) its two
/// ends with the curve's two control points between them.
pub fn route(doc: &Document, c: &Connector) -> Option<Vec<Point>> {
    let bounds = |e: &End| e.node().and_then(|n| doc.world_bounds(n));
    let centre = |e: &End, b: Option<Rect>| match e {
        End::Point { x, y } => Some(Point::new(*x, *y)),
        End::Node { .. } => b.map(|b| b.center()),
    };
    let (fb, tb) = (bounds(&c.from), bounds(&c.to));
    let (fc, tc) = (centre(&c.from, fb)?, centre(&c.to, tb)?);
    let (p0, s0) = attach(&c.from, fb, tc)?;
    let (p1, s1) = attach(&c.to, tb, fc)?;
    let pts = match c.routing {
        Routing::Straight => tidy(vec![p0, p1]),
        Routing::Elbow => elbow(p0, s0, p1, s1, &[fb, tb].into_iter().flatten().collect::<Vec<_>>()),
        Routing::Curved => curve(p0, s0, p1, s1),
    };
    pts.iter().all(|p| p.x.is_finite() && p.y.is_finite()).then_some(pts)
}

/// `pts` without repeats or turns that don't turn (three in a line across or down).
fn tidy(pts: Vec<Point>) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(pts.len());
    for p in pts {
        if out.last() == Some(&p) {
            continue;
        }
        if let [.., a, b] = out.as_slice()
            && ((a.x == b.x && b.x == p.x) || (a.y == b.y && b.y == p.y))
        {
            out.pop();
        }
        out.push(p);
    }
    out
}

/// Where end `e` attaches (its point, or the middle of its layer's side facing `other`) and that
/// side (`None`: a free point).
fn attach(e: &End, b: Option<Rect>, other: Point) -> Option<(Point, Option<Side>)> {
    match e {
        End::Point { x, y } => Some((Point::new(*x, *y), None)),
        End::Node { side, .. } => {
            let b = b?;
            let side = match side {
                Side::Auto => {
                    let d = other - b.center();
                    // Compare against the box's proportions: a wide box takes the sides more.
                    if d.x.abs() * b.height().max(1e-9) >= d.y.abs() * b.width().max(1e-9) {
                        if d.x >= 0.0 { Side::Right } else { Side::Left }
                    } else if d.y >= 0.0 {
                        Side::Bottom
                    } else {
                        Side::Top
                    }
                }
                s => *s,
            };
            Some((side.point(b), Some(side)))
        }
    }
}

/// A curve from `p0` to `p1` leaving and arriving along the sides' normals (a free end pulls no
/// way): the ends and the two control points between them.
fn curve(p0: Point, s0: Option<Side>, p1: Point, s1: Option<Side>) -> Vec<Point> {
    let reach = (p1 - p0).hypot() / 2.0;
    let pull = |p: Point, s: Option<Side>| p + s.map_or(Vec2::ZERO, |s| s.normal() * reach);
    vec![p0, pull(p0, s0), pull(p1, s1), p1]
}

/// A right-angled route from `p0` to `p1`, leaving and arriving along the sides' normals (`s`; a
/// free end follows the other): the short way when it keeps out of the attached objects' `boxes`,
/// else the cheapest of a few ways around them (by length, and turns).
fn elbow(p0: Point, s0: Option<Side>, p1: Point, s1: Option<Side>, boxes: &[Rect]) -> Vec<Point> {
    let direct = tidy(short_elbow(p0, s0.map(Side::horizontal), p1, s1.map(Side::horizontal)));
    let Some(all) = boxes.iter().copied().reduce(|a, b| a.union(b)) else { return direct };
    if !crosses(&direct, boxes) {
        return direct;
    }
    // Out of each end by ROOM, then across on one of a few lines: halfway, or clear of the objects.
    let out = |p: Point, s: Option<Side>| p + s.map_or(Vec2::ZERO, |s| s.normal() * ROOM);
    let (a, b) = (out(p0, s0), out(p1, s1));
    let mut ways = vec![vec![p0, a, Point::new(b.x, a.y), b, p1], vec![p0, a, Point::new(a.x, b.y), b, p1]];
    for x in [(a.x + b.x) / 2.0, all.x0 - ROOM, all.x1 + ROOM] {
        ways.push(vec![p0, a, Point::new(x, a.y), Point::new(x, b.y), b, p1]);
    }
    for y in [(a.y + b.y) / 2.0, all.y0 - ROOM, all.y1 + ROOM] {
        ways.push(vec![p0, a, Point::new(a.x, y), Point::new(b.x, y), b, p1]);
    }
    let cost = |r: &Vec<Point>| r.windows(2).map(|s| (s[1] - s[0]).hypot()).sum::<f64>() + ROOM * r.len() as f64;
    ways.into_iter().map(tidy).filter(|r| !crosses(r, boxes)).min_by(|x, y| cost(x).total_cmp(&cost(y))).unwrap_or(direct)
}

/// Whether the right-angled route `pts` runs through the inside of any of `boxes` (along an edge,
/// or out of one, is outside).
fn crosses(pts: &[Point], boxes: &[Rect]) -> bool {
    const EDGE: f64 = 0.5;
    pts.windows(2).any(|s| {
        let (a, b) = (s[0], s[1]);
        boxes.iter().any(|r| a.x.min(b.x) < r.x1 - EDGE && a.x.max(b.x) > r.x0 + EDGE && a.y.min(b.y) < r.y1 - EDGE && a.y.max(b.y) > r.y0 + EDGE)
    })
}

/// The short right-angled route from `p0` to `p1` (`h`: the side faces sideways; a free end
/// follows the other): halfway across between two sides facing the same way, one turn otherwise.
fn short_elbow(p0: Point, h0: Option<bool>, p1: Point, h1: Option<bool>) -> Vec<Point> {
    let h0 = h0.or(h1).unwrap_or((p1.x - p0.x).abs() >= (p1.y - p0.y).abs());
    let h1 = h1.unwrap_or(h0);
    match (h0, h1) {
        (true, true) => {
            let mx = (p0.x + p1.x) / 2.0;
            vec![p0, Point::new(mx, p0.y), Point::new(mx, p1.y), p1]
        }
        (false, false) => {
            let my = (p0.y + p1.y) / 2.0;
            vec![p0, Point::new(p0.x, my), Point::new(p1.x, my), p1]
        }
        (true, false) => vec![p0, Point::new(p1.x, p0.y), p1],
        (false, true) => vec![p0, Point::new(p0.x, p1.y), p1],
    }
}

// ---------------------------------------------------------------------------------------------
// The board tools.

impl Session {
    /// A press with a board tool: place a sticky, shape or stamp, start a connector or a stroke,
    /// or start erasing.
    pub(crate) fn board_down(&mut self, ev: &PointerEvent) -> Result<()> {
        // A board tool kept while a design file came to the front: back to Move for this press.
        if !is_board(self.doc()?) {
            self.tool.tool = ToolId::Move;
            return self.pointer(ev);
        }
        // A sticky still being typed in ends first, so the next object is an undo step of its own.
        if self.ds()?.text_edit.is_some() {
            crate::textedit::end(self)?;
        }
        match self.tool.tool {
            ToolId::Sticky => {
                let half = STICKY / 2.0;
                let r = self.execute("board.sticky", &json!({"x": ev.x - half, "y": ev.y - half}))?;
                self.board_placed();
                if let Some(id) = r.get("id").and_then(id_of) {
                    edit_text(self, id)?;
                }
                Ok(())
            }
            ToolId::BoardShape => {
                let half = SHAPE / 2.0;
                self.execute("board.shape", &json!({"x": ev.x - half, "y": ev.y - half}))?;
                self.board_placed();
                Ok(())
            }
            ToolId::Stamp => {
                // Stamps go on one after another: the tool stays.
                self.execute("board.stamp", &json!({"x": ev.x, "y": ev.y}))?;
                Ok(())
            }
            ToolId::Marker | ToolId::Highlighter => {
                self.tool.gesture = Gesture::Pencil { points: vec![ev.p()], pressure: vec![ev.pressure] };
                Ok(())
            }
            ToolId::Connector => {
                let from = self.end_at(ev);
                self.tool.gesture = Gesture::Connect { from, start: ev.p() };
                Ok(())
            }
            ToolId::Eraser => {
                self.begin_interaction("Erase")?;
                self.tool.gesture = Gesture::Erase { gone: Vec::new() };
                self.erase_at(ev)
            }
            _ => Ok(()),
        }
    }

    /// After placing an object: back to the Move tool unless tools stay on.
    fn board_placed(&mut self) {
        if !self.prefs.keep_tool {
            self.set_tool(ToolId::Move);
        }
    }

    /// What a connector end at the pointer attaches to: the object there, or the point.
    fn end_at(&self, ev: &PointerEvent) -> End {
        let hit = (|| {
            let doc = self.doc().ok()?;
            let leaf = self.hit(doc, self.page().ok()?, ev.p(), 2.0 / ev.zoom.max(1e-6))?;
            let t = figcraft_doc::geometry::select_target(doc, leaf, &[], false);
            let n = doc.node(t)?;
            (!matches!(n.board, Some(BoardItem::Connector(_)) | Some(BoardItem::Label { .. }) | Some(BoardItem::Ink))).then_some(t)
        })();
        match hit {
            Some(node) => End::Node { node, side: Side::Auto },
            None => End::Point { x: ev.x, y: ev.y },
        }
    }

    pub(crate) fn connect_drag(&mut self, ev: &PointerEvent, from: End, start: Point) {
        let a = match from {
            End::Point { x, y } => Point::new(x, y),
            End::Node { .. } => start,
        };
        self.tool.overlay.rubber = Some([a.x, a.y, ev.x, ev.y]);
        self.tool.overlay.drop_target = from.node();
        self.tool.gesture = Gesture::Connect { from, start };
        if let Ok(ds) = self.ds_mut() {
            ds.revision += 1;
        }
    }

    pub(crate) fn connect_up(&mut self, ev: &PointerEvent, from: End, start: Point) -> Result<()> {
        self.tool.overlay.rubber = None;
        self.tool.overlay.drop_target = None;
        // A click, not a drag: nothing to join.
        if (ev.p() - start).hypot() * ev.zoom.max(1e-6) < 4.0 {
            return Ok(());
        }
        let to = self.end_at(ev);
        if to.node().is_some() && to.node() == from.node() {
            return Ok(());
        }
        let style = self.tool.board.routing.id();
        self.execute("board.connector", &json!({"from": end_json(from), "to": end_json(to), "style": style}))?;
        self.board_placed();
        Ok(())
    }

    /// Dragging the selected connector `id`'s end (`from`: its start): a line from its other end
    /// to the pointer, and the object it would attach to outlined.
    pub(crate) fn reattach_drag(&mut self, ev: &PointerEvent, id: NodeId, from: bool, start: Point) {
        self.tool.gesture = Gesture::Reattach { id, from, start };
        let Ok(doc) = self.doc() else { return };
        let fixed = ends_now(doc, id).map(|(a, b)| if from { b } else { a });
        let other = other_end(doc, id, from).and_then(|e| e.node());
        self.tool.overlay.rubber = fixed.map(|f| [f.x, f.y, ev.x, ev.y]);
        self.tool.overlay.drop_target = self.end_at(ev).node().filter(|n| Some(*n) != other);
        if let Ok(ds) = self.ds_mut() {
            ds.revision += 1;
        }
    }

    /// The end let go: attached to the object under the pointer, or free there (`board.reattach`).
    /// A click, or a drop on the object the other end is on, changes nothing.
    pub(crate) fn reattach_up(&mut self, ev: &PointerEvent, id: NodeId, from: bool, start: Point) -> Result<()> {
        self.tool.overlay.rubber = None;
        if (ev.p() - start).hypot() * ev.zoom.max(1e-6) < 4.0 {
            return Ok(());
        }
        let target = self.end_at(ev);
        let other = other_end(self.doc()?, id, from).and_then(|e| e.node());
        if target.node().is_some() && target.node() == other {
            return Ok(());
        }
        self.execute("board.reattach", &json!({"id": id, "end": if from { "from" } else { "to" }, "target": end_json(target)}))?;
        Ok(())
    }

    /// The eraser at the pointer: the marker and highlighter strokes it touches go (until the
    /// gesture ends, which is one undo step).
    pub(crate) fn erase_at(&mut self, ev: &PointerEvent) -> Result<()> {
        let Gesture::Erase { mut gone } = std::mem::take(&mut self.tool.gesture) else { return Ok(()) };
        let tol = ERASER_PX / ev.zoom.max(1e-6);
        self.tool.overlay.brush = Some([ev.x, ev.y, tol]);
        let (doc, page) = (self.doc()?.clone(), self.page()?);
        if let Some(hit) =
            doc.hit_test(page, ev.p(), tol, None).filter(|h| doc.node(*h).is_some_and(|n| n.board == Some(BoardItem::Ink)) && !gone.contains(h))
        {
            gone.push(hit);
            let ids = gone.clone();
            self.preview(|d, sel| {
                for i in &ids {
                    let _ = d.remove(*i);
                }
                sel.retain(|s| !ids.contains(s));
                Ok(())
            })?;
        }
        self.tool.gesture = Gesture::Erase { gone };
        if let Ok(ds) = self.ds_mut() {
            ds.revision += 1;
        }
        Ok(())
    }

    pub(crate) fn erase_up(&mut self) -> Result<()> {
        self.tool.overlay.brush = None;
        self.commit_interaction()
    }

    /// A finished marker or highlighter stroke (page points).
    pub(crate) fn ink_finish(&mut self, path: &str) -> Result<()> {
        let high = self.tool.tool == ToolId::Highlighter;
        self.execute("board.ink", &json!({"path": path, "highlighter": high, "select": false}))?;
        // Strokes come one after another: the selection stays as it was.
        Ok(())
    }
}

impl Session {
    /// A double-click with the Move tool on a board object: type in its text (a connector gets a
    /// label to type in). True when it was one.
    pub(crate) fn board_double_click(&mut self, doc: &Document, leaf: NodeId, sel: &[NodeId]) -> Result<bool> {
        let whole = figcraft_doc::geometry::select_target(doc, leaf, sel, false);
        let editing = self.ds()?.text_edit.as_ref().is_some_and(|te| owner_of_text(doc, te.id) == Some(whole));
        let Some(item) = doc.node(whole).and_then(|n| n.board.clone()).filter(|_| !editing) else { return Ok(false) };
        self.tool.gesture = Gesture::None;
        self.cancel_interaction()?;
        match item {
            BoardItem::Connector(_) => {
                let label = match label_of(doc, whole) {
                    Some(l) => l,
                    None => {
                        let r = self.execute("board.label", &json!({"id": whole, "text": "Label"}))?;
                        r.get("label").and_then(id_of).ok_or_else(|| EngineError::Other("no label made".into()))?
                    }
                };
                edit_text(self, label)
            }
            b if b.has_text() => edit_text(self, whole),
            _ => Ok(false),
        }
    }
}
