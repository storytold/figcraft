//! `color()` and `color-mix()` colours (#1550): read where they map to a FigCraft colour, a note
//! and a grey (never a silent black) where they don't, and no panic on garbage.

use figcraft_color::{Color, ColorProfile, Paint};
use figcraft_doc::{Document, Node};

use super::css_color::resolve;
use super::import_svg_with_notes;

/// The layers of `body`'s import inside a 100×100 SVG (frame first), and its notes, into a
/// document in `profile`.
fn import_in(profile: ColorProfile, body: &str) -> (Vec<Node>, Vec<String>) {
    let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">{body}</svg>"#);
    let mut d = Document::new("t");
    d.color_profile = profile;
    let p = d.first_page().unwrap();
    let i = import_svg_with_notes(&mut d, svg.as_bytes(), p, 0.0, 0.0, "x").unwrap();
    let nodes = d.subtree(i.frame).into_iter().filter_map(|id| d.node(id).cloned()).collect();
    (nodes, i.notes)
}

/// The fill (colour with the paint's opacity) of a rectangle painted `fill`, and the notes.
fn fill_in(profile: ColorProfile, fill: &str) -> (Option<Color>, Vec<String>) {
    let (nodes, notes) = import_in(profile, &format!(r#"<rect width="10" height="10" style="fill: {fill}"/>"#));
    (nodes.get(1).and_then(|n| n.fills.first()).and_then(Paint::solid_color), notes)
}

fn fill(fill: &str) -> (Option<Color>, Vec<String>) {
    fill_in(ColorProfile::Srgb, fill)
}

fn rgba8(c: Option<Color>) -> Option<[u8; 4]> {
    c.map(Color::to_rgba8)
}

#[test]
fn color_srgb_reads_with_and_without_alpha() {
    let (c, notes) = fill("color(srgb 0.08 0.63 0.24)");
    assert_eq!(rgba8(c), Some([20, 161, 61, 255]));
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(rgba8(fill("color(srgb 0.08 0.63 0.24 / 0.6)").0), Some([20, 161, 61, 153]));
    assert_eq!(rgba8(fill("COLOR(SRGB 100% 0% none / 50%)").0), Some([255, 0, 0, 128]), "percentages, none and any case");
    // In an attribute too.
    let (nodes, _) = import_in(ColorProfile::Srgb, r#"<rect width="10" height="10" fill="color(srgb 0 0 1)"/>"#);
    assert_eq!(rgba8(nodes.get(1).and_then(|n| n.fills.first()).and_then(Paint::solid_color)), Some([0, 0, 255, 255]));
}

#[test]
fn display_p3_converts_to_the_documents_profile() {
    let p3 = Color::new(0.2, 0.6, 0.3, 1.0);
    let in_srgb = ColorProfile::DisplayP3.conversion_to(ColorProfile::Srgb).apply(p3);
    assert_eq!(rgba8(fill("color(display-p3 0.2 0.6 0.3)").0), Some(in_srgb.to_rgba8()));
    assert_ne!(in_srgb.to_rgba8(), p3.to_rgba8());
    // A Display P3 document keeps the values; an sRGB colour moves into its space.
    assert_eq!(rgba8(fill_in(ColorProfile::DisplayP3, "color(display-p3 0.2 0.6 0.3)").0), Some(p3.to_rgba8()));
    let srgb_in_p3 = ColorProfile::Srgb.conversion_to(ColorProfile::DisplayP3).apply(p3);
    assert_eq!(rgba8(fill_in(ColorProfile::DisplayP3, "color(srgb 0.2 0.6 0.3)").0), Some(srgb_in_p3.to_rgba8()));
}

#[test]
fn color_mix_in_srgb_oklab_and_oklch() {
    // The issue's colour: 60% of #14a03c, the rest white.
    let (c, notes) = fill("color-mix(in srgb, #14a03c 60%, white)");
    assert_eq!(rgba8(c), Some([114, 198, 138, 255]));
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(rgba8(fill("color-mix(in srgb, 40% white, #14a03c)").0), Some([114, 198, 138, 255]), "the percentage before the colour");
    // OKLab and OKLCH: white and black meet at lightness 0.5 (#636363).
    assert_eq!(rgba8(fill("color-mix(in oklab, white, black)").0), Some([99, 99, 99, 255]));
    assert_eq!(rgba8(fill("color-mix(in oklch, white, black)").0), Some([99, 99, 99, 255]));
    // A hue takes the shorter way round (through 0°, not 180°).
    let short = Color::parse_css("oklch(0.7 0.1 0)").unwrap();
    assert_eq!(rgba8(fill("color-mix(in oklch shorter hue, oklch(0.7 0.1 340), oklch(0.7 0.1 20))").0), Some(short.to_rgba8()));
    // Percentages under 100% lower the alpha; alphas mix premultiplied.
    assert_eq!(rgba8(fill("color-mix(in srgb, red 20%, blue 20%)").0), Some([128, 0, 128, 102]));
    assert_eq!(rgba8(fill("color-mix(in srgb, red, transparent)").0), Some([255, 0, 0, 128]));
    // Nested, with color() inside.
    assert_eq!(rgba8(fill("color-mix(in srgb, color-mix(in srgb, red, blue), color(srgb 0 1 0) 0%)").0), Some([128, 0, 128, 255]));
}

#[test]
fn strokes_stops_and_style_sheets_read_too() {
    let body = r#"<style>.a { fill: color(srgb 1 0 0); }</style>
        <linearGradient id="g"><stop offset="0" stop-color="color(srgb 0 1 0)"/><stop offset="1" stop-color="blue"/></linearGradient>
        <rect class="a" width="10" height="10" stroke="color-mix(in srgb, blue, red)"/>
        <rect x="20" width="10" height="10" fill="url(#g)"/>"#;
    let (nodes, notes) = import_in(ColorProfile::Srgb, body);
    assert!(notes.iter().all(|n| !n.contains("the colour `")), "{notes:?}");
    let rect = nodes.get(1).unwrap();
    assert_eq!(rgba8(rect.fills.first().and_then(Paint::solid_color)), Some([255, 0, 0, 255]));
    assert_eq!(rgba8(rect.strokes.first().and_then(Paint::solid_color)), Some([128, 0, 128, 255]));
    let stops = match nodes.get(2).and_then(|n| n.fills.first()).map(|p| &p.kind) {
        Some(figcraft_color::PaintKind::Gradient(g)) => g.stops.clone(),
        k => panic!("{k:?}"),
    };
    assert_eq!(stops.first().map(|s| s.color.to_rgba8()), Some([0, 255, 0, 255]));
    // Text and other attributes keep their words.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><text id="color(srgb 1 0 0)" fill="color(srgb 1 0 0)">color(srgb 1 0 0) <tspan>mycolor(1)</tspan></text></svg>"#;
    let (out, _) = resolve(svg.as_bytes(), ColorProfile::Srgb).unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        r##"<svg xmlns="http://www.w3.org/2000/svg"><text id="color(srgb 1 0 0)" fill="#FF0000">color(srgb 1 0 0) <tspan>mycolor(1)</tspan></text></svg>"##
    );
}

#[test]
fn what_isnt_read_gets_a_note_and_a_grey_not_black() {
    let grey = Some([128, 128, 128, 255]);
    for call in [
        "color(srgb a b c)",
        "color(srgb 1 0)",
        "color(from red srgb r g b)",
        "color-mix(in srgb, red)",
        "color-mix(srgb, red, blue)",
        "color-mix(in srgb, red 150%, blue)",
        "color-mix(in srgb, red 0%, blue 0%)",
        "color-mix(in srgb, currentcolor, blue)",
        "color(srgb 1 0 0",
    ] {
        let (c, notes) = fill(call);
        assert_eq!(rgba8(c), grey, "{call}");
        assert!(notes.iter().any(|n| n.contains("the colour `")), "{call}: {notes:?}");
    }
    // Spaces and mixes it doesn't know: close, with a note saying how.
    let (c, notes) = fill("color(rec2020 0 0 1)");
    assert_eq!(rgba8(c), Some([0, 0, 255, 255]));
    assert!(notes.iter().any(|n| n.contains("rec2020") && n.contains("sRGB")), "{notes:?}");
    let (c, notes) = fill("color-mix(in hsl, red, blue)");
    assert_eq!(rgba8(c), Some([128, 0, 128, 255]));
    assert!(notes.iter().any(|n| n.contains("mixed in sRGB")), "{notes:?}");
}

#[test]
fn garbage_and_deep_nesting_are_bounded() {
    // Nested deeper than the limit: a note, no stack overflow.
    let deep = (0..50).fold("red".to_string(), |acc, _| format!("color-mix(in srgb, {acc}, blue)"));
    let (c, notes) = fill(&deep);
    assert!(c.is_some() && notes.iter().any(|n| n.contains("nested")), "{notes:?}");
    // Many calls, unclosed ones, odd bytes: each attribute is read, nothing panics.
    let many = "color(srgb 0 0 1) ".repeat(5000);
    assert!(
        resolve(format!(r#"<svg xmlns="http://www.w3.org/2000/svg"><rect style="fill: {many}"/></svg>"#).as_bytes(), ColorProfile::Srgb).is_some()
    );
    let mut seed = 7u64;
    let pieces = ["color(", "color-mix(", "in ", "srgb", "oklch", ")", "(", ",", " ", "%", "/", "50", "1e40", "NaN", "é", "#fff", "none"];
    for _ in 0..500 {
        let junk: String = (0..12)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                pieces[(seed % pieces.len() as u64) as usize]
            })
            .collect();
        let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="1" height="1" fill="{junk}" style="stroke: {junk}"/></svg>"#);
        let _ = resolve(svg.as_bytes(), ColorProfile::Srgb);
        let mut d = Document::new("t");
        let p = d.first_page().unwrap();
        let _ = import_svg_with_notes(&mut d, svg.as_bytes(), p, 0.0, 0.0, "x");
    }
    // Nothing to do: the document comes back as it was.
    assert!(resolve(br#"<svg xmlns="http://www.w3.org/2000/svg"><rect fill="red"/></svg>"#, ColorProfile::Srgb).is_none());
    assert!(resolve(b"\xff\xfecolor(", ColorProfile::Srgb).is_none());
}
