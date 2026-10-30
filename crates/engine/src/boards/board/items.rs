use super::connectors::{CAPS, make};
use super::{
    AUTHOR_INK, Affine, AutoLayout, BlendMode, BoardItem, BoardShape, Color, Connector, Constraints, CrossAlign, Document, Effect, EffectKind, End,
    EngineError, HConstraint, HIGHLIGHTER, INK, LayoutMode, MARKER, MainAlign, Node, NodeId, NodeKind, Paint, Result, SHAPE, STAMP_SIZE, STICKY,
    STICKY_GAP, STICKY_PAD, Session, Shadow, Side, Sizing, StrokeAlign, StrokeCap, StrokeJoin, TextAlignH, TextAutoResize, TextData, VConstraint,
    Value, VectorNetwork, author, bad, boolean, color_param, get, id, id_of, json, line_shade, num, num_or, parent_of, req_num, require, shape_of,
    stamp_of, string, textedit,
};

/// A new layer at the top of `parent`, at `at` in it, made by `f`.
pub(super) fn add(
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
pub(crate) fn add_text(doc: &mut Document, parent: NodeId, text: &str, size: f64, color: Color, align: TextAlignH, width: f64) -> Result<NodeId> {
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

pub(super) fn sticky_cmd(s: &mut Session, p: &Value) -> Result<Value> {
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

pub(super) fn next_sticky(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.nextSticky";
    require(s, cmd)?;
    let editing = s.ds()?.text_edit.as_ref().map(|t| t.id);
    let doc = s.doc()?;
    let from = id(p, "id")
        .or_else(|| editing.and_then(|t| owner_of_text(doc, t)))
        .or_else(|| s.selection().first().copied().map(|i| owner_of_text(doc, i).unwrap_or(i)))
        .ok_or_else(|| EngineError::Disabled(cmd.into(), "select a sticky or a shape first".into()))?;
    let n = doc.get(from)?;
    if matches!(n.board, Some(BoardItem::Shape { .. })) {
        // A shape gets the next one joined to it, as the "+" beside it makes (#1597).
        return add_connected(s, p);
    }
    if !matches!(n.board, Some(BoardItem::Sticky { .. })) {
        return Err(bad(cmd, "that isn't a sticky or a shape"));
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

/// The room between a board object and the next one `board.addConnected` makes, joined by a
/// connector: enough for the arrow to read.
pub const CONNECTED_GAP: f64 = 80.0;

/// Where `board.addConnected` puts the next object on `side` of one at `at` sized `size`, `next`
/// big (its top-left), and the sides the connector joins (from, to). `None`: not a side.
pub fn connected_place(at: (f64, f64), size: (f64, f64), next: (f64, f64), side: &str) -> Option<((f64, f64), Side, Side)> {
    let ((x, y), (w, h), (nw, nh)) = (at, size, next);
    let (cx, cy) = (x + (w - nw) / 2.0, y + (h - nh) / 2.0);
    Some(match side {
        "right" => ((x + w + CONNECTED_GAP, cy), Side::Right, Side::Left),
        "left" => ((x - CONNECTED_GAP - nw, cy), Side::Left, Side::Right),
        "below" => ((cx, y + h + CONNECTED_GAP), Side::Bottom, Side::Top),
        "above" => ((cx, y - CONNECTED_GAP - nh), Side::Top, Side::Bottom),
        _ => return None,
    })
}

/// `board.addConnected`: the next sticky or shape on a side of one, of the same kind, size and
/// colour, joined to it by a connector the way the connector tool makes one; one undo step.
pub(super) fn add_connected(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.addConnected";
    require(s, cmd)?;
    let editing = s.ds()?.text_edit.as_ref().map(|t| t.id);
    let doc = s.doc()?;
    let from = id(p, "id")
        .or_else(|| editing.and_then(|t| owner_of_text(doc, t)))
        .or_else(|| s.selection().first().copied().map(|i| owner_of_text(doc, i).unwrap_or(i)))
        .ok_or_else(|| EngineError::Disabled(cmd.into(), "select a sticky or a shape first".into()))?;
    let n = doc.get(from)?;
    let shape = match &n.board {
        Some(BoardItem::Sticky { .. }) => None,
        Some(BoardItem::Shape { shape }) => Some(*shape),
        _ => return Err(bad(cmd, "that isn't a sticky or a shape")),
    };
    let side = string(p, "side").unwrap_or_else(|| "right".into());
    let parent = n.parent.ok_or_else(|| bad(cmd, "that object has no parent"))?;
    let at = n.transform.translation();
    let colour_of = |n: &Node| n.fills.iter().find_map(Paint::solid_color);
    let (next, color) = match shape {
        None => ((STICKY, STICKY), colour_of(n)),
        Some(_) => ((n.width, n.height), n.children.first().and_then(|c| doc.node(*c)).and_then(colour_of)),
    };
    let color = color.unwrap_or(s.tool.board.sticky);
    let (to_at, from_side, to_side) = connected_place((at.x, at.y), (n.width, n.height), next, &side)
        .ok_or_else(|| bad(cmd, format!("`side` is right, below, left or above, not `{side}`")))?;
    let (routing, author, page) = (s.tool.board.routing, author(s), s.page()?);
    if editing.is_some() {
        textedit::end(s)?;
    }
    let doc = s.doc_mut()?;
    let (id, text) = match shape {
        None => make_sticky(doc, parent, to_at, "", color, &author)?,
        Some(shape) => make_shape(doc, parent, to_at, next, shape, "", color)?,
    };
    let connector = Connector { from: End::Node { node: from, side: from_side }, to: End::Node { node: id, side: to_side }, routing, path: None };
    let line = make(doc, page, connector, CAPS)?;
    s.set_selection(vec![id]);
    if boolean(p, "edit") == Some(true) {
        s.request_ui("board.editText", json!({"id": id}));
    }
    Ok(json!({"id": id, "text": text, "connector": line}))
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

pub(super) fn shape_padding(shape: BoardShape, w: f64, h: f64) -> [f64; 4] {
    let [t, r, b, l] = shape.text_insets();
    [t * h, r * w, b * h, l * w]
}

pub(super) fn shape_cmd(s: &mut Session, p: &Value) -> Result<Value> {
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

pub(super) fn stamp_cmd(s: &mut Session, p: &Value) -> Result<Value> {
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
    // It sticks to the object it lands on (it moves with it).
    let on = super::derived::stamp_target(doc, page, figcraft_geom::Point::new(x, y), None);
    let id = add(doc, page, NodeKind::Frame, stamp.label(), (x - half, y - half), (STAMP_SIZE, STAMP_SIZE), |n| {
        n.fills = vec![Paint::solid(stamp.color())];
        n.corners.radii = [half; 4];
        n.effects = vec![soft_shadow()];
        n.clip = false;
        n.board = Some(BoardItem::Stamp { stamp, author, on });
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

pub(super) fn ink_cmd(s: &mut Session, p: &Value) -> Result<Value> {
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
