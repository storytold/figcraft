//! Icon fonts and the font database: registering a document's fonts, rebuilding one whose outlines
//! changed, and dropping the families that are gone.

use super::*;
use figcraft_doc::IconGlyph;
use figcraft_geom::Shape;

/// A square in the em square, y down: the shape `to_em_square` stores.
fn square(size: f64) -> String {
    let mut p = BezPath::new();
    p.move_to((100.0, 100.0));
    p.line_to((100.0 + size, 100.0));
    p.line_to((100.0 + size, 100.0 + size));
    p.line_to((100.0, 100.0 + size));
    p.close_path();
    p.to_svg()
}

fn glyph(name: &str, codepoint: u32, size: f64) -> IconGlyph {
    IconGlyph { id: format!("g-{name}"), name: name.into(), codepoint, path: square(size), node: None }
}

fn font(name: &str, glyphs: Vec<IconGlyph>) -> IconFont {
    IconFont { id: format!("id-{name}"), name: name.into(), glyphs }
}

fn doc(fonts: Vec<IconFont>) -> Document {
    let mut doc = Document::new("Test");
    doc.icon_fonts = fonts;
    doc
}

fn registered(family: &str) -> bool {
    fontdb::family_names().iter().any(|n| n == family)
}

/// The family's face, or a panic naming what was looked for.
fn face(family: &str) -> std::sync::Arc<fontdb::Face> {
    let f = fontdb::find_face(family, "Regular").expect("the database has a face to fall back on");
    assert_eq!(f.family, family, "the family is registered (not just the fallback face)");
    f
}

/// The width of the glyph `family` maps `c` to, in font units.
fn glyph_width(family: &str, c: char) -> f64 {
    let f = face(family);
    assert!(f.has_char(c), "{family} maps {c:?}");
    let bb = f.outline(f.glyph_for(c)).bounding_box();
    bb.width()
}

#[test]
fn a_documents_icon_font_becomes_a_family_text_can_use() {
    let _guard = test_guard();
    let doc = doc(vec![font("Test Icons", vec![glyph("Home", 0xE000, 700.0)])]);
    assert_eq!(sync_icon_fonts(&doc), 1, "the first sync builds it");
    assert!(registered("Test Icons"));
    assert!((glyph_width("Test Icons", '\u{E000}') - 700.0).abs() < 2.0);
    // Only the icons are mapped: a text layer typing a letter gets nothing from this family.
    assert!(!face("Test Icons").has_char('A'));
}

#[test]
fn an_unchanged_font_is_not_built_again() {
    let _guard = test_guard();
    let doc = doc(vec![font("Steady Icons", vec![glyph("Home", 0xE000, 700.0)])]);
    assert_eq!(sync_icon_fonts(&doc), 1);
    assert_eq!(sync_icon_fonts(&doc), 0, "nothing changed, so nothing is compiled");
    assert!(registered("Steady Icons"), "and it stays registered");
}

#[test]
fn an_edited_outline_rebuilds_the_font() {
    let _guard = test_guard();
    let mut doc = doc(vec![font("Editing Icons", vec![glyph("Home", 0xE000, 700.0)])]);
    sync_icon_fonts(&doc);
    assert!((glyph_width("Editing Icons", '\u{E000}') - 700.0).abs() < 2.0);
    // Redraw the icon smaller.
    doc.icon_fonts[0].glyphs[0].path = square(300.0);
    assert_eq!(sync_icon_fonts(&doc), 1, "the outline changed, so it is compiled again");
    assert!((glyph_width("Editing Icons", '\u{E000}') - 300.0).abs() < 2.0);
}

#[test]
fn a_font_that_is_gone_loses_its_family() {
    let _guard = test_guard();
    let both = doc(vec![font("Kept Icons", vec![glyph("Home", 0xE000, 700.0)]), font("Dropped Icons", vec![glyph("Home", 0xE000, 700.0)])]);
    sync_icon_fonts(&both);
    assert!(registered("Kept Icons") && registered("Dropped Icons"));
    // The next document has one of them: the other's family goes.
    let one = doc(vec![font("Kept Icons", vec![glyph("Home", 0xE000, 700.0)])]);
    sync_icon_fonts(&one);
    assert!(registered("Kept Icons"));
    assert!(!registered("Dropped Icons"), "a family no document carries is dropped");
}

#[test]
fn renaming_a_font_moves_the_family() {
    let _guard = test_guard();
    let mut doc = doc(vec![font("Old Icons", vec![glyph("Home", 0xE000, 700.0)])]);
    sync_icon_fonts(&doc);
    assert!(registered("Old Icons"));
    doc.icon_fonts[0].name = "New Icons".into();
    assert_eq!(sync_icon_fonts(&doc), 1, "a new family name is a new font");
    assert!(registered("New Icons"));
    assert!(!registered("Old Icons"));
}

#[test]
fn a_font_with_nothing_drawable_is_not_registered_and_not_retried() {
    let _guard = test_guard();
    // No icons at all.
    let empty = doc(vec![font("Empty Icons", Vec::new())]);
    assert_eq!(sync_icon_fonts(&empty), 1, "it is tried once");
    assert!(!registered("Empty Icons"));
    assert_eq!(sync_icon_fonts(&empty), 0, "and not again — the outlines are the same ones");
    // An icon whose stored outline holds nothing. (`icon.create` never stores one: a selection with
    // no extent is refused, so this is a document that was edited or written by hand.)
    let blank = doc(vec![font(
        "Blank Icons",
        vec![IconGlyph { id: "g-Home".into(), name: "Home".into(), codepoint: 0xE000, path: String::new(), node: None }],
    )]);
    sync_icon_fonts(&blank);
    assert!(!registered("Blank Icons"));
}

#[test]
fn forget_makes_the_next_sync_build_again() {
    let _guard = test_guard();
    let doc = doc(vec![font("Again Icons", vec![glyph("Home", 0xE000, 700.0)])]);
    assert_eq!(sync_icon_fonts(&doc), 1);
    assert_eq!(sync_icon_fonts(&doc), 0);
    forget(Some("Again Icons"));
    assert_eq!(sync_icon_fonts(&doc), 1, "forgetting it forces a rebuild");
    assert!(registered("Again Icons"));
    // Forgetting everything does the same.
    forget(None);
    assert_eq!(sync_icon_fonts(&doc), 1);
}
