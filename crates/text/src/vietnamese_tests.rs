//! Vietnamese stacked diacritics (#1478): a letter and its marks are shaped with one face, written
//! precomposed (NFC) or decomposed (NFD).

use figcraft_doc::TextData;

use crate::layout::pick_cluster_face;
use crate::{Glyph, TextLayout, family_names, find_face, layout, scan_system_fonts};

/// "Ể Ứ" precomposed and decomposed.
const NFC: &str = "\u{1EC2}\u{1EE8}";
const NFD: &str = "E\u{0302}\u{0309}U\u{031B}\u{0301}";

/// A stand-in face: the chars it has glyphs for, and whether those are colour glyphs.
struct Fake {
    chars: &'static str,
    color: bool,
}

const fn fake(chars: &'static str) -> Fake {
    Fake { chars, color: false }
}

fn pick<'a>(cluster: &str, primary: &'a Fake, emoji: &'a [Fake], fallbacks: &'a [Fake]) -> &'a str {
    let cl: Vec<char> = cluster.chars().collect();
    pick_cluster_face(&cl, primary, emoji, fallbacks, |f, c| f.chars.contains(c), |f| f.color).chars
}

#[test]
fn a_cluster_takes_one_face_that_has_all_its_chars() {
    let latin = fake("EU");
    let full = [fake("EU\u{0302}\u{0309}\u{031B}\u{0301}")];
    let marks_only = [fake("\u{0302}\u{0309}"), fake("EU\u{0302}\u{0309}")];
    // The style's own face has the whole cluster: it keeps it.
    assert_eq!(pick("E\u{0302}\u{0309}", &full[0], &[], &[]), full[0].chars);
    // The style's face lacks a mark: the first fallback with the letter and every mark, not one
    // with the marks alone.
    assert_eq!(pick("E\u{0302}\u{0309}", &latin, &[], &marks_only), "EU\u{0302}\u{0309}");
    // Joiners and variation selectors need no glyph of their own.
    assert_eq!(pick("E\u{200D}\u{FE0F}", &latin, &[], &full), "EU");
    // No face has the whole cluster: the letter's own face, then the first fallback with the letter.
    assert_eq!(pick("E\u{031B}\u{0303}", &latin, &[], &marks_only), "EU");
    assert_eq!(pick("Q\u{0302}", &fake("U"), &[], &[fake("\u{0302}"), fake("Q")]), "Q");
    assert_eq!(pick("Z\u{0302}", &latin, &[], &marks_only), "EU");
    // A line break stays with the style's face.
    assert_eq!(pick("\n", &latin, &[], &full), "EU");
}

#[test]
fn an_emoji_takes_a_colour_face_unless_the_style_has_colour_glyphs() {
    let emoji = [Fake { chars: "\u{1F600}\u{1F601}", color: true }];
    let mono = fake("\u{1F600}");
    assert_eq!(pick("\u{1F600}", &mono, &emoji, &[]), emoji[0].chars);
    let colour = Fake { chars: "\u{1F600}", color: true };
    assert!(std::ptr::eq(pick_cluster_face(&['\u{1F600}'], &colour, &emoji, &[], |f, c| f.chars.contains(c), |f| f.color), &colour));
}

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
    // A font with the letters but not the combining marks (most Latin-1 fonts); none on some systems.
    let Some(lacking) = family_names()
        .into_iter()
        .find(|f| find_face(f, "Regular").is_some_and(|face| face.has_char('E') && face.has_char('U') && !face.has_char('\u{0309}')))
    else {
        return;
    };
    // The precomposed letters come from a fallback face; without one there is nothing to compare.
    if lay_out(&lacking, NFC).glyphs.iter().any(|g| g.gid == 0) {
        return;
    }
    let l = lay_out(&lacking, NFD);
    assert!(l.glyphs.iter().all(|g| g.gid != 0), "{lacking}: a mark drew as a missing glyph");
    for start in [0, 3] {
        let gs = cluster(&l, start);
        assert!(gs.iter().all(|g| g.face.id == gs[0].face.id), "{lacking}: cluster {start} split across faces");
    }
}
