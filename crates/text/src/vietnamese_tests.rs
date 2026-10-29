//! Vietnamese stacked diacritics (#1478): a letter and its marks are shaped with one face that has
//! them all, whether written precomposed (NFC) or decomposed (NFD). The face rule is checked with
//! stand-in faces, so it runs on any machine whatever fonts it has.

use crate::layout::pick_cluster_face;

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
    // The style's face lacks a mark (Ể typed decomposed in a Latin-1 font): the first fallback with
    // the letter and every mark, not one with the marks alone.
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
