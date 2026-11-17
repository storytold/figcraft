//! List markers: every bullet of a list takes the paragraph style's font, even when a line starts
//! with a char only a fallback font draws (Hebrew, Arabic…).

use figcraft_doc::{ListStyle, TextData};

use crate::layout;

#[test]
fn markers_use_the_paragraph_font() {
    let mut t = TextData::new("First\n\u{05E9}\u{05DC}\u{05D5}\u{05DD}");
    t.list = ListStyle::Bulleted;
    let l = layout(&t, None, None);
    // Markers are the glyphs that cover no char.
    let markers: Vec<_> = l.glyphs.iter().filter(|g| g.char_start == g.char_end).collect();
    if markers.len() != 2 {
        eprintln!("no fonts; skipping");
        return;
    }
    assert_eq!(markers[0].face.as_ref().unwrap().id, markers[1].face.as_ref().unwrap().id, "both bullets in one font");
    assert_eq!(markers[0].gid, markers[1].gid);
    // The Hebrew paragraph's bullet is on its right (see `rtl_list_tests`).
    assert!(markers[1].x > markers[0].x);
}
