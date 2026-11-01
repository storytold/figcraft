//! A line separator (U+2028: a soft break in `.fig` files, kept on import) breaks the
//! line inside its paragraph: no paragraph spacing or new list marker after it, and no glyph (#852).

use super::*;
use figcraft_doc::{LineHeight, ListStyle, TextAutoResize, TextData};

const EPS: f64 = 1e-6;

/// A 20 px text on 30 px lines in the default face, or None (skip) on a machine without fonts.
fn text(s: &str) -> Option<TextData> {
    scan_system_fonts();
    let f = db().write().unwrap().default_face()?;
    let mut t = TextData::new(s);
    (t.style.font_family, t.style.font_style) = (f.family.clone(), f.style.clone());
    t.style.font_size = 20.0;
    t.style.line_height = LineHeight::Pixels(30.0);
    t.auto_resize = TextAutoResize::WidthAndHeight;
    Some(t)
}

/// The chars of each line.
fn line_texts(t: &TextData, l: &TextLayout) -> Vec<String> {
    let chars: Vec<char> = t.characters.chars().collect();
    l.lines.iter().map(|ln| chars.get(ln.char_start..ln.char_end).unwrap_or(&[]).iter().collect::<String>()).collect()
}

#[test]
fn breaks_the_line_but_not_the_paragraph() {
    let Some(mut t) = text("one\u{2028}two\nthree") else { return };
    t.paragraph_spacing = 10.0;
    let l = layout(&t, None, None);
    assert_eq!(line_texts(&t, &l), ["one\u{2028}", "two\n", "three"]);
    assert!(!l.lines[0].para_end && l.lines[0].break_after);
    assert!(l.lines[1].para_end && l.lines[1].break_after);
    assert!(!l.lines[2].break_after);
    // Paragraph spacing after the newline only: 30, then 30 + 10.
    assert!((l.lines[1].top - 30.0).abs() < EPS && (l.lines[2].top - 70.0).abs() < EPS, "{} {}", l.lines[1].top, l.lines[2].top);
    // The separator draws nothing.
    assert!(l.glyphs.iter().all(|g| !(g.char_start..g.char_end).contains(&3)));
}

#[test]
fn before_a_newline_or_at_the_end_it_leaves_an_empty_line() {
    // "Hi,⏎¶Season" in #852: an empty line between the two.
    let Some(t) = text("Hi,\u{2028}\nSeason") else { return };
    let l = layout(&t, None, None);
    assert_eq!(line_texts(&t, &l), ["Hi,\u{2028}", "\n", "Season"]);
    let Some(t) = text("end\u{2028}") else { return };
    let l = layout(&t, None, None);
    assert_eq!(line_texts(&t, &l), ["end\u{2028}", ""]);
    assert!((l.height - 60.0).abs() < EPS, "{}", l.height);
}

#[test]
fn a_list_item_continues_after_it_without_a_new_marker() {
    let Some(mut t) = text("item\u{2028}more\nnext") else { return };
    t.list = ListStyle::Bulleted;
    let l = layout(&t, None, None);
    assert_eq!(line_texts(&t, &l), ["item\u{2028}", "more\n", "next"]);
    // Markers are the glyphs that cover no char: one per paragraph.
    assert_eq!(l.glyphs.iter().filter(|g| g.char_start == g.char_end).count(), 2);
    // The line after the separator lines up with the item's text, past the bullet.
    let text_x = |li: usize| l.glyphs.iter().find(|g| g.char_start == l.lines[li].char_start && g.char_end > g.char_start).map(|g| g.x);
    let (first, cont) = (text_x(0).unwrap(), text_x(1).unwrap());
    assert!(first > 0.0 && (first - cont).abs() < 1e-3, "{first} {cont}");
}

#[test]
fn carets_and_clicks_treat_it_like_a_newline() {
    let (Some(sep), Some(nl)) = (text("ab\u{2028}cd"), text("ab\ncd")) else { return };
    let (ls, ln) = (layout(&sep, None, None), layout(&nl, None, None));
    for i in 0..=5 {
        assert_eq!(ls.caret(i), ln.caret(i), "caret before char {i}");
    }
    for line in &ln.lines {
        for x in [-10.0, 5.0, line.width + 50.0] {
            assert_eq!(ls.hit(x, line.top + 5.0), ln.hit(x, line.top + 5.0), "click at {x} on the line at {}", line.top);
        }
    }
    // Before the separator: the end of the first line; after it: the start of the second.
    assert_eq!(ls.caret(2).1, 0);
    assert!(ls.caret(2).0 > 0.0);
    assert_eq!(ls.caret(3), (0.0, 1));
    // A click right of the first line lands before the separator, not after it.
    assert_eq!(ls.hit(ls.lines[0].width + 50.0, ls.lines[0].top + 5.0), 2);
}

#[test]
fn a_selection_across_it_shows_the_break_like_a_newline() {
    let (Some(sep), Some(nl)) = (text("ab\u{2028}cd"), text("ab\ncd")) else { return };
    let (ls, ln) = (layout(&sep, None, None), layout(&nl, None, None));
    let (before, across) = (ls.selection_rects(0, 2), ls.selection_rects(0, 5));
    assert_eq!((before.len(), across.len()), (1, 2));
    // Reaching past the separator adds the 4 px break mark at the first line's end.
    assert!((across[0].width() - before[0].width() - 4.0).abs() < EPS, "{} {}", across[0].width(), before[0].width());
    assert_eq!(across, ln.selection_rects(0, 5));
}
