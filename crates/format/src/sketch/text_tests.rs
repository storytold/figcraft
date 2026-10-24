//! Sketch text layers → text with styled runs; font PostScript names → family and style.

use figcraft_doc::color::PaintKind;
use figcraft_doc::{Document, LetterSpacing, LineHeight, NodeKind, TextAlignH, TextAlignV, TextAutoResize, TextCase, TextData, TextDecoration};
use serde_json::{Value, json};

use super::fixture::*;
use super::import;
use super::text::font_name;

const BLACK: [f64; 4] = [0.0, 0.0, 0.0, 1.0];
const RED: [f64; 4] = [1.0, 0.0, 0.0, 1.0];

fn open(layers: Vec<Value>) -> (Document, Vec<String>) {
    let z = sketch_zip("50.2", 103, document(&["P"]), &[page("P", "Page", vec![artboard("A", "Board", 0.0, 0.0, 400.0, 400.0, layers)])], &[]);
    import(&z, "f").unwrap()
}

fn data<'d>(doc: &'d Document, name: &str) -> &'d TextData {
    match &doc.nodes().find(|n| n.name == name).unwrap().kind {
        NodeKind::Text(td) => td,
        other => panic!("{name}: {other:?}"),
    }
}

fn red(fills: &Option<Vec<figcraft_doc::color::Paint>>) -> bool {
    fills.as_ref().is_some_and(|f| f.iter().any(|p| matches!(p.kind, PaintKind::Solid { color } if color.r == 1.0 && color.g == 0.0)))
}

#[test]
fn two_runs() {
    let t = text("Hi", 0.0, 0.0, 200.0, 20.0, "Hello world", &[(0, 5, "HelveticaNeue-Bold", 16.0, RED), (5, 6, "HelveticaNeue", 12.0, BLACK)]);
    let (doc, _) = open(vec![t]);
    let td = data(&doc, "Hi");
    assert_eq!(td.characters, "Hello world");
    let style_at = |i: usize| td.runs.iter().find(|r| r.start <= i && i < r.end).map_or(&td.style, |r| &r.style);
    let (a, b) = (style_at(0), style_at(7));
    assert_eq!((a.font_family.as_str(), a.font_style.as_str(), a.font_size), ("Helvetica Neue", "Bold", 16.0));
    assert_eq!((b.font_family.as_str(), b.font_style.as_str(), b.font_size), ("Helvetica Neue", "Regular", 12.0));
    let node = doc.nodes().find(|n| n.name == "Hi").unwrap();
    assert!(red(&a.fills) || (red(&Some(node.fills.clone())) && !red(&b.fills)));
}

#[test]
fn utf16_ranges() {
    // "a😀b": the emoji takes two UTF-16 units.
    let t = text(
        "Emoji",
        0.0,
        0.0,
        200.0,
        20.0,
        "a😀b",
        &[(0, 1, "Helvetica", 10.0, BLACK), (1, 2, "Helvetica", 20.0, BLACK), (3, 1, "Helvetica", 30.0, BLACK)],
    );
    let (doc, _) = open(vec![t]);
    let td = data(&doc, "Emoji");
    let size_at = |i: usize| td.runs.iter().find(|r| r.start <= i && i < r.end).map_or(td.style.font_size, |r| r.style.font_size);
    assert_eq!((size_at(0), size_at(1), size_at(2)), (10.0, 20.0, 30.0));
    // A range splitting the emoji and one past the end: no panic, the text stays whole.
    let t = text(
        "Split",
        0.0,
        50.0,
        200.0,
        20.0,
        "a😀b",
        &[(0, 2, "Helvetica", 10.0, BLACK), (2, 99, "Helvetica", 20.0, BLACK), (500, 3, "Helvetica", 5.0, BLACK)],
    );
    let (doc, _) = open(vec![t]);
    assert_eq!(data(&doc, "Split").characters, "a😀b");
}

#[test]
fn paragraph_and_spacing() {
    let mut t = text("Para", 0.0, 0.0, 200.0, 40.0, "Centered", &[(0, 8, "Helvetica", 14.0, BLACK)]);
    let a = &mut t["attributedString"]["attributes"][0]["attributes"];
    a["paragraphStyle"] =
        json!({"_class": "paragraphStyle", "alignment": 2, "maximumLineHeight": 24, "minimumLineHeight": 24, "paragraphSpacing": 8});
    a["kerning"] = json!(0.5);
    let (doc, _) = open(vec![t]);
    let td = data(&doc, "Para");
    assert_eq!(td.align_h, TextAlignH::Center);
    assert_eq!(td.style.line_height, LineHeight::Pixels(24.0));
    assert_eq!(td.paragraph_spacing, 8.0);
    assert_eq!(td.style.letter_spacing, LetterSpacing::Pixels(0.5));
    for (code, want) in [(0, TextAlignH::Left), (1, TextAlignH::Right), (3, TextAlignH::Justified), (4, TextAlignH::Left)] {
        let mut t = text("A", 0.0, 0.0, 100.0, 20.0, "x", &[(0, 1, "Helvetica", 14.0, BLACK)]);
        t["attributedString"]["attributes"][0]["attributes"]["paragraphStyle"]["alignment"] = json!(code);
        let (doc, _) = open(vec![t]);
        assert_eq!(data(&doc, "A").align_h, want, "alignment {code}");
    }
}

#[test]
fn behaviour_and_vertical_alignment() {
    let cases = [(0, TextAutoResize::WidthAndHeight), (1, TextAutoResize::Height), (2, TextAutoResize::None)];
    for (b, want) in cases {
        let mut t = text("T", 0.0, 0.0, 100.0, 20.0, "x", &[(0, 1, "Helvetica", 14.0, BLACK)]);
        t["textBehaviour"] = json!(b);
        let (doc, _) = open(vec![t]);
        assert_eq!(data(&doc, "T").auto_resize, want, "behaviour {b}");
    }
    for (v, want) in [(0, TextAlignV::Top), (1, TextAlignV::Middle), (2, TextAlignV::Bottom)] {
        let mut t = text("V", 0.0, 0.0, 100.0, 60.0, "x", &[(0, 1, "Helvetica", 14.0, BLACK)]);
        t["textBehaviour"] = json!(2);
        t["style"]["textStyle"] = json!({"_class": "textStyle", "verticalAlignment": v, "encodedAttributes": {}});
        let (doc, _) = open(vec![t]);
        assert_eq!(data(&doc, "V").align_v, want, "vertical {v}");
    }
}

#[test]
fn decorations_and_case() {
    let mut t = text(
        "Deco",
        0.0,
        0.0,
        200.0,
        20.0,
        "abcdef",
        &[(0, 2, "Helvetica", 14.0, BLACK), (2, 2, "Helvetica", 14.0, BLACK), (4, 2, "Helvetica", 14.0, BLACK)],
    );
    t["attributedString"]["attributes"][0]["attributes"]["underlineStyle"] = json!(1);
    t["attributedString"]["attributes"][1]["attributes"]["strikethroughStyle"] = json!(1);
    t["attributedString"]["attributes"][2]["attributes"]["MSAttributedStringTextTransformAttribute"] = json!(1);
    let (doc, _) = open(vec![t]);
    let td = data(&doc, "Deco");
    let at = |i: usize| td.runs.iter().find(|r| r.start <= i && i < r.end).map_or(&td.style, |r| &r.style);
    assert_eq!(at(0).decoration, TextDecoration::Underline);
    assert_eq!(at(2).decoration, TextDecoration::Strikethrough);
    assert_eq!(at(4).case, TextCase::Upper);
}

#[test]
fn text_layer_fill_wins_over_run_colour() {
    let t = styled(text("Filled", 0.0, 0.0, 100.0, 20.0, "x", &[(0, 1, "Helvetica", 14.0, BLACK)]), "fills", json!([solid_fill(1.0, 0.0, 0.0, 1.0)]));
    let (doc, _) = open(vec![t]);
    let n = doc.nodes().find(|n| n.name == "Filled").unwrap();
    assert!(red(&Some(n.fills.clone())));
}

#[test]
fn font_names() {
    let cases = [
        ("HelveticaNeue-Bold", "Helvetica Neue", "Bold"),
        ("HelveticaNeue", "Helvetica Neue", "Regular"),
        ("HelveticaNeue-Medium", "Helvetica Neue", "Medium"),
        ("PingFangSC-Semibold", "PingFang SC", "Semi Bold"),
        ("PingFangTC-Regular", "PingFang TC", "Regular"),
        (".SFNSText", "SF Pro Text", "Regular"),
        (".SFNSDisplay-Bold", "SF Pro Display", "Bold"),
        ("SanFranciscoText-Semibold", "SF Pro Text", "Semi Bold"),
        ("Impact", "Impact", "Regular"),
        ("SFUIText-Semibold", "SF UI Text", "Semi Bold"),
        ("SFUIDisplay-Regular", "SF UI Display", "Regular"),
        ("SFCompactDisplay-Semibold", "SF Compact Display", "Semi Bold"),
        ("SFProText-Bold", "SF Pro Text", "Bold"),
        ("IBMPlexSans-MediumItalic", "IBM Plex Sans", "Medium Italic"),
        ("Roboto-Italic", "Roboto", "Italic"),
        ("", "Helvetica", "Regular"),
    ];
    for (ps, family, style) in cases {
        assert_eq!(font_name(ps), (family.to_string(), style.to_string()), "{ps}");
    }
}

#[test]
fn long_and_odd_text_is_safe() {
    let long = "x".repeat(10_000);
    let t = text("Long", 0.0, 0.0, 100.0, 20.0, &long, &[(0, usize::MAX >> 1, "Helvetica", 14.0, BLACK)]);
    let mut odd = text("Odd", 0.0, 0.0, 100.0, 20.0, "abc", &[(0, 3, "Helvetica", -5.0, BLACK)]);
    odd["attributedString"]["attributes"][0]["attributes"]["MSAttributedStringFontAttribute"] = json!("junk");
    let mut empty = text("Empty", 0.0, 0.0, 100.0, 20.0, "", &[]);
    empty["attributedString"]["string"] = json!(5);
    let (doc, _) = open(vec![t, odd, empty]);
    assert_eq!(data(&doc, "Long").characters.len(), 10_000);
    assert!(data(&doc, "Odd").style.font_size > 0.0);
}

#[test]
fn embedded_fonts_are_reported() {
    let mut d = document(&["P"]);
    d["fontReferences"] = json!([
        {"_class": "fontReference", "fontFamilyName": "IBM Plex Sans", "fontFileName": "IBMPlexSans-Regular.otf", "fontData": {"_ref": "fonts/abc"}},
        {"_class": "fontReference", "fontFamilyName": "IBM Plex Sans", "fontFileName": "IBMPlexSans-Bold.otf", "fontData": {"_ref": "fonts/def"}},
        {"_class": "fontReference", "fontFamilyName": "IBM Plex Mono", "fontData": {"_ref": "fonts/ghi"}},
    ]);
    let z = sketch_zip("83.2", 139, d, &[page("P", "Page", vec![])], &[("fonts/abc", b"x")]);
    let (_, warnings) = import(&z, "f").unwrap();
    let w = warnings.iter().find(|w| w.contains("embedded")).expect("a note about embedded fonts");
    assert!(w.contains("IBM Plex Mono") && w.contains("IBM Plex Sans"), "{w}");
}

/// Where Sketch puts a line's extra height: above the text (`lineSpacingBehaviour` 0), below it
/// (1) or around it (2, like FigCraft); the text layer moves by the difference.
#[test]
fn line_spacing_behaviour_moves_text() {
    let mk = |name: &str, beh: i64| {
        let mut t = text(name, 0.0, 100.0, 100.0, 30.0, "Hg", &[(0, 2, "Helvetica", 14.0, BLACK)]);
        t["lineSpacingBehaviour"] = json!(beh);
        let a = &mut t["attributedString"]["attributes"][0]["attributes"]["paragraphStyle"];
        a["maximumLineHeight"] = json!(30);
        a["minimumLineHeight"] = json!(30);
        t
    };
    let (doc, _) = open(vec![mk("B0", 0), mk("B1", 1), mk("B2", 2)]);
    let y = |n: &str| doc.world_transform(doc.nodes().find(|x| x.name == n).unwrap().id).as_coeffs()[5];
    // Extra height: 30 − 14 × 1.2 = 13.2; half of it either way.
    assert!((y("B2") - 100.0).abs() < 1e-6, "{}", y("B2"));
    assert!((y("B0") - 106.6).abs() < 1e-6, "{}", y("B0"));
    assert!((y("B1") - 93.4).abs() < 1e-6, "{}", y("B1"));
}

/// A variable font's axes (Sketch keys them by the tag as a number: 2003265652 is `wght`).
#[test]
fn font_variations() {
    let mut t = text("Wide", 0.0, 0.0, 200.0, 50.0, "Orbit", &[(0, 5, "Skia", 40.0, BLACK)]);
    t["attributedString"]["attributes"][0]["attributes"]["MSAttributedStringFontAttribute"]["attributes"]["variation"] =
        json!({"2003265652": 3, "2003072104": 0.65, "junk": 1, "1": "x"});
    let (doc, _) = open(vec![t]);
    let v = &data(&doc, "Wide").style.variations;
    assert_eq!(v.get("wght"), Some(&3.0));
    assert_eq!(v.get("wdth"), Some(&0.65));
    assert_eq!(v.len(), 2, "{v:?}");
}

/// Font references without font data are only the fonts the document uses: no note.
#[test]
fn font_references_without_data_are_no_embedded_fonts() {
    let mut d = document(&["P"]);
    d["fontReferences"] =
        json!([{"_class": "fontReference", "fontFamilyName": "Skia", "fontFileName": "Skia.ttf", "postscriptNames": ["Skia-Regular"]}]);
    let z = sketch_zip("2026.3.1", 196, d, &[page("P", "Page", vec![])], &[]);
    let (_, warnings) = import(&z, "f").unwrap();
    assert!(!warnings.iter().any(|w| w.contains("embedded")), "{warnings:?}");
}

/// With a font lookup, a PostScript name gives the installed family and style it belongs to (the
/// name alone can't always tell: "CourierNewPSMT" is Courier New Regular); without one, or for a
/// font that isn't installed, the family is read from the name.
#[test]
fn postscript_names_find_installed_fonts() {
    let mk = |name: &str, ps: &str| text(name, 0.0, 0.0, 100.0, 20.0, "Hi", &[(0, 2, ps, 14.0, BLACK)]);
    let z = sketch_zip("83.2", 139, document(&["P"]), &[page("P", "Page", vec![mk("A", "CourierNewPSMT"), mk("B", "BrandSans-Bold")])], &[]);
    let lookup: crate::sketch::FontLookup = Box::new(|ps: &str| (ps == "CourierNewPSMT").then(|| ("Courier New".to_string(), "Regular".to_string())));
    let (doc, _) = crate::sketch::import_with_fonts(&z, "f", Some(lookup)).unwrap();
    assert_eq!((data(&doc, "A").style.font_family.as_str(), data(&doc, "A").style.font_style.as_str()), ("Courier New", "Regular"));
    assert_eq!(data(&doc, "B").style.font_family, "Brand Sans");
    let (doc, _) = import(&z, "f").unwrap();
    assert_eq!(data(&doc, "A").style.font_family, "Courier New PSMT");
}

/// A run without kerning, a fixed line height or font axes has none: it doesn't take the first
/// run's (the text's base style) as its own.
#[test]
fn later_runs_dont_inherit_the_first_runs_settings() {
    let mut t = text("Mixed", 0.0, 0.0, 200.0, 40.0, "AaBb", &[(0, 2, "Skia", 20.0, BLACK), (2, 2, "Helvetica", 14.0, BLACK)]);
    let first = &mut t["attributedString"]["attributes"][0]["attributes"];
    first["kerning"] = json!(2);
    first["paragraphStyle"]["minimumLineHeight"] = json!(24);
    first["paragraphStyle"]["maximumLineHeight"] = json!(24);
    first["MSAttributedStringFontAttribute"]["attributes"]["variation"] = json!({"2003265652": 3});
    let (doc, _) = open(vec![t]);
    let td = data(&doc, "Mixed");
    let at = |i: usize| td.runs.iter().find(|r| r.start <= i && i < r.end).map_or(&td.style, |r| &r.style);
    assert_eq!(at(0).letter_spacing.resolve(20.0), 2.0);
    let second = at(2);
    assert_eq!(second.font_family, "Helvetica");
    assert_eq!(second.letter_spacing.resolve(14.0), 0.0, "{second:?}");
    assert_eq!(second.line_height, figcraft_doc::LineHeight::Auto, "{second:?}");
    assert!(second.variations.is_empty(), "{second:?}");
}
