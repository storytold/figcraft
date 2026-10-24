//! Which installed family an SVG `font-family` list picks.

use usvg::FontFamily;

use super::text::{Fonts, family};

/// macOS-like: `system-ui` and the other system-font names draw in "System Font".
fn mac_resolve(f: &str) -> String {
    match f.trim().to_lowercase().as_str() {
        "system-ui" | "-apple-system" | "sf pro" => "System Font".into(),
        _ => f.into(),
    }
}

fn no_alias(f: &str) -> String {
    f.into()
}

fn fonts(names: &[&str], resolve: fn(&str) -> String) -> Fonts {
    Fonts::new(names.iter().map(|s| (*s).to_string()), resolve)
}

fn named(n: &str) -> FontFamily {
    FontFamily::Named(n.into())
}

/// `system-ui` keeps its own name when the system font is installed, ahead of the sans stand-ins.
#[test]
fn system_ui_wins_over_the_generic_fallback_when_the_system_font_is_installed() {
    let mac = fonts(&["System Font", "Inter", "Helvetica"], mac_resolve);
    assert_eq!(family(&[named("system-ui")], &mac), "system-ui");
    assert_eq!(family(&[named("system-ui"), FontFamily::SansSerif], &mac), "system-ui");
    assert_eq!(family(&[named("-apple-system"), named("Helvetica")], &mac), "-apple-system");
    // The other generic families still pick their stand-ins.
    assert_eq!(family(&[FontFamily::SansSerif], &mac), "Inter");
    assert_eq!(family(&[FontFamily::Monospace], &fonts(&["Menlo", "System Font"], mac_resolve)), "Menlo");
}

/// Without the system font (Linux, Windows, the web), `system-ui` falls back to a sans family as
/// before.
#[test]
fn system_ui_falls_back_to_a_sans_family_without_the_system_font() {
    let other = fonts(&["Inter", "Helvetica"], no_alias);
    assert_eq!(family(&[named("system-ui")], &other), "Inter");
    assert_eq!(family(&[named("system-ui")], &fonts(&["DejaVu Sans"], no_alias)), "DejaVu Sans");
    // An alias that resolves to nothing installed is no better than a missing family.
    assert_eq!(family(&[named("system-ui")], &fonts(&["Helvetica"], mac_resolve)), "Helvetica");
    assert_eq!(family(&[named("-apple-system"), FontFamily::SansSerif], &other), "Inter");
}

/// On a Mac with its system font, an imported `system-ui` text layer keeps that name and draws in
/// "System Font" (not Inter) at the weight the SVG asks for.
#[cfg(target_os = "macos")]
#[test]
fn system_ui_text_imports_in_the_macos_system_font() {
    use figcraft_doc::{Document, NodeKind};
    figcraft_text::scan_system_fonts();
    if !figcraft_text::has_family("System Font") {
        eprintln!("skipped: no system fonts on this machine");
        return;
    }
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="60"><text x="10" y="40" font-family="system-ui" font-size="24" font-weight="700">Hello</text></svg>"#;
    let mut d = Document::new("t");
    let p = d.first_page().unwrap();
    let f = super::import_svg_with_notes(&mut d, svg, p, 0.0, 0.0, "x").unwrap().frame;
    let t = d
        .node(f)
        .unwrap()
        .children
        .iter()
        .find_map(|c| match &d.node(*c).unwrap().kind {
            NodeKind::Text(t) => Some(t.clone()),
            _ => None,
        })
        .expect("a text layer");
    assert_eq!(t.style.font_family, "system-ui");
    assert_eq!(t.style.font_style, "Bold");
    let face = figcraft_text::fontdb::find_face(&t.style.font_family, &t.style.font_style).unwrap();
    assert!(face.family.eq_ignore_ascii_case("System Font"), "drew in {}", face.family);
    // The system font is variable: Bold is a named instance or a `wght` of 700.
    let w = face.axis("wght").map_or(f64::from(face.weight), |a| a.clamp(700.0));
    assert!(face.instance("Bold").is_some() || w >= 600.0, "Bold is {} at {w}", face.style);
    eprintln!("system-ui Bold → {} (wght {w}, Bold instance: {})", face.family, face.instance("Bold").is_some());
}
