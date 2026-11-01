//! Line breaks a file saved with its text (an imported `.fig`'s layout): followed while the text
//! and its box are as saved, so a font whose advances differ slightly from the file's doesn't move
//! a word to the next line or split one, and dropped by any edit that could change the wrap.

use std::sync::Arc;

use figcraft_doc::{TextAutoResize, TextData};

use super::{db, layout, layout_cached, scan_system_fonts};

/// A 20 px auto-height text in the default face; None (skip) on a machine without fonts.
fn text(s: &str) -> Option<TextData> {
    scan_system_fonts();
    let f = db().write().unwrap().default_face()?;
    let mut t = TextData::new(s);
    (t.style.font_family, t.style.font_style) = (f.family.clone(), f.style.clone());
    t.style.font_size = 20.0;
    t.auto_resize = TextAutoResize::Height;
    Some(t)
}

/// How wide `s` sets on one line.
fn width(t: &TextData, s: &str) -> f64 {
    layout(&TextData { characters: s.into(), auto_resize: TextAutoResize::WidthAndHeight, ..t.clone() }, None, None).width
}

fn lines(t: &TextData, w: f64) -> Vec<String> {
    let chars: Vec<char> = t.characters.chars().collect();
    layout(t, Some(w), None).lines.iter().map(|l| chars.get(l.char_start..l.char_end).unwrap_or(&[]).iter().collect()).collect()
}

#[test]
fn saved_breaks_keep_a_word_our_metrics_would_push_to_the_next_line() {
    let Some(mut t) = text("Lorem ipsum a") else { return };
    // A box 1% narrower than "Lorem ipsum": the file's font fitted it, ours doesn't.
    let w = width(&t, "Lorem ipsum") * 0.99;
    assert_eq!(lines(&t, w), ["Lorem ", "ipsum a"], "unhinted, the word wraps early");
    t.save_breaks(vec![12], w);
    assert_eq!(lines(&t, w), ["Lorem ipsum ", "a"]);
}

#[test]
fn saved_breaks_keep_a_word_whole_in_a_narrow_box() {
    let Some(mut t) = text("Followers") else { return };
    let w = width(&t, "Followers") * 0.99;
    assert_eq!(lines(&t, w).len(), 2, "unhinted, the word splits");
    t.save_breaks(vec![], w);
    assert_eq!(lines(&t, w), ["Followers"]);
}

#[test]
fn saved_breaks_stop_applying_after_a_layout_edit_or_resize() {
    let Some(mut t) = text("Lorem ipsum a") else { return };
    let w = width(&t, "Lorem ipsum") * 0.99;
    t.save_breaks(vec![12], w);
    // Links and vertical settings don't move breaks: the saved ones still hold.
    let mut painted = t.clone();
    painted.style.link = Some("https://example.com".into());
    painted.paragraph_spacing = 8.0;
    assert_eq!(lines(&painted, w), ["Lorem ipsum ", "a"]);
    let mut typed = t.clone();
    typed.characters = "Lorem ipsum ab".into();
    let mut sized = t.clone();
    sized.style.font_size = 21.0;
    for (edited, why) in [(&typed, "chars"), (&sized, "size")] {
        assert_eq!(edited.saved_breaks_for(w), None, "{why}");
    }
    assert_eq!(lines(&typed, w), ["Lorem ", "ipsum ab"]);
    assert_eq!(t.saved_breaks_for(w + 1.0), None, "another box width");
    // Hinted and unhinted texts are told apart by the layout cache.
    let plain = TextData { saved_breaks: None, ..t.clone() };
    assert!(!Arc::ptr_eq(&layout_cached(&t, Some(w), None), &layout_cached(&plain, Some(w), None)));
}

#[test]
fn saved_breaks_that_would_overflow_the_box_are_dropped() {
    let Some(mut t) = text("Lorem ipsum a") else { return };
    // 10% narrower: a font that much wider isn't the one the file was laid out with.
    let w = width(&t, "Lorem ipsum") * 0.9;
    let own = lines(&t, w);
    t.save_breaks(vec![12], w);
    assert_eq!(lines(&t, w), own);
}

#[test]
fn saved_breaks_leave_hard_breaks_and_bad_starts_alone() {
    let Some(mut t) = text("Lorem ipsum a\nb") else { return };
    let w = width(&t, "Lorem ipsum") * 0.99;
    t.save_breaks(vec![12], w);
    assert_eq!(lines(&t, w), ["Lorem ipsum ", "a\n", "b"]);
    for junk in [vec![0], vec![12, 12], vec![15], vec![40]] {
        t.save_breaks(junk.clone(), w);
        assert_eq!(t.saved_breaks_for(w), None, "{junk:?}");
    }
}
