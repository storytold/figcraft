//! Code blocks (#2237): `board.codeBlock` and `board.setLanguage`, the pass that keeps a block as
//! tall as its lines and its text plain code, and the text editor's three literal-typing lines.

use figcraft_color::{Color, Paint};
use figcraft_doc::{
    BoardItem, CODE_FONT, CODE_MAX_CHARS, CodeLanguage, EffectKind, FileKind, LayoutMode, LineHeight, ListStyle, NodeId, NodeKind, Sizing,
    TextAlignH, TextAutoResize, TextCase, TextData, TextDecoration,
};
use figcraft_geom::Rect;
use serde_json::{Value, json};

use crate::board::{is_code_text, text_of};
use crate::board_tests::{board, id, node};
use crate::collab_live_tests::Room;
use crate::tools::{PointerEvent, PointerKind};
use crate::{EngineError, Session};

fn bx(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().world_bounds(id).unwrap()
}

fn td(s: &Session, block: NodeId) -> TextData {
    let t = text_of(s.doc().unwrap(), block).unwrap();
    match node(s, t).kind {
        NodeKind::Text(td) => td,
        other => panic!("not text: {other:?}"),
    }
}

/// `board.codeBlock` with `p`: the frame and its text.
fn block(s: &mut Session, p: Value) -> (NodeId, NodeId) {
    let out = s.execute("board.codeBlock", &p).unwrap();
    (id(&out, "id"), id(&out, "text"))
}

/// How many visual lines the text lays out in.
fn lines(s: &Session, text: NodeId) -> usize {
    let n = node(s, text);
    figcraft_text::layout_node(&n).map_or(0, |(l, _)| l.lines.len())
}

fn solid(hex: (u8, u8, u8)) -> Vec<Paint> {
    vec![Paint::solid(Color::from_rgb8_const(hex.0, hex.1, hex.2))]
}

fn ten(n: usize) -> String {
    vec!["x"; n].join("\n")
}

fn drag(s: &mut Session, from: (f64, f64), to: (f64, f64)) {
    s.pointer(&PointerEvent::at(PointerKind::Down, from.0, from.1, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, (from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, to.0, to.1, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, to.0, to.1, 1.0)).unwrap();
}

fn undo(s: &mut Session) {
    s.execute("edit.undo", &json!({})).unwrap();
}

#[test]
fn a_code_block_is_a_dark_panel_576_by_128_holding_one_monospace_text() {
    let mut s = board();
    let before = s.doc().unwrap().ids().count();
    let depth = s.undo_depth().0;
    let out = s.execute("board.codeBlock", &json!({"x": 100, "y": 200})).unwrap();
    assert_eq!(out["language"], json!("typescript"));
    let (f, t) = (id(&out, "id"), id(&out, "text"));
    let n = node(&s, f);
    assert_eq!(n.board, Some(BoardItem::CodeBlock { language: CodeLanguage::Typescript }));
    assert!(n.is_board_atom());
    let r = bx(&s, f);
    assert_eq!((r.x0, r.y0, r.x1, r.y1), (100.0, 200.0, 676.0, 328.0));
    assert_eq!(n.fills, solid((0x1E, 0x1E, 0x1E)));
    assert_eq!(n.corners.radii, [10.0; 4]);
    assert_eq!(n.effects.iter().filter(|e| matches!(e.kind, EffectKind::DropShadow(_))).count(), 1);
    let l = n.layout.clone().unwrap();
    assert_eq!((l.mode, l.padding), (LayoutMode::Vertical, [16.0, 16.0, 16.0, 64.0]));
    assert_eq!(n.child_layout.sizing_v, Sizing::Hug);
    assert_eq!((n.child_layout.min_h, n.child_layout.min_w), (Some(128.0), Some(210.0)));
    assert_eq!(n.children, vec![t]);
    let tr = bx(&s, t);
    assert_eq!((tr.x0, tr.y0, tr.width()), (164.0, 216.0, 496.0));
    let d = td(&s, f);
    assert_eq!(d.characters, "");
    assert_eq!((d.style.font_family.as_str(), d.style.font_style.as_str(), d.style.font_size), (CODE_FONT, "Regular", 16.0));
    assert_eq!(d.style.line_height, LineHeight::Percent(150.0));
    assert_eq!((d.auto_resize, d.align_h), (TextAutoResize::Height, TextAlignH::Left));
    assert_eq!(node(&s, t).fills, solid((0xE3, 0xE1, 0xDD)));
    assert_eq!((d.features.get("liga"), d.features.get("calt")), (Some(&false), Some(&false)));
    assert!(d.runs.is_empty());
    assert_eq!(d.list, ListStyle::None);
    assert_eq!(s.selection(), vec![f]);
    assert_eq!(s.undo_depth().0, depth + 1);
    undo(&mut s);
    assert_eq!(s.doc().unwrap().ids().count(), before);
}

#[test]
fn a_block_without_a_place_is_centred_and_edit_asks_for_typing() {
    let mut s = board();
    // Centred on the view without a place.
    let (vx, vy) = {
        let v = &s.ds().unwrap().view;
        (v.cx, v.cy)
    };
    let (f, _) = block(&mut s, json!({}));
    let r = bx(&s, f);
    assert_eq!((r.x0, r.y0), (vx - 288.0, vy - 64.0));
    // Typing starts in it on request.
    let (f, _) = block(&mut s, json!({"x": 0, "y": 500, "edit": true}));
    assert!(s.ui_requests.iter().any(|q| q.kind == "board.editText" && q.data["id"] == json!(f)));
}

#[test]
fn the_height_is_its_lines_24_each_and_never_under_four() {
    let mut s = board();
    let heights = [("a", 128.0), ("a\nb\nc\nd", 128.0), ("a\nb\nc\nd\ne", 152.0)];
    for (text, h) in heights {
        let (f, _) = block(&mut s, json!({"x": 100, "y": 200, "text": text}));
        let r = bx(&s, f);
        assert_eq!((r.x0, r.y0, r.height()), (100.0, 200.0, h), "{text:?}");
    }
    let (f, t) = block(&mut s, json!({"x": 100, "y": 200, "text": ten(9)}));
    assert_eq!(bx(&s, f).height(), 248.0);
    assert_eq!(bx(&s, t).height(), 216.0);
    s.execute("text.set", &json!({"id": t, "characters": "a\nb"})).unwrap();
    let r = bx(&s, f);
    assert_eq!((r.x0, r.y0, r.height()), (100.0, 200.0, 128.0));
}

#[test]
fn long_lines_wrap_and_each_visual_line_adds_24() {
    let mut s = board();
    let (f, t) = block(&mut s, json!({"x": 0, "y": 0, "text": "ab ".repeat(400)}));
    let n = lines(&s, t);
    assert!(n >= 3, "{n}");
    assert_eq!(bx(&s, f).height(), 32.0 + 24.0 * n as f64);
    assert_eq!(bx(&s, t).width(), 496.0);
    if figcraft_text::has_family(CODE_FONT) {
        let (f, _) = block(&mut s, json!({"x": 0, "y": 0, "width": 976, "text": "x".repeat(93)}));
        assert_eq!(bx(&s, f).height(), 128.0);
        let (f, t) = block(&mut s, json!({"x": 0, "y": 0, "width": 976, "text": "x".repeat(94)}));
        assert_eq!(lines(&s, t), 2);
        assert_eq!(bx(&s, f).height(), 128.0);
        let (f, t) = block(&mut s, json!({"x": 0, "y": 0, "width": 976, "text": format!("{}\na\nb\nc", "x".repeat(94))}));
        assert_eq!(lines(&s, t), 5);
        assert_eq!(bx(&s, f).height(), 152.0);
    }
}

#[test]
fn only_the_width_is_the_users() {
    let mut s = board();
    let (f, t) = block(&mut s, json!({"x": 100, "y": 200, "text": ten(9)}));
    assert_eq!(bx(&s, f).height(), 248.0);
    s.execute("node.set", &json!({"id": f, "props": {"height": 400}})).unwrap();
    assert_eq!(bx(&s, f).height(), 248.0);
    assert_eq!(node(&s, f).child_layout.sizing_v, Sizing::Hug);
    s.execute("transform.resize", &json!({"ids": [f], "width": 300, "height": 500})).unwrap();
    let r = bx(&s, f);
    assert_eq!((r.width(), r.height(), bx(&s, t).width()), (300.0, 248.0, 220.0));
    s.execute("node.set", &json!({"id": f, "props": {"width": 100}})).unwrap();
    assert_eq!((bx(&s, f).width(), bx(&s, t).width()), (210.0, 130.0));

    // A pointer drag of the bottom-right handle.
    let mut s = board();
    let (f, _) = block(&mut s, json!({"x": 100, "y": 200}));
    let depth = s.undo_depth().0;
    drag(&mut s, (676.0, 328.0), (876.0, 628.0));
    let r = bx(&s, f);
    assert_eq!((r.x0, r.y0, r.width(), r.height()), (100.0, 200.0, 776.0, 128.0));
    assert_eq!(s.undo_depth().0, depth + 1);
    undo(&mut s);
    let r = bx(&s, f);
    assert_eq!((r.width(), r.height()), (576.0, 128.0));

    // A narrower block wraps into more lines and grows.
    let mut s = board();
    let (f, t) = block(&mut s, json!({"x": 0, "y": 0, "text": "ab ".repeat(400)}));
    let was = lines(&s, t);
    s.execute("node.set", &json!({"id": f, "props": {"width": 300}})).unwrap();
    let now = lines(&s, t);
    assert!(now > was, "{now} > {was}");
    assert_eq!(bx(&s, f).height(), 32.0 + 24.0 * now as f64);
}

/// A block settled after `how` made it incoherent: still 5 lines tall, 576 wide, hugging, with
/// its layout, and a second pass changes nothing.
fn settled_after(how: impl Fn(&mut Session, NodeId)) {
    let mut s = board();
    let (f, t) = block(&mut s, json!({"x": 0, "y": 0, "text": ten(4)}));
    how(&mut s, f);
    s.execute("text.set", &json!({"id": t, "characters": ten(5)})).unwrap();
    let n = node(&s, f);
    assert!(n.layout.is_some(), "the layout is restored");
    assert_eq!((n.child_layout.sizing_h, n.child_layout.sizing_v), (Sizing::Fixed, Sizing::Hug));
    assert_eq!((bx(&s, f).width(), bx(&s, f).height()), (576.0, 152.0));
    assert_eq!(node(&s, t).child_layout.sizing_h, Sizing::Fill);
    let doc = s.doc().unwrap().clone();
    let mut again = doc.clone();
    again.mark_dirty(doc.ids().collect::<Vec<_>>());
    crate::normalize(&mut again, &doc);
    assert_eq!(again, doc, "a second pass changes nothing");
}

#[test]
fn the_pass_keeps_a_blocks_layout_and_width_sizing() {
    settled_after(|s, f| {
        s.execute("layout.remove", &json!({"ids": [f]})).unwrap();
    });
    settled_after(|s, f| {
        s.execute("node.set", &json!({"id": f, "props": {"layout": null}})).unwrap();
    });
    settled_after(|s, f| {
        s.execute("node.set", &json!({"id": f, "props": {"sizingH": "hug"}})).unwrap();
    });
}

#[test]
fn a_default_position_is_within_a_billion_too() {
    let mut s = board();
    s.execute("view.set", &json!({"cx": 1e300})).unwrap();
    match s.execute("board.codeBlock", &json!({})) {
        Err(EngineError::BadParams { .. }) => {}
        Err(other) => panic!("not a named refusal: {other:?}"),
        Ok(out) => panic!("placed at {out}"),
    }
}

#[test]
fn the_next_block_is_128_to_the_right_as_wide_and_in_the_same_language() {
    let mut s = board();
    let (a, _) = block(&mut s, json!({"x": 0, "y": 0, "width": 976, "language": "rust", "text": "fn main() {}"}));
    let connectors = |s: &Session| s.doc().unwrap().nodes().filter(|n| matches!(n.board, Some(BoardItem::Connector(_)))).count();
    let depth = s.undo_depth().0;
    let out = s.execute("board.nextSticky", &json!({"id": a})).unwrap();
    let b = id(&out, "id");
    let r = bx(&s, b);
    assert_eq!((r.x0, r.y0, r.width(), r.height()), (1104.0, 0.0, 976.0, 128.0));
    assert_eq!(node(&s, b).board, Some(BoardItem::CodeBlock { language: CodeLanguage::Rust }));
    assert_eq!(td(&s, b).characters, "");
    assert_eq!(connectors(&s), 0);
    assert_eq!(s.selection(), vec![b]);
    assert_eq!(s.undo_depth().0, depth + 1);
    undo(&mut s);
    assert!(s.doc().unwrap().node(b).is_none());
    // While typing in the source, with no id, and asking for the next one to be typed in.
    let t = text_of(s.doc().unwrap(), a).unwrap();
    s.execute("text.edit.begin", &json!({"id": t})).unwrap();
    let out = s.execute("board.nextSticky", &json!({"edit": true})).unwrap();
    let c = id(&out, "id");
    assert_eq!(bx(&s, c).x0, 1104.0);
    assert!(s.ds().unwrap().text_edit.is_none(), "the edit has ended");
    assert!(s.ui_requests.iter().any(|q| q.kind == "board.editText" && q.data["id"] == json!(c)));
    let out = s.execute("board.nextSticky", &json!({"id": a, "side": "below"})).unwrap();
    let r = bx(&s, id(&out, "id"));
    assert_eq!((r.x0, r.y0), (0.0, 256.0));
    assert!(s.execute("board.nextSticky", &json!({"id": a, "side": "up"})).is_err());
}

#[test]
fn a_block_is_in_one_of_sixteen_languages() {
    let mut s = board();
    let (f, _) = block(&mut s, json!({"x": 0, "y": 0}));
    let lang = |s: &Session| node(s, f).board;
    s.execute("board.setLanguage", &json!({"id": f, "language": "python"})).unwrap();
    assert_eq!(lang(&s), Some(BoardItem::CodeBlock { language: CodeLanguage::Python }));
    for name in ["C++", "CPP"] {
        s.execute("board.setLanguage", &json!({"id": f, "language": name})).unwrap();
        assert_eq!(lang(&s), Some(BoardItem::CodeBlock { language: CodeLanguage::Cpp }));
    }
    for l in CodeLanguage::ALL {
        s.execute("board.setLanguage", &json!({"id": f, "language": l.id()})).unwrap();
        assert_eq!(lang(&s), Some(BoardItem::CodeBlock { language: l }));
    }
    s.execute("board.setLanguage", &json!({"id": f, "language": "go"})).unwrap();
    let e = s.execute("board.setLanguage", &json!({"id": f, "language": "klingon"})).unwrap_err().to_string();
    assert!(e.contains("typescript") && e.contains("plainText"), "{e}");
    let sticky = id(&s.execute("board.sticky", &json!({"x": 900, "y": 0})).unwrap(), "id");
    assert!(s.execute("board.setLanguage", &json!({"id": sticky, "language": "go"})).is_err());
    s.execute("select.none", &json!({})).unwrap();
    assert!(matches!(s.execute("board.setLanguage", &json!({"language": "go"})), Err(EngineError::Disabled(..))));
    // While typing in it, the text is the selection and the edit goes on.
    let t = text_of(s.doc().unwrap(), f).unwrap();
    s.execute("text.edit.begin", &json!({"id": t})).unwrap();
    s.execute("board.setLanguage", &json!({"language": "ruby"})).unwrap();
    assert_eq!(lang(&s), Some(BoardItem::CodeBlock { language: CodeLanguage::Ruby }));
    assert!(s.ds().unwrap().text_edit.is_some());
    s.execute("text.edit.key", &json!({"key": "Escape"})).unwrap();
    undo(&mut s);
    assert_eq!(lang(&s), Some(BoardItem::CodeBlock { language: CodeLanguage::Go }));
    // The creating command refuses what it cannot make.
    assert!(s.execute("board.codeBlock", &json!({"language": "nope"})).is_err());
    assert!(s.execute("board.codeBlock", &json!({"width": 5})).is_err());
    assert!(s.execute("board.codeBlock", &json!({"text": "x".repeat(CODE_MAX_CHARS + 1)})).is_err());
}

fn insert(s: &mut Session, text: &str) {
    s.execute("text.edit.insert", &json!({"text": text})).unwrap();
}

fn key(s: &mut Session, key: &str, shift: bool) {
    s.execute("text.edit.key", &json!({"key": key, "shift": shift})).unwrap();
}

fn characters(s: &Session, t: NodeId) -> String {
    match node(s, t).kind {
        NodeKind::Text(td) => td.characters,
        other => panic!("not text: {other:?}"),
    }
}

#[test]
fn typing_in_a_block_is_literal() {
    let mut s = board();
    let (f, t) = block(&mut s, json!({"x": 0, "y": 0}));
    assert!(is_code_text(s.doc().unwrap(), t));
    s.execute("text.edit.begin", &json!({"id": t})).unwrap();
    insert(&mut s, "-");
    insert(&mut s, " ");
    assert_eq!(characters(&s, t), "- ");
    assert!(td(&s, f).paragraphs.iter().all(|p| p.list == ListStyle::None) && td(&s, f).list == ListStyle::None);
    insert(&mut s, "\n1.");
    insert(&mut s, " ");
    assert_eq!(characters(&s, t), "- \n1. ");
    assert!(td(&s, f).paragraphs.iter().all(|p| p.list == ListStyle::None));
    // Tab types two spaces.
    s.execute("text.set", &json!({"id": t, "characters": ""})).unwrap();
    s.execute("text.edit.begin", &json!({"id": t, "caret": 0})).unwrap();
    key(&mut s, "Tab", false);
    assert_eq!(characters(&s, t), "  ");
    key(&mut s, "Tab", true);
    assert_eq!(characters(&s, t), "  ", "Shift+Tab does nothing");
    // Enter keeps the line's indent, and nothing more after a brace.
    insert(&mut s, "foo");
    key(&mut s, "Enter", false);
    assert_eq!(characters(&s, t), "  foo\n  ");
    s.execute("text.set", &json!({"id": t, "characters": "foo"})).unwrap();
    s.execute("text.edit.begin", &json!({"id": t, "caret": 3})).unwrap();
    key(&mut s, "Enter", false);
    assert_eq!(characters(&s, t), "foo\n");
    s.execute("text.set", &json!({"id": t, "characters": "{"})).unwrap();
    s.execute("text.edit.begin", &json!({"id": t, "caret": 1})).unwrap();
    key(&mut s, "Enter", false);
    assert_eq!(characters(&s, t), "{\n");
    // Nothing is made smart.
    s.execute("text.set", &json!({"id": t, "characters": ""})).unwrap();
    s.execute("text.edit.begin", &json!({"id": t, "caret": 0})).unwrap();
    let raw = "\"a\" -- 'b' ... -> != fi";
    insert(&mut s, raw);
    let got = characters(&s, t);
    assert_eq!(got, raw);
    assert!(!got.chars().any(|c| matches!(c, '\u{201C}' | '\u{2019}' | '\u{2014}' | '\u{2026}')));
    // In a sticky the same keys still make a list.
    let note = id(&s.execute("board.sticky", &json!({"x": 900, "y": 0})).unwrap(), "text");
    s.execute("text.edit.begin", &json!({"id": note})).unwrap();
    insert(&mut s, "-");
    insert(&mut s, " ");
    match node(&s, note).kind {
        NodeKind::Text(d) => assert!(d.list == ListStyle::Bulleted || d.paragraphs.iter().any(|p| p.list == ListStyle::Bulleted)),
        other => panic!("not text: {other:?}"),
    }
}

#[test]
fn lists_links_and_styled_ranges_do_not_stick() {
    let mut s = board();
    let (f, t) = block(&mut s, json!({"x": 0, "y": 0, "text": "one\ntwo"}));
    let plain = |s: &Session| {
        let d = td(s, f);
        assert_eq!(d.list, ListStyle::None);
        assert!(d.paragraphs.iter().all(|p| p.list == ListStyle::None));
        assert_eq!(d.style.link, None);
        assert_eq!((d.style.decoration, d.style.case), (TextDecoration::None, TextCase::Original));
        assert!(d.runs.is_empty());
        assert_eq!(d.characters, "one\ntwo");
        assert_eq!(d.auto_resize, TextAutoResize::Height);
    };
    for (cmd, p) in [
        ("text.bulletList", json!({"ids": [t]})),
        ("text.link", json!({"url": "https://example.com", "ids": [t]})),
        ("text.underline", json!({"ids": [t]})),
        ("text.caseUpper", json!({"ids": [t]})),
    ] {
        // Done or refused by name, and the block is plain either way.
        match s.execute(cmd, &p) {
            Ok(_) | Err(EngineError::BadParams { .. } | EngineError::Disabled(..)) => {}
            Err(other) => panic!("{cmd}: {other:?}"),
        }
        plain(&s);
    }
    s.execute("text.edit.begin", &json!({"id": t, "caret": 0})).unwrap();
    for _ in 0..3 {
        key(&mut s, "ArrowRight", true);
    }
    s.execute("text.bold", &json!({})).unwrap();
    s.execute("text.edit.key", &json!({"key": "Escape"})).unwrap();
    plain(&s);
    assert_eq!(td(&s, f).style.font_style, "Regular");
    s.execute("node.set", &json!({"id": t, "props": {"textAutoResize": "widthAndHeight"}})).unwrap();
    plain(&s);
    // A sticky's note keeps its link (the guard).
    let note = id(&s.execute("board.sticky", &json!({"x": 900, "y": 0, "text": "hi"})).unwrap(), "text");
    s.execute("text.link", &json!({"url": "https://example.com", "ids": [note]})).unwrap();
    match node(&s, note).kind {
        NodeKind::Text(d) => assert_eq!(d.style.link.as_deref(), Some("https://example.com")),
        other => panic!("not text: {other:?}"),
    }
}

#[test]
fn saved_files_live_sessions_and_a_second_pass() {
    let mut s = board();
    let (f, _) = block(&mut s, json!({"x": 100, "y": 200, "width": 400, "language": "rust", "text": "a\nb"}));
    let doc = s.doc().unwrap();
    let (back, _) = figcraft_format::from_json(&figcraft_format::to_json(doc).unwrap().to_string()).unwrap();
    let n = back.node(f).unwrap();
    assert_eq!(n.board, Some(BoardItem::CodeBlock { language: CodeLanguage::Rust }));
    assert_eq!((n.width, n.height), (400.0, 128.0));

    let mut s = board();
    let (f, t) = block(&mut s, json!({"x": 0, "y": 0}));
    let mut room = Room::new(s.doc().unwrap().clone(), 2, 7);
    room.s(0).execute("text.set", &json!({"id": t, "characters": ten(6)})).unwrap();
    room.s(1).execute("board.setLanguage", &json!({"id": f, "language": "go"})).unwrap();
    room.settle();
    room.check();
    let server = room.server.document();
    let n = server.node(f).unwrap();
    assert_eq!(n.height, 176.0);
    assert_eq!(n.board, Some(BoardItem::CodeBlock { language: CodeLanguage::Go }));

    // A second pass over a settled document changes nothing.
    let doc = s.doc().unwrap().clone();
    let mut again = doc.clone();
    again.mark_dirty(doc.ids().collect::<Vec<_>>());
    crate::normalize(&mut again, &doc);
    assert_eq!(again, doc);

    // Not on design files.
    let mut d = Session::new();
    assert_eq!(d.doc().unwrap().kind, FileKind::Design);
    assert!(matches!(d.execute("board.codeBlock", &json!({})), Err(EngineError::Disabled(..))));
    assert!(matches!(d.execute("board.setLanguage", &json!({"language": "go"})), Err(EngineError::Disabled(..))));
}

#[test]
fn code_blocks_survive_junk_commands() {
    let junk = [
        json!({}),
        json!(null),
        json!({"language": 7, "id": "x", "ids": [1], "side": 3, "text": [1]}),
        json!({"language": "nope", "id": 99_999, "side": "up", "width": 1e308, "x": "NaN"}),
    ];
    let ids: Vec<&str> = crate::cmd::all_commands()
        .iter()
        .map(|c| c.id)
        .filter(|i| {
            i.starts_with("text.") && *i != "text.rescanFonts" || i.starts_with("layout.") || matches!(*i, "board.nextSticky" | "board.setLanguage")
        })
        .collect();
    assert!(ids.len() > 20, "{ids:?}");
    for editing in [false, true] {
        for cmd in &ids {
            for j in &junk {
                let mut s = board();
                let (f, t) = block(&mut s, json!({"x": 0, "y": 0, "text": "a\nb"}));
                if editing {
                    s.execute("text.edit.begin", &json!({"id": t})).unwrap();
                } else {
                    s.set_selection(vec![f]);
                }
                if let Err(EngineError::Internal { cmd, msg }) = s.execute(cmd, j) {
                    panic!("{cmd} panicked on {j} (editing {editing}): {msg}");
                }
            }
        }
    }
}

#[test]
fn a_structured_text_is_refused_and_no_block_is_made() {
    for text in [json!({"a": 1}), json!([1]), json!(7), json!(true)] {
        let mut s = board();
        let before = s.doc().unwrap().ids().count();
        let err = s.execute("board.codeBlock", &json!({"x": 0, "y": 0, "text": text}));
        assert!(matches!(err, Err(EngineError::BadParams { .. })), "{text}: {err:?}");
        assert_eq!(s.doc().unwrap().ids().count(), before, "{text}");
    }
}

#[test]
fn the_pass_restores_padding_and_the_minimums() {
    let mut s = board();
    let (f, t) = block(&mut s, json!({"x": 0, "y": 0, "text": ten(6)}));
    assert_eq!(bx(&s, f).height(), 176.0);
    s.execute("node.set", &json!({"id": f, "props": {"padding": 0}})).unwrap();
    s.execute("text.set", &json!({"id": t, "characters": ten(6)})).unwrap();
    assert_eq!(node(&s, f).layout.unwrap().padding, [16.0, 16.0, 16.0, 64.0]);
    assert_eq!(bx(&s, f).height(), 176.0);

    let (f, t) = block(&mut s, json!({"x": 0, "y": 500}));
    s.execute("node.set", &json!({"id": f, "props": {"minHeight": null}})).unwrap();
    s.execute("text.set", &json!({"id": t, "characters": "a"})).unwrap();
    assert_eq!(node(&s, f).child_layout.min_h, Some(128.0));
    assert_eq!(bx(&s, f).height(), 128.0);
}
