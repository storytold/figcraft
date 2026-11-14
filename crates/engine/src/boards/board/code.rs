//! Code blocks (#2237): a frame hugging one monospace text ([`BoardItem::CodeBlock`]), as wide as
//! it was made and as tall as its lines. The commands only make the block; [`settle_code`] keeps
//! it that way in the normalize pass (the height is the lines', the text is plain code), so every
//! entry point (a drag, `node.set`, a styled paste, a live session's edit) ends the same.

use std::collections::BTreeSet;

use figcraft_color::Paint;
use figcraft_doc::{
    AutoLayout, BoardItem, CODE_FONT, CODE_FONT_SIZE, CODE_GAP, CODE_MAX_CHARS, CODE_MAX_W, CODE_MIN_H, CODE_MIN_W, CODE_PAD, CODE_PANEL,
    CODE_RADIUS, CODE_TEXT, CODE_WIDTH, CodeLanguage, Document, LayoutMode, LineHeight, ListStyle, NodeId, NodeKind, ParaList, Sizing, TextAlignH,
    TextAutoResize, TextCase, TextData, TextDecoration,
};
use serde_json::{Value, json};

use super::items::{add, add_text, owner_of_text, soft_shadow, text_of};
use super::{Result, Session, bad, boolean, get, num, parent_of, req_string, req_targets, require, string, textedit};

/// How far from the page's middle a block may be put.
const CODE_MAX_COORD: f64 = 1.0e9;

/// Whether `text` is the text of a code block: what the text editor asks before it converts what
/// is typed (a list from `- `, and any smart punctuation or link to come).
pub fn is_code_text(doc: &Document, text: NodeId) -> bool {
    owner_of_text(doc, text).and_then(|o| doc.node(o)).is_some_and(|n| matches!(n.board, Some(BoardItem::CodeBlock { .. })))
}

/// The auto layout every code block has: its text stacked inside the padding.
fn block_layout() -> AutoLayout {
    AutoLayout { mode: LayoutMode::Vertical, gap: 0.0, padding: CODE_PAD, ..AutoLayout::default() }
}

/// A code block's frame and its text: `width` wide at `at` in `parent`, holding `text`.
fn make_code_block(doc: &mut Document, parent: NodeId, at: (f64, f64), width: f64, language: CodeLanguage, text: &str) -> Result<(NodeId, NodeId)> {
    let frame = add(doc, parent, NodeKind::Frame, "Code block", at, (width, CODE_MIN_H), |n| {
        n.fills = vec![Paint::solid(CODE_PANEL)];
        n.corners.radii = [CODE_RADIUS; 4];
        n.effects = vec![soft_shadow()];
        n.clip = false;
        n.layout = Some(block_layout());
        n.child_layout.sizing_v = Sizing::Hug;
        n.child_layout.min_h = Some(CODE_MIN_H);
        n.child_layout.min_w = Some(CODE_MIN_W);
        n.board = Some(BoardItem::CodeBlock { language });
    })?;
    let inner = width - CODE_PAD[1] - CODE_PAD[3];
    let label = add_text(doc, frame, text, CODE_FONT_SIZE, CODE_TEXT, TextAlignH::Left, inner)?;
    if let Some(NodeKind::Text(td)) = doc.node_mut(label).map(|n| &mut n.kind) {
        td.style.font_family = CODE_FONT.into();
        td.style.line_height = LineHeight::Percent(150.0);
        td.features.insert("liga".into(), false);
        td.features.insert("calt".into(), false);
    }
    Ok((frame, label))
}

/// A language from its id or name, or an error listing the ids.
fn language_of(cmd: &str, v: &str) -> Result<CodeLanguage> {
    CodeLanguage::from_id(v).ok_or_else(|| {
        let known: Vec<&str> = CodeLanguage::ALL.iter().map(|l| l.id()).collect();
        bad(cmd, format!("unknown language `{v}` ({})", known.join("|")))
    })
}

/// A coordinate from the params: given, it is a finite number within a billion of the origin.
fn coord(cmd: &str, p: &Value, key: &str, default: f64) -> Result<f64> {
    let v = if get(p, key).is_none_or(Value::is_null) { Some(default) } else { num(p, key) };
    v.filter(|v| v.is_finite() && v.abs() <= CODE_MAX_COORD).ok_or_else(|| bad(cmd, format!("`{key}` is a position within a billion of the origin")))
}

pub(super) fn code_block_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.codeBlock";
    require(s, cmd)?;
    let parent = parent_of(cmd, s, p)?;
    let width = if get(p, "width").is_none_or(Value::is_null) {
        CODE_WIDTH
    } else {
        num(p, "width")
            .filter(|w| w.is_finite() && (CODE_MIN_W..=CODE_MAX_W).contains(w))
            .ok_or_else(|| bad(cmd, format!("`width` is a size from {CODE_MIN_W} to {CODE_MAX_W}")))?
    };
    // The view's centre is on the page, and `x`/`y` are in the parent: a block in a section or
    // frame takes the centre in that parent's coordinates (a parent that can't be inverted gives a
    // non-finite centre, which `coord` refuses).
    let (vx, vy) = {
        let ds = s.ds()?;
        let c = ds.doc.world_transform(parent).inverse() * figcraft_geom::Point::new(ds.view.cx, ds.view.cy);
        (c.x, c.y)
    };
    let x = coord(cmd, p, "x", vx - width / 2.0)?;
    let y = coord(cmd, p, "y", vy - CODE_MIN_H / 2.0)?;
    let language = match string(p, "language") {
        Some(v) => language_of(cmd, &v)?,
        None if get(p, "language").is_none_or(Value::is_null) => CodeLanguage::default(),
        None => return Err(bad(cmd, "`language` is a language id")),
    };
    let text = match get(p, "text") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(t)) => t.clone(),
        Some(_) => return Err(bad(cmd, "`text` is a string")),
    };
    if text.chars().count() > CODE_MAX_CHARS {
        return Err(bad(cmd, format!("`text` is at most {CODE_MAX_CHARS} characters")));
    }
    let (frame, label) = make_code_block(s.doc_mut()?, parent, (x, y), width, language, &text)?;
    s.set_selection(vec![frame]);
    if boolean(p, "edit") == Some(true) {
        s.request_ui("board.editText", json!({"id": frame}));
    }
    Ok(json!({"id": frame, "text": label, "language": language.id()}))
}

pub(super) fn set_language(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.setLanguage";
    require(s, cmd)?;
    let language = language_of(cmd, &req_string(cmd, p, "language")?)?;
    let targets = req_targets(cmd, s, p)?;
    let doc = s.doc()?;
    let blocks: BTreeSet<NodeId> = targets
        .into_iter()
        .map(|i| owner_of_text(doc, i).unwrap_or(i))
        .filter(|i| doc.node(*i).is_some_and(|n| matches!(n.board, Some(BoardItem::CodeBlock { .. }))))
        .collect();
    if blocks.is_empty() {
        return Err(bad(cmd, "select a code block"));
    }
    let doc = s.doc_mut()?;
    for &b in &blocks {
        if doc.node(b).is_some_and(|n| n.board != Some(BoardItem::CodeBlock { language })) {
            doc.get_mut(b)?.board = Some(BoardItem::CodeBlock { language });
        }
    }
    Ok(json!({"ids": blocks.into_iter().collect::<Vec<_>>()}))
}

/// `board.nextSticky` on code block `from`: the next one beside it, as wide and in the same
/// language, with no connector.
pub(super) fn next_block(s: &mut Session, from: NodeId, language: CodeLanguage, p: &Value) -> Result<Value> {
    let cmd = "board.nextSticky";
    let editing = s.ds()?.text_edit.is_some();
    let doc = s.doc()?;
    let n = doc.get(from)?;
    let parent = n.parent.ok_or_else(|| bad(cmd, "that code block has no parent"))?;
    let at = n.transform.translation();
    let (w, h) = (n.width, n.height);
    let (x, y) = match string(p, "side").as_deref().unwrap_or("right") {
        "right" => (at.x + w + CODE_GAP, at.y),
        "left" => (at.x - CODE_GAP - w, at.y),
        "below" => (at.x, at.y + h + CODE_GAP),
        "above" => (at.x, at.y - CODE_GAP - CODE_MIN_H),
        other => return Err(bad(cmd, format!("`side` is right, below, left or above, not `{other}`"))),
    };
    if !(x.is_finite() && y.is_finite() && w.is_finite()) {
        return Err(bad(cmd, "the next block can't be placed there"));
    }
    let width = w.clamp(CODE_MIN_W, CODE_MAX_W);
    if editing {
        textedit::end(s)?;
    }
    let (frame, label) = make_code_block(s.doc_mut()?, parent, (x, y), width, language, "")?;
    s.set_selection(vec![frame]);
    if boolean(p, "edit") == Some(true) {
        s.request_ui("board.editText", json!({"id": frame}));
    }
    Ok(json!({"id": frame, "text": label, "language": language.id()}))
}

/// Whether `td` is already plain code: no list, indent, spacing, link, decoration, case change or
/// hand-made run, and a height that follows its lines.
fn is_plain(td: &TextData) -> bool {
    td.auto_resize == TextAutoResize::Height
        && td.list == ListStyle::None
        && td.paragraphs == plain_paragraphs(&td.paragraphs)
        && td.paragraph_indent == 0.0
        && td.paragraph_spacing == 0.0
        && td.list_spacing == 0.0
        && td.style.link.is_none()
        && td.style.decoration == TextDecoration::None
        && td.style.decoration_look.is_none()
        && td.style.case == TextCase::Original
        && td.runs.is_empty()
}

/// `paragraphs` with no list, level or counting offset (the direction stays); empty when that
/// leaves every paragraph at its default.
fn plain_paragraphs(paragraphs: &[ParaList]) -> Vec<ParaList> {
    let v: Vec<ParaList> = paragraphs.iter().map(|p| ParaList { direction: p.direction, ..ParaList::default() }).collect();
    if v.iter().all(|p| *p == ParaList::default()) { Vec::new() } else { v }
}

fn make_plain(td: &mut TextData) {
    td.auto_resize = TextAutoResize::Height;
    td.list = ListStyle::None;
    td.paragraphs = plain_paragraphs(&td.paragraphs);
    td.paragraph_indent = 0.0;
    td.paragraph_spacing = 0.0;
    td.list_spacing = 0.0;
    td.style.link = None;
    td.style.decoration = TextDecoration::None;
    td.style.decoration_look = None;
    td.style.case = TextCase::Original;
    td.runs.clear();
}

/// In the normalize pass, before text is measured and layout runs: every code block an edit
/// touched (or whose text it touched) is as tall as its lines again and holds plain code. Nothing
/// is written for a block already in that form, so a second pass changes nothing.
pub(crate) fn settle_code(doc: &mut Document, touched: &BTreeSet<NodeId>) {
    let is_block = |doc: &Document, id: NodeId| doc.node(id).is_some_and(|n| matches!(n.board, Some(BoardItem::CodeBlock { .. })));
    let mut blocks = BTreeSet::new();
    for &id in touched {
        if is_block(doc, id) {
            blocks.insert(id);
        } else if let Some(p) = doc.node(id).and_then(|n| n.parent).filter(|p| is_block(doc, *p)) {
            blocks.insert(p);
        }
    }
    for b in blocks {
        // The frame's form is complete first: its layout and minimums (or it cannot hug) and a width that
        // is not Hug (or its text cannot fill), else normalize puts them back every pass.
        if let Some(n) = doc.node_mut(b) {
            if n.layout.as_ref() != Some(&block_layout()) {
                n.layout = Some(block_layout());
            }
            if n.child_layout.min_h != Some(CODE_MIN_H) {
                n.child_layout.min_h = Some(CODE_MIN_H);
            }
            if n.child_layout.min_w != Some(CODE_MIN_W) {
                n.child_layout.min_w = Some(CODE_MIN_W);
            }
            if n.child_layout.sizing_h == Sizing::Hug {
                n.child_layout.sizing_h = Sizing::Fixed;
            }
        }
        // A fixed height is not kept: the layout pass sizes the block to its lines again.
        if doc.node(b).is_some_and(|n| n.child_layout.sizing_v != Sizing::Hug)
            && let Some(n) = doc.node_mut(b)
        {
            n.child_layout.sizing_v = Sizing::Hug;
        }
        let Some(t) = text_of(doc, b) else { continue };
        let Some(node) = doc.node(t) else { continue };
        let off = match &node.kind {
            NodeKind::Text(td) => !is_plain(td),
            _ => continue,
        };
        let fill_off = node.child_layout.sizing_h != Sizing::Fill;
        if !(off || fill_off) {
            continue;
        }
        if let Some(n) = doc.node_mut(t) {
            if fill_off {
                n.child_layout.sizing_h = Sizing::Fill;
            }
            if off && let NodeKind::Text(td) = &mut n.kind {
                make_plain(td);
            }
        }
    }
}
