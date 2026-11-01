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
    // Delete removes a whole cluster; Backspace takes a letter's marks one at a time (#1220) and
    // an emoji cluster whole.
    key(&mut s, "Delete", false, false, false).unwrap();
    assert_eq!(chars(&s, t), "ab\u{1F1EF}\u{1F1F5}e\u{0301}");
    key(&mut s, "Right", false, false, true).unwrap(); // to the end
    key(&mut s, "Backspace", false, false, false).unwrap();
    assert_eq!(chars(&s, t), "ab\u{1F1EF}\u{1F1F5}e");
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

/// Asserts `delete_surrounding_bytes` removes an exact UTF-8 byte range across CJK and a
/// supplementary emoji, leaves nothing past the start, and the whole edit is one undo step.
#[test]
fn ime_delete_surrounding_bytes_is_exact_and_one_undo_step() {
    // 0.37's `DeleteSurroundingBytes` counts UTF-8 bytes around the selection, before the paired
    // commit. Multibyte CJK and a supplementary emoji must be removed exactly, not as tofu or
    // extended to grapheme clusters, and the whole edit remains one undo step.
    let mut s = Session::new();
    let t = text(&mut s, "ab\u{4F60}\u{597D}c\u{1F600}d", json!({"fontSize": 24}));
    begin(&mut s, t, Some(4), false).unwrap();
    assert!(crate::textedit::delete_surrounding_bytes(&mut s, 6, 1).unwrap(), "你好 before and c after");
    assert_eq!(chars(&s, t), "ab\u{1F600}d");
    assert_eq!(caret(&s), (2, 2));
    crate::textedit::set_caret(&mut s, 3, false).unwrap();
    assert!(crate::textedit::delete_surrounding_bytes(&mut s, 4, 0).unwrap(), "the emoji is four bytes");
    assert_eq!(chars(&s, t), "abd");
    assert_eq!(caret(&s), (2, 2));
    // Past the start of the text: nothing more to delete, not an error.
    assert!(!crate::textedit::delete_surrounding_bytes(&mut s, 0, 0).unwrap());
    crate::textedit::end(&mut s).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(chars(&s, t), "ab\u{4F60}\u{597D}c\u{1F600}d", "one undo restores the whole session");
}

/// Asserts a byte range that only partly covers a character leaves it in place, while an
/// exact range deletes it, and a selection survives for the paired commit with its
/// direction preserved.
#[test]
fn ime_delete_surrounding_bytes_keeps_partial_characters_and_selection() {
    // The fork deletes the byte range exactly and leaves a character the range only partly
    // covers. FigCraft's editor counts characters with the same rule (mirroring egui's
    // `surrounding_bytes_to_chars`), including around a selection.
    let mut s = Session::new();
    let t = text(&mut s, "\u{e9}x\u{1F600}", json!({"fontSize": 24}));
    let del = |s: &mut Session, b: usize, a: usize| crate::textedit::delete_surrounding_bytes(s, b, a).unwrap();
    begin(&mut s, t, Some(1), false).unwrap();
    assert!(!del(&mut s, 1, 0), "one byte ends inside 'é' (two bytes): leave it");
    assert_eq!(chars(&s, t), "\u{e9}x\u{1F600}");
    crate::textedit::set_caret(&mut s, 2, false).unwrap();
    assert!(!del(&mut s, 0, 2), "two bytes start inside the four-byte emoji: leave it");
    assert_eq!(chars(&s, t), "\u{e9}x\u{1F600}");
    assert!(del(&mut s, 0, 4), "its four bytes cover it exactly");
    assert_eq!(chars(&s, t), "\u{e9}x");
    assert_eq!(caret(&s), (2, 2));
    // Complete bytes before the selection go; a character the range only partly covers stays,
    // and the selection itself is kept for the paired commit.
    crate::textedit::set_caret(&mut s, 1, false).unwrap();
    crate::textedit::set_caret(&mut s, 1, true).unwrap();
    crate::textedit::set_caret(&mut s, 2, true).unwrap();
    assert!(del(&mut s, 2, 2), "é before (two bytes); two bytes start inside the four-byte emoji");
    // The selected 'x' is kept for the paired commit, as in stock `TextEdit`; the caret moves left
    // with the deleted prefix and the selection direction is preserved.
    assert_eq!(chars(&s, t), "x");
    assert_eq!(caret(&s), (0, 1));
    crate::textedit::end(&mut s).unwrap();
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
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let t = text(&mut s, "Indented paragraph", json!({"fontSize": 20, "paragraphIndent": 30}));
    let first_x = |s: &Session| {
        let (l, _) = figcraft_text::layout_node(s.doc().unwrap().node(t).unwrap()).unwrap();
        (l.glyphs.first().map(|g| g.x), l.width)
    };
    // Without any font there are no glyphs, but the indent alone still gives the box a width.
    let (Some(x), w) = first_x(&s) else { return };
    assert!((x - 30.0).abs() < 1e-6, "left: indented");
    s.execute("node.set", &json!({"id": t, "props": {"textAlign": "center"}})).unwrap();
    let (x, w2) = first_x(&s);
    assert!((w2 - (w - 30.0)).abs() < 1e-6 && x.is_some_and(|x| x.abs() < 1e-6), "centered auto-width text: no indent, the box shrinks by it");
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

#[test]
fn new_text_starts_in_the_builds_default_font() {
    // `text.create` without a fontFamily and a click with the Text tool both start in
    // `figcraft_doc::DEFAULT_FONT` (Inter, unless the build names another with
    // `FIGCRAFT_DEFAULT_FONT`), and the family is written with the text.
    use crate::tools::{PointerEvent, PointerKind, ToolId};
    let mut s = Session::new();
    let made = text(&mut s, "Text", json!({}));
    s.set_tool(ToolId::Text);
    s.pointer(&PointerEvent::at(PointerKind::Down, 300.0, 300.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, 300.0, 300.0, 1.0)).unwrap();
    crate::textedit::insert(&mut s, "Tool").unwrap();
    let typed = s.ds().unwrap().text_edit.as_ref().unwrap().id;
    assert_ne!(made, typed);
    let doc = s.doc().unwrap();
    for t in [made, typed] {
        let td = doc.node(t).unwrap().kind.text().unwrap();
        assert_eq!(td.style.font_family, figcraft_doc::DEFAULT_FONT);
        assert_eq!(serde_json::to_value(td).unwrap()["style"]["font_family"], figcraft_doc::DEFAULT_FONT);
    }
}

fn resize_mode(s: &Session, t: NodeId) -> figcraft_doc::TextAutoResize {
    s.doc().unwrap().node(t).unwrap().kind.text().unwrap().auto_resize
}

#[test]
fn a_truncated_text_keeps_truncating_when_its_box_is_resized() {
    // #1376: a truncating text is already fixed both ways, so an explicit size changes its box only.
    use figcraft_doc::TextAutoResize;
    let mut s = Session::new();
    let t = text(&mut s, "Lorem consectetur", json!({"textAutoResize": "truncate", "width": 100, "height": 24}));
    assert_eq!(resize_mode(&s, t), TextAutoResize::Truncate);
    for (cmd, p, why) in [
        ("node.set", json!({"id": t, "props": {"height": 24}}), "node.set height (the value it has)"),
        ("node.set", json!({"id": t, "props": {"width": 120, "height": 30}}), "node.set width and height"),
        ("transform.resize", json!({"ids": [t], "height": 40}), "transform.resize height"),
        ("transform.resize", json!({"ids": [t], "width": 90}), "transform.resize width"),
        ("node.set", json!({"id": t, "props": {"sizingV": "fixed"}}), "fixed vertical sizing"),
    ] {
        s.execute(cmd, &p).unwrap();
        assert_eq!(resize_mode(&s, t), TextAutoResize::Truncate, "{why}");
    }
    // Text that doesn't truncate still turns fixed size.
    let plain = text(&mut s, "Lorem consectetur", json!({"textAutoResize": "height", "width": 100}));
    s.execute("node.set", &json!({"id": plain, "props": {"height": 24}})).unwrap();
    assert_eq!(resize_mode(&s, plain), TextAutoResize::None);
}

#[test]
fn max_lines_on_auto_height_text_hugs_that_many_lines() {
    // #1377: an auto-height text with max lines grows up to that many lines and ends in an
    // ellipsis; the layer reports its max lines.
    use figcraft_doc::TextAutoResize;
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let t = text(&mut s, "Lorem ipsum dolor sit amet consectetur", json!({"fontSize": 16, "lineHeight": 24}));
    s.execute("node.set", &json!({"id": t, "props": {"textAutoResize": "height", "width": 100}})).unwrap();
    assert!(crate::inspect::node_json(s.doc().unwrap(), t, 0).get("maxLines").is_none(), "no limit: not reported");
    // Wrapping needs a font (CI may have none).
    if size(&s, t).1 < 48.0 {
        return;
    }
    s.execute("text.maxLines", &json!({"ids": [t], "lines": 1})).unwrap();
    assert_eq!(resize_mode(&s, t), TextAutoResize::Height, "max lines keep auto height");
    assert_eq!(size(&s, t), (100.0, 24.0), "one line tall");
    assert_eq!(crate::inspect::node_json(s.doc().unwrap(), t, 0)["maxLines"], json!(1));
    s.execute("node.set", &json!({"id": t, "props": {"maxLines": 2}})).unwrap();
    assert_eq!(size(&s, t).1, 48.0, "two lines tall");
    // No limit: it grows to all of its lines again.
    s.execute("node.set", &json!({"id": t, "props": {"maxLines": 0}})).unwrap();
    assert!(size(&s, t).1 > 48.0);
    // An explicit height on a text that truncates makes a truncating box.
    s.execute("node.set", &json!({"id": t, "props": {"maxLines": 1}})).unwrap();
    s.execute("node.set", &json!({"id": t, "props": {"height": 30}})).unwrap();
    assert_eq!((resize_mode(&s, t), size(&s, t).1), (TextAutoResize::Truncate, 30.0));
    // Other text still becomes a truncating box with max lines.
    let fixed = text(&mut s, "Lorem", json!({"textAutoResize": "none", "width": 100, "height": 100}));
    s.execute("text.maxLines", &json!({"ids": [fixed], "lines": 2})).unwrap();
    assert_eq!(resize_mode(&s, fixed), TextAutoResize::Truncate);
}

/// A synthetic monospaced TrueType font (outline-less glyphs, every printable ASCII character 500 of
/// 1000 units wide), loaded into the font database so text measures the same on every machine.
fn test_mono_font() -> &'static str {
    const FAMILY: &str = "FigCraft Test Mono";
    static LOADED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    LOADED.get_or_init(|| {
        let families = figcraft_text::load_font_bytes(synthetic_mono_font(FAMILY));
        assert_eq!(families, [FAMILY], "the synthetic font loads");
    });
    FAMILY
}

fn synthetic_mono_font(family: &str) -> Vec<u8> {
    let be16 = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_be_bytes());
    let glyphs: u16 = 1 + (0x7E - 0x20 + 1);
    let mut head = Vec::new();
    for x in [0x0001_0000u32, 0x0001_0000, 0, 0x5F0F_3CF5] {
        head.extend_from_slice(&x.to_be_bytes());
    }
    be16(&mut head, 0); // flags
    be16(&mut head, 1000); // unitsPerEm
    head.extend_from_slice(&[0; 16]); // created, modified
    for x in [0u16, (-200i16) as u16, 500, 800, 0, 8, 2, 0, 0] {
        be16(&mut head, x); // bbox, macStyle, lowestRecPPEM, direction hint, short loca, glyph format
    }
    let mut hhea = 0x0001_0000u32.to_be_bytes().to_vec();
    for x in [800u16, (-200i16) as u16, 0, 500, 0, 0, 500, 1, 0, 0, 0, 0, 0, 0, 0, glyphs] {
        be16(&mut hhea, x);
    }
    let mut maxp = 0x0001_0000u32.to_be_bytes().to_vec();
    be16(&mut maxp, glyphs);
    for x in [0u16, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0] {
        be16(&mut maxp, x);
    }
    let mut hmtx = Vec::new();
    for _ in 0..glyphs {
        be16(&mut hmtx, 500);
        be16(&mut hmtx, 0);
    }
    let loca = vec![0; 2 * (usize::from(glyphs) + 1)];
    let glyf = vec![0; 4];
    // cmap: one Windows Unicode BMP subtable, format 4, 0x20..=0x7E → glyphs 1.. plus the end segment.
    let mut cmap = Vec::new();
    for x in [0u16, 1, 3, 1, 0, 12] {
        be16(&mut cmap, x);
    }
    for x in [4u16, 32, 0, 4, 4, 1, 0, 0x7E, 0xFFFF, 0, 0x20, 0xFFFF, (1i16 - 0x20) as u16, 1, 0, 0] {
        be16(&mut cmap, x);
    }
    let strings = [(1u16, family.to_string()), (2, "Regular".into()), (6, family.replace(' ', "") + "-Regular")];
    let mut name = Vec::new();
    be16(&mut name, 0);
    be16(&mut name, strings.len() as u16);
    be16(&mut name, 6 + 12 * strings.len() as u16);
    let mut storage = Vec::new();
    for (id, s) in &strings {
        let utf16: Vec<u8> = s.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for x in [3u16, 1, 0x409, *id, utf16.len() as u16, storage.len() as u16] {
            be16(&mut name, x);
        }
        storage.extend(utf16);
    }
    name.extend(storage);
    let mut post = 0x0003_0000u32.to_be_bytes().to_vec();
    post.extend_from_slice(&[0; 28]);
    let tables: [(&[u8; 4], Vec<u8>); 9] = [
        (b"cmap", cmap),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca),
        (b"maxp", maxp),
        (b"name", name),
        (b"post", post),
    ];
    let mut font = 0x0001_0000u32.to_be_bytes().to_vec();
    for x in [tables.len() as u16, 128, 3, 16] {
        be16(&mut font, x);
    }
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in &tables {
        font.extend_from_slice(*tag);
        font.extend_from_slice(&0u32.to_be_bytes());
        font.extend_from_slice(&(offset as u32).to_be_bytes());
        font.extend_from_slice(&(data.len() as u32).to_be_bytes());
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in &tables {
        font.extend_from_slice(data);
        font.resize(font.len().next_multiple_of(4), 0);
    }
    font
}

/// The world position of a text's top-right corner (where right-aligned text is anchored).
fn top_right(s: &Session, t: NodeId) -> (f64, f64) {
    let doc = s.doc().unwrap();
    let p = doc.world_transform(t) * figcraft_geom::kurbo::Point::new(doc.node(t).unwrap().width, 0.0);
    (p.x, p.y)
}

fn assert_near(a: (f64, f64), b: (f64, f64), why: &str) {
    assert!((a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6, "{why}: {a:?} != {b:?}");
}

#[test]
fn right_aligned_auto_width_text_grows_and_shrinks_to_the_left() {
    let font = test_mono_font();
    for (src, rotation) in [("ANCHOR TEST\nabc", 0.0), ("abc", 0.0), ("ANCHOR TEST\nabc", 90.0)] {
        let mut s = Session::new();
        let t = text(&mut s, src, json!({"fontSize": 40, "fontFamily": font, "textAlign": "right", "textAutoResize": "widthAndHeight"}));
        s.execute("node.set", &json!({"id": t, "props": {"x": 200, "y": 50, "rotation": rotation}})).unwrap();
        let anchor = top_right(&s, t);
        let w0 = size(&s, t).0;
        let widest = src.lines().map(|l| l.chars().count()).max().unwrap_or(0);
        assert!((w0 - 20.0 * widest as f64).abs() < 1e-6, "{src:?}: the test font measures 20 px a character, not {w0}");
        begin(&mut s, t, Some(src.chars().count()), false).unwrap();
        for c in "defghijklmnopqrstuvwxyz".chars() {
            crate::textedit::insert(&mut s, &c.to_string()).unwrap();
            assert_near(top_right(&s, t), anchor, &format!("{src:?} at {rotation}°: typing {c}"));
        }
        assert!(size(&s, t).0 > w0 + 1e-6, "{src:?} at {rotation}°: typing grows the text");
        for _ in 0..23 {
            key(&mut s, "Backspace", false, false, false).unwrap();
            assert_near(top_right(&s, t), anchor, &format!("{src:?} at {rotation}°: deleting"));
        }
        assert!((size(&s, t).0 - w0).abs() < 1e-6, "back to the starting width");
    }
}

#[test]
fn right_aligned_auto_width_growth_undoes_to_its_old_place() {
    let font = test_mono_font();
    let mut s = Session::new();
    let t = text(&mut s, "abc", json!({"fontSize": 40, "fontFamily": font, "textAlign": "right", "textAutoResize": "widthAndHeight"}));
    let before = (s.doc().unwrap().node(t).unwrap().transform, size(&s, t));
    let anchor = top_right(&s, t);
    s.execute("node.set", &json!({"id": t, "props": {"characters": "abcdefghijklmnop"}})).unwrap();
    assert_near(top_right(&s, t), anchor, "set characters");
    let after = (s.doc().unwrap().node(t).unwrap().transform, size(&s, t));
    assert!(after.1.0 > before.1.0 + 1e-6, "the text grew");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((s.doc().unwrap().node(t).unwrap().transform, size(&s, t)), before, "undo");
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!((s.doc().unwrap().node(t).unwrap().transform, size(&s, t)), after, "redo");
}

#[test]
fn auto_layout_places_right_aligned_auto_width_text() {
    let font = test_mono_font();
    let mut s = Session::new();
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 100})).unwrap());
    s.execute("node.set", &json!({"id": f, "props": {"layout": "horizontal", "padding": 10, "gap": 0}})).unwrap();
    let t = id(s
        .execute(
            "text.create",
            &json!({"text": "abc", "x": 0, "y": 0, "parent": f, "props": {"fontSize": 40, "fontFamily": font, "textAlign": "right", "textAutoResize": "widthAndHeight"}}),
        )
        .unwrap());
    s.execute("node.set", &json!({"id": t, "props": {"characters": "abcdefghijklmnop"}})).unwrap();
    let c = s.doc().unwrap().node(t).unwrap().transform.as_coeffs();
    assert_eq!((c[4], c[5]), (10.0, 10.0), "the auto layout keeps it at its padding");
}

#[test]
fn css_font_family_escapes_backslash_and_newline() {
    let mut s = Session::new();
    let t = text(&mut s, "abc", json!({"fontFamily": "Foo\\"}));
    let css = crate::codegen::css_decls(s.doc().unwrap(), s.doc().unwrap().node(t).unwrap(), &mut Default::default(), (true, true));
    assert!(css.contains(r#"font-family: "Foo\\";"#), "{css}");
    let t = text(&mut s, "abc", json!({"fontFamily": "Foo\nBar"}));
    let css = crate::codegen::css_decls(s.doc().unwrap(), s.doc().unwrap().node(t).unwrap(), &mut Default::default(), (true, true));
    assert!(css.contains(r#"font-family: "Foo\A Bar";"#), "{css}");
    // A quote stays a quote, and a name can't close the page's <style> block.
    let t = text(&mut s, "abc", json!({"fontFamily": "Foo\"</style>"}));
    let css = crate::codegen::css_decls(s.doc().unwrap(), s.doc().unwrap().node(t).unwrap(), &mut Default::default(), (true, true));
    assert!(css.contains(r#"font-family: "Foo\"\3C /style>";"#), "{css}");
}
