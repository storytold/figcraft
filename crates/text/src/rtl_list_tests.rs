//! Lists and paragraph indents in right-to-left paragraphs: the marker gutter and the indent are
//! mirrored to the right, numbers read "1." with the full stop on their left, wrapped lines hang
//! from the right, and trailing spaces don't push the text out of a right-aligned box.

use figcraft_doc::{ListStyle, TextAlignH, TextAutoResize, TextData};

use crate::{TextLayout, layout};

/// Hebrew words (shalom, olam, tov), from fonts on the system or the bundled set.
const SHALOM: &str = "\u{05E9}\u{05DC}\u{05D5}\u{05DD}";
const OLAM: &str = "\u{05E2}\u{05D5}\u{05DC}\u{05DD}";
const TOV: &str = "\u{05D8}\u{05D5}\u{05D1}";

/// Lay out `t` in a box `w` wide, with the system's fonts for scripts the bundled ones lack.
fn layout_in(t: &TextData, w: f64) -> TextLayout {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(crate::scan_system_fonts);
    layout(t, Some(w), None)
}

/// Whether every glyph has a font (else the test can't measure anything and is skipped).
fn drawable(l: &TextLayout) -> bool {
    let ok = !l.glyphs.is_empty() && l.glyphs.iter().all(|g| g.gid != 0);
    if !ok {
        eprintln!("no font for Hebrew; skipping");
    }
    ok
}

fn text(chars: &str, list: ListStyle, align: TextAlignH) -> TextData {
    let mut t = TextData::new(chars);
    t.style.font_size = 20.0;
    t.list = list;
    t.align_h = align;
    t.auto_resize = TextAutoResize::Height;
    t
}

/// Line `li`'s marker glyphs (they cover no chars), left to right.
fn marker(l: &TextLayout, li: usize) -> Vec<&crate::Glyph> {
    let ln = &l.lines[li];
    l.glyphs[ln.glyph_start..ln.glyph_end].iter().filter(|g| g.char_start == g.char_end).collect()
}

/// The left and right ends of line `li`'s visible text glyphs (not spaces).
fn text_span(l: &TextLayout, li: usize) -> (f64, f64) {
    let ln = &l.lines[li];
    let g = l.glyphs[ln.glyph_start..ln.glyph_end].iter().filter(|g| g.char_end > g.char_start && g.gid != g.face.as_ref().unwrap().glyph_for(' '));
    g.fold((f64::MAX, f64::MIN), |(a, b), g| (a.min(g.x), b.max(g.x + g.advance)))
}

#[test]
fn bullets_sit_right_of_right_to_left_text() {
    let l = layout_in(&text(&format!("{SHALOM} {OLAM}\nabc"), ListStyle::Bulleted, TextAlignH::Right), 300.0);
    if !drawable(&l) {
        return;
    }
    assert!(l.lines[0].rtl && !l.lines[1].rtl);
    // The Hebrew line: bullet on the right, ending at the box's right edge; the text left of it.
    let m = marker(&l, 0);
    assert_eq!(m.len(), 1);
    let (_, text_right) = text_span(&l, 0);
    assert!((m[0].x + m[0].advance - 300.0).abs() < 0.5, "{} + {}", m[0].x, m[0].advance);
    assert!(text_right < m[0].x, "{text_right} < {}", m[0].x);
    // The text keeps the gutter's width clear: 1.5 em.
    assert!((300.0 - text_right - 30.0).abs() < 0.5, "{text_right}");
    // The English paragraph of the same list keeps its bullet on the left of its text.
    let m = marker(&l, 1);
    let (text_left, _) = text_span(&l, 1);
    assert!(m[0].x < text_left);
}

#[test]
fn numbers_read_with_the_full_stop_on_their_left() {
    let paras = [SHALOM, OLAM, TOV].join("\n");
    let l = layout_in(&text(&paras, ListStyle::Numbered, TextAlignH::Right), 300.0);
    if !drawable(&l) {
        return;
    }
    assert_eq!(l.lines.len(), 3);
    let starts: Vec<f64> = (0..3).map(|li| marker(&l, li)[0].x).collect();
    for li in 0..3 {
        let m = marker(&l, li);
        // ".1", ".2", ".3": the full stop first from the left, then the digit.
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].gid, m[0].face.as_ref().unwrap().glyph_for('.'));
        assert_eq!(m[1].gid, m[1].face.as_ref().unwrap().glyph_for(char::from(b'1' + li as u8)));
        assert!(m[0].x < m[1].x);
        // Half an em between the text and the number; the numbers start at one x.
        let (_, text_right) = text_span(&l, li);
        assert!((m[0].x - text_right - 10.0).abs() < 0.5, "{} vs {text_right}", m[0].x);
        assert!((starts[li] - starts[0]).abs() < 1e-6);
        // Inside the box.
        assert!(m[1].x + m[1].advance <= 300.0 + 1e-6);
    }
}

#[test]
fn wrapped_right_to_left_list_lines_hang_from_the_right() {
    let words = [SHALOM, OLAM, TOV, SHALOM, OLAM, TOV, SHALOM].join(" ");
    let l = layout_in(&text(&words, ListStyle::Bulleted, TextAlignH::Right), 160.0);
    if !drawable(&l) {
        return;
    }
    assert!(l.lines.len() >= 2, "{}", l.lines.len());
    // Every line's text ends at the gutter (the box's right edge less 1.5 em); trailing spaces
    // hang outside on the left instead of pushing it right.
    for li in 0..l.lines.len() {
        let (left, right) = text_span(&l, li);
        assert!((right - (160.0 - 30.0)).abs() < 0.5, "line {li}: {right}");
        assert!(left >= -1e-6, "line {li}: {left}");
    }
    assert_eq!(marker(&l, 0).len(), 1);
    assert!(marker(&l, 1).is_empty(), "only the paragraph's first line has a bullet");
}

#[test]
fn right_to_left_paragraph_indent_is_on_the_right() {
    let words = [SHALOM, OLAM, TOV, SHALOM, OLAM, TOV, SHALOM].join(" ");
    let mut t = text(&words, ListStyle::None, TextAlignH::Right);
    t.paragraph_indent = 40.0;
    let l = layout_in(&t, 160.0);
    if !drawable(&l) {
        return;
    }
    assert!(l.lines.len() >= 2);
    let (_, first) = text_span(&l, 0);
    let (_, second) = text_span(&l, 1);
    assert!((first - 120.0).abs() < 0.5, "{first}");
    assert!((second - 160.0).abs() < 0.5, "{second}");
}

#[test]
fn left_aligned_and_justified_right_to_left_lists_keep_the_marker_after_the_text() {
    let words = [SHALOM, OLAM, TOV, SHALOM, OLAM, TOV, SHALOM].join(" ");
    for align in [TextAlignH::Left, TextAlignH::Justified, TextAlignH::Center] {
        let l = layout_in(&text(&words, ListStyle::Numbered, align), 160.0);
        if !drawable(&l) {
            return;
        }
        let m = marker(&l, 0);
        let (left, right) = text_span(&l, 0);
        assert!(m[0].x > right, "{align:?}: {} > {right}", m[0].x);
        assert!(m[1].x + m[1].advance <= 160.0 + 0.5, "{align:?}");
        assert!(left >= -1e-6, "{align:?}: {left}");
        if align == TextAlignH::Justified {
            // A justified line fills the box up to the gutter.
            assert!(left.abs() < 0.5, "{left}");
        }
    }
}
