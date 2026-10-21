//! Icon fonts: code point allocation, name de-duplication and persistence.

use crate::{Document, ICON_PUA_END, ICON_PUA_START, IconFont, IconGlyph};

fn glyph(name: &str, codepoint: u32) -> IconGlyph {
    IconGlyph { id: format!("g-{name}"), name: name.into(), codepoint, path: "M0 0 L10 0 L10 10 Z".into(), node: None }
}

fn font(id: &str, name: &str, glyphs: Vec<IconGlyph>) -> IconFont {
    IconFont { id: id.into(), name: name.into(), glyphs }
}

#[test]
fn code_points_are_handed_out_from_the_private_use_area_in_order() {
    let mut f = font("iconfont-1", "Icons", Vec::new());
    assert_eq!(f.next_codepoint(), ICON_PUA_START);
    f.glyphs.push(glyph("Home", ICON_PUA_START));
    assert_eq!(f.next_codepoint(), ICON_PUA_START + 1);
    // A gap is filled before the next free code point is used.
    f.glyphs.push(glyph("Star", ICON_PUA_START + 5));
    assert_eq!(f.next_codepoint(), ICON_PUA_START + 1);
    // Out-of-order glyphs don't matter.
    f.glyphs.push(glyph("Bell", ICON_PUA_START + 2));
    assert_eq!(f.next_codepoint(), ICON_PUA_START + 1);
}

#[test]
fn the_bmp_area_fills_before_plane_15_is_used() {
    let full: Vec<IconGlyph> = (ICON_PUA_START..=ICON_PUA_END).map(|c| glyph(&format!("g{c}"), c)).collect();
    let f = font("iconfont-1", "Icons", full);
    assert_eq!(f.next_codepoint(), crate::ICON_PUA_SUPPLEMENTARY_START);
}

#[test]
fn icon_names_and_font_names_do_not_repeat() {
    let f = font("iconfont-1", "Icons", vec![glyph("Home", ICON_PUA_START), glyph("Home 2", ICON_PUA_START + 1)]);
    assert_eq!(f.fresh_glyph_name("Home"), "Home 3");
    assert_eq!(f.fresh_glyph_name("Star"), "Star");
    // A blank name falls back to "Icon" rather than staying empty.
    assert_eq!(f.fresh_glyph_name("   "), "Icon");

    let mut d = Document::new("a");
    d.icon_fonts.push(font("iconfont-1", "Icons", Vec::new()));
    assert_eq!(d.fresh_icon_font_name("Icons"), "Icons 2");
    assert_eq!(d.fresh_icon_font_name("Symbols"), "Symbols");
    assert_eq!(d.fresh_icon_font_name(""), "Icons 2");
}

#[test]
fn fonts_and_glyphs_are_found_by_id_or_name() {
    let mut d = Document::new("a");
    d.icon_fonts.push(font("iconfont-1", "Icons", vec![glyph("Home", ICON_PUA_START)]));
    d.icon_fonts.push(font("iconfont-2", "Symbols", Vec::new()));
    assert_eq!(d.icon_font_index("iconfont-2"), Some(1));
    assert_eq!(d.icon_font_index("Symbols"), Some(1));
    assert_eq!(d.icon_font_index("nope"), None);
    assert_eq!(d.icon_font("Icons").map(|f| f.id.as_str()), Some("iconfont-1"));
    assert!(d.icon_font_mut("Symbols").is_some());
    assert_eq!(d.fresh_icon_font_id(), "iconfont-3");
    let f = d.icon_font("Icons").unwrap();
    assert_eq!(f.glyph("Home").map(|g| g.codepoint), Some(ICON_PUA_START));
    assert_eq!(f.glyph_at(ICON_PUA_START).map(|g| g.name.as_str()), Some("Home"));
    assert_eq!(f.glyph("Star"), None);
}

#[test]
fn icon_fonts_survive_a_serialisation_round_trip() {
    let mut d = Document::new("a");
    d.icon_fonts.push(font(
        "iconfont-1",
        "Icons",
        vec![IconGlyph {
            id: "g-Home".into(),
            name: "Home".into(),
            codepoint: ICON_PUA_START,
            path: "M0 0 L10 0 L10 10 Z".into(),
            node: Some(crate::NodeId(7)),
        }],
    ));
    let json = serde_json::to_string(&d).unwrap();
    let back: Document = serde_json::from_str(&json).unwrap();
    assert_eq!(back.icon_fonts(), d.icon_fonts());
}

#[test]
fn a_document_without_icon_fonts_omits_them_from_json() {
    let json = serde_json::to_string(&Document::new("a")).unwrap();
    assert!(!json.contains("icon_fonts"), "{json}");
    // And an old file without the field still loads.
    let back: Document = serde_json::from_str(&json).unwrap();
    assert!(back.icon_fonts().is_empty());
}
