//! Which family an SVG's `font-family` list gives a text layer.

use usvg::FontFamily;

use super::{Fonts, family};

fn fonts(installed: &[&str]) -> Fonts {
    Fonts(installed.iter().map(|f| f.to_lowercase()).collect())
}

fn named(list: &[&str]) -> Vec<FontFamily> {
    list.iter()
        .map(|n| match *n {
            "sans-serif" => FontFamily::SansSerif,
            n => FontFamily::Named(n.to_string()),
        })
        .collect()
}

/// The names CSS gives the system font keep their name on a Mac, where they draw in its System
/// Font, and fall back as before without it.
#[test]
fn css_system_font_names_keep_their_name_where_the_system_font_is() {
    let mac = fonts(&["System Font", "Inter", "Helvetica"]);
    assert_eq!(family(&named(&["-apple-system", "BlinkMacSystemFont", "system-ui", "sans-serif"]), &mac), "-apple-system");
    assert_eq!(family(&named(&["BlinkMacSystemFont", "sans-serif"]), &mac), "BlinkMacSystemFont");
    assert_eq!(family(&named(&["system-ui", "sans-serif"]), &mac), "system-ui");
    assert_eq!(family(&named(&["Helvetica", "system-ui"]), &mac), "Helvetica", "an installed family the SVG lists first still wins");
    let elsewhere = fonts(&["Inter", "DejaVu Sans"]);
    assert_eq!(family(&named(&["-apple-system", "BlinkMacSystemFont", "system-ui", "sans-serif"]), &elsewhere), "Inter");
    assert_eq!(family(&named(&["-apple-system"]), &elsewhere), "-apple-system", "nothing installed: the first name it gives");
}

#[test]
fn char_index_counts_the_characters_before_a_byte_offset() {
    let t = "aé😀b";
    let ci = super::char_index(t, [0, 1, 3, 7, 8, 2, 4, 99]);
    assert_eq!([ci(0), ci(1), ci(3), ci(7), ci(8)], [0, 1, 2, 3, 4]);
    assert_eq!([ci(2), ci(4), ci(99)], [4, 4, 4], "inside a character or past the end: all of them");
}
