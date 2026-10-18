//! Tests for the Icons panel. The render-heavy `show` path needs a full `FigApp` and an egui
//! context; here we cover the pure data path the panel reads from a document, which is where the
//! icon fonts live and what every button in the panel acts on.

use figcraft_doc::{Document, IconFont, IconGlyph};

use super::{Key, Thumbs, fonts_of, listed, refresh_thumbs};

fn doc_with(fonts: Vec<IconFont>) -> Document {
    let mut doc = Document::new("Icons test");
    doc.icon_fonts = fonts;
    doc
}

#[test]
fn the_panel_lists_the_documents_icon_fonts() {
    let doc = doc_with(vec![
        IconFont { id: "star".into(), name: "Stars".into(), glyphs: Vec::new() },
        IconFont { id: "arrow".into(), name: "Arrows".into(), glyphs: Vec::new() },
    ]);
    let fonts = fonts_of(&doc);
    assert_eq!(fonts.len(), 2);
    assert_eq!(fonts[0].id, "star");
    assert_eq!(fonts[1].id, "arrow");
}

#[test]
fn an_empty_document_lists_no_icon_fonts() {
    assert!(fonts_of(&Document::new("empty")).is_empty());
}

#[test]
fn the_panel_keeps_each_fonts_icons() {
    let font = IconFont {
        id: "ui".into(),
        name: "UI".into(),
        glyphs: vec![
            IconGlyph { id: "g-home".into(), name: "home".into(), codepoint: 0xE000, path: "M0,0 L1,0 L1,1 L0,1 Z".into(), node: None },
            IconGlyph { id: "g-gear".into(), name: "gear".into(), codepoint: 0xE001, path: "M2,2 L3,2 L3,3 L2,3 Z".into(), node: None },
        ],
    };
    let fonts = fonts_of(&doc_with(vec![font]));
    assert_eq!(fonts[0].glyphs.len(), 2);
    assert_eq!(fonts[0].glyphs[0].name, "home");
    assert_eq!(fonts[0].glyphs[1].codepoint, 0xE001);
}

fn two_fonts() -> Vec<IconFont> {
    vec![
        IconFont {
            id: "ui".into(),
            name: "UI".into(),
            glyphs: vec![
                IconGlyph { id: "g-home".into(), name: "home".into(), codepoint: 0xE000, path: String::new(), node: None },
                IconGlyph { id: "g-gear".into(), name: "gear".into(), codepoint: 0xE001, path: String::new(), node: None },
            ],
        },
        IconFont {
            id: "brand".into(),
            name: "Brand".into(),
            glyphs: vec![IconGlyph { id: "g-logo".into(), name: "Logo".into(), codepoint: 0xE100, path: String::new(), node: None }],
        },
    ]
}

/// With no query, every icon of every font is listed, and `whole` says the list is the font's own
/// order (which is the only order a drop can be placed into).
#[test]
fn searching_nothing_lists_every_icon() {
    let fonts = two_fonts();
    let rows = listed(&fonts, "");
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.whole));
    assert_eq!(rows[0].glyphs.len(), 2);
    assert_eq!(rows[1].glyphs.len(), 1);
}

/// A query keeps only the icons whose name matches, and drops fonts left with nothing. `whole` is
/// false, so a reorder drop is ignored while the list is filtered (the places shown aren't the
/// font's own).
#[test]
fn searching_by_name_keeps_only_the_matches() {
    let fonts = two_fonts();
    let rows = listed(&fonts, "ge");
    assert_eq!(rows.len(), 1, "only UI has a match");
    assert_eq!(rows[0].font.id, "ui");
    assert_eq!(rows[0].glyphs.len(), 1);
    assert_eq!(rows[0].glyphs[0].name, "gear");
    assert!(!rows[0].whole);
}

/// The code point is searchable, in both "e001" and "u+e001" writing.
#[test]
fn searching_by_code_point_finds_the_icon() {
    let fonts = two_fonts();
    for q in ["e001", "u+e001", "U+E001"] {
        let rows = listed(&fonts, &q.to_lowercase());
        assert_eq!(rows.len(), 1, "{q}");
        assert_eq!(rows[0].glyphs[0].name, "gear", "{q}");
    }
}

/// Case doesn't matter: icon names are lowercased and the query is lowercased before both sides
/// are compared, so "LOGO" finds "Logo".
#[test]
fn searching_ignores_case() {
    let fonts = two_fonts();
    let rows = listed(&fonts, "logo");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].glyphs[0].name, "Logo");
}

/// A query nothing matches lists nothing at all (not even fonts).
#[test]
fn searching_for_nothing_that_exists_lists_no_fonts() {
    let fonts = two_fonts();
    assert!(listed(&fonts, "zzz").is_empty());
}

/// A colour that isn't the theme's, so a thumb's key is predictable in a test.
const INK: [u8; 4] = [0x1E, 0x1E, 0x1E, 0xFF];

/// A square outline, so the rendered preview has something in it.
const SQUARE: &str = "M0,0 L700,0 L700,700 L0,700 Z";

fn key(name: &str) -> Key {
    ("ui".to_string(), name.to_string(), 0, INK)
}

/// The regression for the flickering previews: a thumb built on one frame must still be there on
/// the next. It used to be dropped the frame after it was built (only the *missing* keys were kept
/// alive) and rebuilt the frame after that, so every preview blinked forever.
#[test]
fn a_built_preview_survives_the_next_frame() {
    let ctx = egui::Context::default();
    let mut thumbs = Thumbs::default();
    // Frame 1: the row is drawn, no texture yet, so it asks for one.
    let live: std::collections::HashSet<Key> = [key("home")].into_iter().collect();
    let missing = vec![(key("home"), SQUARE.to_string())];
    refresh_thumbs(&ctx, &mut thumbs, &live, &missing);
    assert_eq!(thumbs.0.len(), 1, "the preview was built");
    assert!(thumbs.0[&key("home")].is_some(), "and it holds a texture");

    // Frame 2: the same row, now with a texture, so nothing is missing.
    refresh_thumbs(&ctx, &mut thumbs, &live, &[]);
    assert_eq!(thumbs.0.len(), 1, "the preview is still there (it used to be dropped here)");
}

/// A preview that is no longer on screen is still dropped, so the cache can't grow forever.
#[test]
fn a_preview_that_left_the_screen_is_dropped() {
    let ctx = egui::Context::default();
    let mut thumbs = Thumbs::default();
    let both: std::collections::HashSet<Key> = [key("home"), key("gear")].into_iter().collect();
    let missing = vec![(key("home"), SQUARE.to_string()), (key("gear"), SQUARE.to_string())];
    refresh_thumbs(&ctx, &mut thumbs, &both, &missing);
    assert_eq!(thumbs.0.len(), 2);

    // "gear" is gone (its icon was deleted, or its font folded away).
    let only_home: std::collections::HashSet<Key> = [key("home")].into_iter().collect();
    refresh_thumbs(&ctx, &mut thumbs, &only_home, &[]);
    assert_eq!(thumbs.0.len(), 1);
    assert!(thumbs.0.contains_key(&key("home")));
}

/// An outline that draws nothing is remembered as such, so it isn't re-rendered every frame.
#[test]
fn an_outline_with_nothing_to_draw_is_not_retried() {
    let ctx = egui::Context::default();
    let mut thumbs = Thumbs::default();
    let live: std::collections::HashSet<Key> = [key("void")].into_iter().collect();
    let missing = vec![(key("void"), "M0,0 Z".to_string())];
    refresh_thumbs(&ctx, &mut thumbs, &live, &missing);
    assert!(thumbs.0.contains_key(&key("void")), "the attempt is remembered");
    assert!(thumbs.0[&key("void")].is_none(), "cached as undrawable");

    // The row still offers it as missing, but the cache says don't bother.
    refresh_thumbs(&ctx, &mut thumbs, &live, &missing);
    assert_eq!(thumbs.0.len(), 1);
    assert!(thumbs.0[&key("void")].is_none());
}
