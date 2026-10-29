//! Vietnamese stacked diacritics (#1478): a letter and its marks are shaped with one face, written
//! precomposed (NFC) or decomposed (NFD), and the font's mark positioning stacks them over the letter.

use figcraft_doc::TextData;

use crate::{Glyph, TextLayout, family_names, find_face, layout, scan_system_fonts};

/// "Ể Ứ" precomposed and decomposed.
const NFC: &str = "\u{1EC2}\u{1EE8}";
const NFD: &str = "E\u{0302}\u{0309}U\u{031B}\u{0301}";

fn lay_out(family: &str, s: &str) -> TextLayout {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(scan_system_fonts);
    let mut t = TextData::new(s);
    t.style.font_family = family.into();
    t.style.font_size = 100.0;
    layout(&t, None, None)
}

fn cluster(l: &TextLayout, start: usize) -> Vec<&Glyph> {
    l.glyphs.iter().filter(|g| g.char_start == start).collect()
}

#[test]
fn decomposed_marks_fall_back_with_their_letter() {
    // A font with the letters but not the combining marks (most Latin-1 fonts).
    let Some(lacking) = family_names()
        .into_iter()
        .find(|f| find_face(f, "Regular").is_some_and(|face| face.has_char('E') && face.has_char('U') && !face.has_char('\u{0309}')))
    else {
        eprintln!("no font without Vietnamese marks; skipping");
        return;
    };
    // The precomposed letters come from a fallback face; without one there is nothing to compare.
    if lay_out(&lacking, NFC).glyphs.iter().any(|g| g.gid == 0) {
        eprintln!("no fallback font for Vietnamese; skipping");
        return;
    }
    let l = lay_out(&lacking, NFD);
    assert!(l.glyphs.iter().all(|g| g.gid != 0), "{lacking}: a mark drew as a missing glyph");
    for start in [0, 3] {
        let gs = cluster(&l, start);
        assert!(!gs.is_empty());
        assert!(gs.iter().all(|g| g.face.id == gs[0].face.id), "{lacking}: cluster {start} split across faces");
    }
}

#[test]
fn stacked_marks_sit_over_their_letter() {
    // Q has no precomposed form with these marks, so the font's mark positioning places them.
    let Some(family) = ["Arial", "Segoe UI", "DejaVu Sans", "Liberation Sans", "Noto Sans"]
        .into_iter()
        .find(|f| find_face(f, "Regular").is_some_and(|face| face.family == *f && ['Q', '\u{0302}', '\u{0309}'].iter().all(|c| face.has_char(*c))))
    else {
        eprintln!("no font with mark positioning; skipping");
        return;
    };
    let l = lay_out(family, "Q\u{0302}\u{0309}");
    let [base, circumflex, hook] = l.glyphs.as_slice() else { panic!("{family}: {} glyphs", l.glyphs.len()) };
    assert!([circumflex, hook].iter().all(|m| m.face.id == base.face.id && m.gid != 0));
    // Where the marks land is the font's own anchor data; it's checked only in fonts whose anchors
    // stack these marks over Q (other builds, e.g. on Linux runners, place them differently).
    if !matches!(family, "Arial" | "Segoe UI") {
        return;
    }
    for m in [circumflex, hook] {
        assert!(m.x > base.x && m.x < base.x + base.advance, "{family}: mark beside the letter at {}", m.x);
        assert!(m.y < base.y, "{family}: mark not raised");
    }
    assert!(hook.y < circumflex.y, "{family}: the hook doesn't stack above the circumflex");
}
