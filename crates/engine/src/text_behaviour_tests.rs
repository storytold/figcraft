//! Text behaviour through commands (the rules of `figcraft_text`'s behaviour suite that involve
//! editing or settings): caret moves, deletes and word selection by grapheme cluster and by script,
//! vertical trim and paragraph indent through `node.set`, and renders and exports from one layout.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::textedit::{begin, key, select_word};
use figcraft_doc::{LeadingTrim, NodeId, NodeKind};

fn id(v: Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn text(s: &mut Session, chars: &str, props: Value) -> NodeId {
    id(s.execute("text.create", &json!({"text": chars, "x": 0, "y": 0, "props": props})).unwrap())
}

fn chars(s: &Session, t: NodeId) -> String {
    s.doc().unwrap().node(t).unwrap().kind.text().unwrap().characters.clone()
}

fn caret(s: &Session) -> (usize, usize) {
    let te = s.ds().unwrap().text_edit.clone().unwrap();
    (te.anchor, te.caret)
}

fn size(s: &Session, t: NodeId) -> (f64, f64) {
    let n = s.doc().unwrap().node(t).unwrap();
    (n.width, n.height)
}

const FAMILY: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";

#[test]
fn arrows_and_deletes_step_over_whole_clusters() {
    // A user-perceived character (emoji ZWJ sequence, flag, letter + combining mark) is one step.
    let mut s = Session::new();
    let src = format!("a{FAMILY}b\u{1F1EF}\u{1F1F5}e\u{0301}");
    let t = text(&mut s, &src, json!({"fontSize": 24}));
    begin(&mut s, t, Some(0), false).unwrap();
    // (key, caret after, derivation)
    for (k, want, why) in [
        ("Right", 1, "over 'a'"),
        ("Right", 6, "over the five-char family emoji"),
        ("Right", 7, "over 'b'"),
        ("Right", 9, "over the two regional indicators of a flag"),
        ("Right", 11, "over e + combining acute"),
        ("Right", 11, "the end stays"),
        ("Left", 9, "back over é"),
        ("Left", 7, "back over the flag"),
        ("Left", 6, "back over 'b'"),
        ("Left", 1, "back over the family"),
    ] {
        key(&mut s, k, false, false, false).unwrap();
        assert_eq!(caret(&s), (want, want), "{why}");
    }
    // Shift extends by clusters too.
    key(&mut s, "Right", true, false, false).unwrap();
    assert_eq!(caret(&s), (1, 6));
    key(&mut s, "Left", false, false, false).unwrap();
    // Delete and Backspace remove a whole cluster.
    key(&mut s, "Delete", false, false, false).unwrap();
    assert_eq!(chars(&s, t), "ab\u{1F1EF}\u{1F1F5}e\u{0301}");
    key(&mut s, "Right", false, false, true).unwrap(); // to the end
    key(&mut s, "Backspace", false, false, false).unwrap();
    assert_eq!(chars(&s, t), "ab\u{1F1EF}\u{1F1F5}");
    key(&mut s, "Backspace", false, false, false).unwrap();
    assert_eq!(chars(&s, t), "ab");
}

#[test]
fn word_selection_and_motion_follow_scripts() {
    // CJK text has no spaces: a double click or word move takes the run of one script (Han,
    // hiragana, katakana, Hangul); Latin words keep their combining marks. Documentation silent on
    // dictionary segmentation (a Han compound next to a Han word is one run).
    let mut s = Session::new();
    let src = "日本語のテキストです。한국어 단어 cafe\u{0301} ok";
    let t = text(&mut s, src, json!({"fontSize": 24}));
    begin(&mut s, t, Some(0), false).unwrap();
    // (double-click index, selection, derivation)
    for (at, want, why) in [
        (1, (0, 3), "日本語: the Han run"),
        (3, (3, 4), "の: one hiragana"),
        (5, (4, 8), "テキスト: the katakana run"),
        (9, (8, 10), "です: hiragana"),
        (10, (8, 10), "right after です (before 。) selects です"),
        (12, (11, 14), "한국어: the Hangul word"),
        (17, (15, 17), "단어: up to the space"),
        (20, (18, 23), "café with a combining accent is one word"),
    ] {
        select_word(&mut s, at).unwrap();
        assert_eq!(caret(&s), want, "{why}");
    }
    // Word moves stop at the same edges.
    crate::textedit::set_caret(&mut s, 0, false).unwrap();
    for want in [3, 4, 8, 10, 14, 17, 23, 26] {
        key(&mut s, "Right", false, true, false).unwrap();
        assert_eq!(caret(&s).1, want, "word right");
    }
    for want in [24, 18, 15, 11, 8, 4, 3, 0] {
        key(&mut s, "Left", false, true, false).unwrap();
        assert_eq!(caret(&s).1, want, "word left");
    }
    // The platforms' word keys reach the same moves: Ctrl+arrow on Windows and Linux, Alt on macOS.
    use crate::keymap::{Mods, Platform, text_motion};
    for (platform, m) in [(Platform::Other, Mods { ctrl: true, ..Default::default() }), (Platform::Mac, Mods { alt: true, ..Default::default() })] {
        crate::textedit::set_caret(&mut s, 0, false).unwrap();
        let (word, line) = text_motion(platform, "ArrowRight", m);
        key(&mut s, "ArrowRight", false, word, line).unwrap();
        assert_eq!(caret(&s).1, 3, "{platform:?}: one word right");
    }
}

#[test]
fn ime_composition_with_cjk() {
    // Composing shows inline and wraps like committed text; committing types it in one go.
    let mut s = Session::new();
    let t = text(&mut s, "x", json!({"fontSize": 24}));
    s.execute("node.set", &json!({"id": t, "props": {"textAutoResize": "height", "width": 60, "characters": ""}})).unwrap();
    begin(&mut s, t, Some(0), false).unwrap();
    crate::textedit::preedit(&mut s, "にほんごのぶんしょう", None).unwrap();
    let (doc, range, cursor) = crate::textedit::preedit_doc(&s).unwrap();
    assert_eq!((range, cursor), ((0, 10), 10));
    assert_eq!(chars(&s, t), "", "the document waits for the commit");
    let shown = doc.node(t).unwrap();
    if let Some((l, _)) = figcraft_text::layout_node(shown).filter(|(l, _)| !l.glyphs.is_empty()) {
        assert!(l.lines.len() > 1, "a long composition wraps in the 60 px box");
        assert!((shown.height - l.height).abs() < 1e-6, "the box grows with it");
    }
    crate::textedit::insert(&mut s, "日本語の文章").unwrap();
    assert_eq!(chars(&s, t), "日本語の文章");
    assert!(s.ds().unwrap().text_edit.as_ref().unwrap().preedit.is_none());
}

#[test]
fn vertical_trim_through_node_set() {
    let mut s = Session::new();
    let t = text(&mut s, "Hello\nworld", json!({"fontSize": 40, "lineHeight": 60}));
    let Some((l, _)) = figcraft_text::layout_node(s.doc().unwrap().node(t).unwrap()) else { return };
    if l.glyphs.is_empty() {
        return;
    }
    let (w, h) = size(&s, t);
    assert!((h - 120.0).abs() < 1e-6, "two 60 px lines");
    s.execute("node.set", &json!({"id": t, "props": {"leadingTrim": "capHeight"}})).unwrap();
    let n = s.doc().unwrap().node(t).unwrap();
    assert_eq!(n.kind.text().unwrap().leading_trim, LeadingTrim::CapHeight);
    let (l, _) = figcraft_text::layout_node(n).unwrap();
    let cap = l.lines[0].baseline;
    // The auto-size box now runs from the first cap height to the last baseline: cap + 60.
    assert!((n.height - (cap + 60.0)).abs() < 1e-6 && cap > 10.0 && cap < 40.0, "{} vs {cap}", n.height);
    assert_eq!(n.width, w, "the width doesn't change");
    // Descenders hang below the trimmed box: exports reach past it to keep them.
    let eb = figcraft_render::export_bounds(s.doc().unwrap(), t).unwrap();
    assert!(eb.y1 > n.height + 10.0, "{eb:?}");
    // The PNG's ink starts at the box's top (cap height at y = 0; ascenders like "l" may poke a few
    // px above it, by how much depends on the font); SVG and PDF use the same layout.
    let pix = figcraft_render::render_node(s.doc().unwrap(), t, 1.0).unwrap();
    let first_ink = (0..pix.height()).find(|y| (0..pix.width()).any(|x| pix.pixel(x, *y).is_some_and(|p| p.alpha() > 64)));
    assert!(first_ink.is_some_and(|y| (-8.0..=2.0).contains(&(f64::from(y) + eb.y0))), "{first_ink:?} {eb:?}");
    let svg = figcraft_svg::export_svg(s.doc().unwrap(), &[t], &figcraft_svg::SvgOptions::default()).unwrap();
    assert!(!svg.contains("NaN"));
    assert!(figcraft_pdf::export_nodes_pdf(s.doc().unwrap(), &[t]).is_ok());
    // Inspect and code say so; one undo step back; junk is refused.
    let css = crate::codegen::css_decls(s.doc().unwrap(), s.doc().unwrap().node(t).unwrap(), &mut Default::default(), (true, true));
    assert!(css.contains("text-box: trim-both cap alphabetic"), "{css}");
    s.undo().unwrap();
    assert_eq!(size(&s, t), (w, h));
    assert!(s.execute("node.set", &json!({"id": t, "props": {"leadingTrim": "sideways"}})).is_err());
    // The REST spelling.
    s.execute("node.set", &json!({"id": t, "props": {"leadingTrim": "CAP_HEIGHT"}})).unwrap();
    assert!(matches!(&s.doc().unwrap().node(t).unwrap().kind, NodeKind::Text(td) if td.leading_trim == LeadingTrim::CapHeight));
    assert_eq!(crate::inspect::node_json(s.doc().unwrap(), t, 0)["leadingTrim"], "capHeight", "inspect reports it");
}

#[test]
fn paragraph_indent_only_moves_left_aligned_text() {
    let mut s = Session::new();
    let t = text(&mut s, "Indented paragraph", json!({"fontSize": 20, "paragraphIndent": 30}));
    let first_x = |s: &Session| {
        let (l, _) = figcraft_text::layout_node(s.doc().unwrap().node(t).unwrap()).unwrap();
        (l.glyphs.first().map_or(0.0, |g| g.x), l.width)
    };
    let (x, w) = first_x(&s);
    if w <= 0.0 {
        return;
    }
    assert!((x - 30.0).abs() < 1e-6, "left: indented");
    s.execute("node.set", &json!({"id": t, "props": {"textAlign": "center"}})).unwrap();
    let (x, w2) = first_x(&s);
    assert!((w2 - (w - 30.0)).abs() < 1e-6 && x.abs() < 1e-6, "centered auto-width text: no indent, the box shrinks by it");
}

/// A new text made with the Text tool by a press at `from`, a drag to `to` and a release, holding
/// `chars`: its box and resize mode.
fn text_tool(from: (f64, f64), to: (f64, f64), chars: &str) -> (figcraft_geom::Rect, figcraft_doc::TextAutoResize) {
    use crate::tools::{PointerEvent, PointerKind, ToolId};
    let mut s = Session::new();
    s.set_tool(ToolId::Text);
    s.pointer(&PointerEvent::at(PointerKind::Down, from.0, from.1, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, to.0, to.1, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, to.0, to.1, 1.0)).unwrap();
    crate::textedit::insert(&mut s, chars).unwrap();
    let t = s.ds().unwrap().text_edit.as_ref().unwrap().id;
    let doc = s.doc().unwrap();
    let NodeKind::Text(td) = &doc.node(t).unwrap().kind else { panic!("not a text") };
    (doc.world_bounds(t).unwrap(), td.auto_resize)
}

#[test]
fn a_drag_with_the_text_tool_makes_a_fixed_size_box() {
    // #360: the drag's size, its top-left where the drag started, and its text wraps in it; a
    // click, or a drag within a few pixels, still makes auto-width text.
    use figcraft_doc::TextAutoResize;
    figcraft_text::scan_system_fonts();
    let long = "A sentence much longer than eighty pixels";
    let (line, auto) = text_tool((100.0, 100.0), (100.0, 100.0), long);
    assert_eq!(auto, TextAutoResize::WidthAndHeight);
    assert_eq!(text_tool((100.0, 100.0), (101.0, 101.0), long).1, TextAutoResize::WidthAndHeight, "a slip of the hand is a click");
    let (b, auto) = text_tool((100.0, 100.0), (180.0, 400.0), long);
    assert_eq!(auto, TextAutoResize::None);
    assert_eq!((b.x0, b.y0, b.width(), b.height()), (100.0, 100.0, 80.0, 300.0), "the text wraps in a box that keeps its size");
    // Dragged up and to the left, the box is the same.
    let (b2, _) = text_tool((180.0, 400.0), (100.0, 100.0), long);
    assert_eq!((b2.x0, b2.y0, b2.width(), b2.height()), (100.0, 100.0, 80.0, 300.0));
    // A flat drag still makes a box one line tall.
    let (b3, _) = text_tool((100.0, 100.0), (180.0, 101.0), "A");
    assert_eq!((b3.width(), b3.height()), (80.0, line.height()));
}
