use super::*;
use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, Effect, NodeKind};

#[test]
fn export_basic_shapes() {
    let mut d = Document::new("t");
    let p = d.first_page().unwrap();
    let f = d.create(p, NodeKind::Frame, "Card", 0.0, 0.0, 200.0, 100.0).unwrap();
    let r = d.create(f, NodeKind::Rectangle, "R", 10.0, 10.0, 50.0, 50.0).unwrap();
    d.get_mut(r).unwrap().fills = vec![Paint::solid(Color::from_hex("FF0000").unwrap())];
    d.get_mut(r).unwrap().effects.push(Effect::drop_shadow());
    let svg = export_svg(&d, &[f], &SvgOptions::default()).unwrap();
    assert!(svg.starts_with("<svg width=\"200\" height=\"100\""));
    assert!(svg.contains("#FF0000"));
    assert!(svg.contains("<filter"));
    assert!(svg.contains("clip-path"));
    // Round-trip through import.
    let mut d2 = Document::new("t2");
    let p2 = d2.first_page().unwrap();
    let frame = import_svg(&mut d2, svg.as_bytes(), p2, 0.0, 0.0, "Imported").unwrap();
    assert!(!d2.subtree(frame).is_empty());
    assert!(d2.validate().is_empty());
}

#[test]
fn import_rejects_garbage() {
    let mut d = Document::new("t");
    let p = d.first_page().unwrap();
    assert!(import_svg(&mut d, b"not svg", p, 0.0, 0.0, "x").is_err());
}

#[test]
fn import_gradient_and_groups() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50">
      <defs><linearGradient id="g"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient></defs>
      <g id="grp"><rect x="10" y="10" width="30" height="30" fill="url(#g)" stroke="#000" stroke-width="2"/></g>
      <circle cx="70" cy="25" r="20" fill="#0f0" fill-opacity="0.5"/></svg>"##;
    let mut d = Document::new("t");
    let p = d.first_page().unwrap();
    let f = import_svg(&mut d, svg, p, 0.0, 0.0, "x").unwrap();
    let kinds: Vec<&str> = d.subtree(f).iter().map(|i| d.node(*i).unwrap().kind.type_name()).collect();
    assert!(kinds.contains(&"GROUP"));
    assert!(kinds.contains(&"RECTANGLE") && kinds.contains(&"ELLIPSE"), "{kinds:?}");
}

/// #325: a frame with children inside a clipping frame keeps its own paint inside the clip. The
/// clip path is in page coordinates, so it must sit on a group around the transformed path, never
/// on the path itself (where it would be read after the path's transform, offset by it).
#[test]
fn nested_frame_paint_is_clipped_in_page_space() {
    let mut d = Document::new("t");
    let p = d.first_page().unwrap();
    let page = d.create(p, NodeKind::Frame, "Page", 0.0, 0.0, 400.0, 300.0).unwrap();
    d.get_mut(page).unwrap().fills = vec![Paint::solid(Color::from_hex("FFFFFF").unwrap())];
    let card = d.create(page, NodeKind::Frame, "Card", 40.0, 60.0, 300.0, 120.0).unwrap();
    let c = d.get_mut(card).unwrap();
    c.fills = vec![Paint::solid(Color::from_hex("DDDDDD").unwrap())];
    c.corners.radii = [12.0; 4];
    c.clip = true;
    let badge = d.create(card, NodeKind::Frame, "Badge", 200.0, 40.0, 80.0, 40.0).unwrap();
    let b = d.get_mut(badge).unwrap();
    b.fills = vec![Paint::solid(Color::from_hex("1E6F8E").unwrap())];
    b.strokes = vec![Paint::solid(Color::from_hex("AA0000").unwrap())];
    b.stroke.weight = 2.0;
    let dot = d.create(badge, NodeKind::Rectangle, "Dot", 10.0, 10.0, 20.0, 20.0).unwrap();
    d.get_mut(dot).unwrap().fills = vec![Paint::solid(Color::from_hex("FFFFFF").unwrap())];
    for simplify_stroke in [true, false] {
        let svg = export_svg(&d, &[page], &SvgOptions { simplify_stroke, ..SvgOptions::default() }).unwrap();
        for tag in svg.split('<').filter(|t| t.contains(" clip-path=")) {
            assert!(!tag.contains(" transform="), "clip-path on a transformed element: <{tag}\n{svg}");
        }
        // The card's clip: its rounded outline starts after the top-left radius.
        let def = svg.find("<path d=\"M52 60").unwrap();
        let open = svg[..def].rfind("<clipPath id=\"").unwrap() + "<clipPath id=\"".len();
        let card_clip = &svg[open..open + svg[open..].find('"').unwrap()];
        let wrapped = format!("<g clip-path=\"url(#{card_clip})\">");
        for color in ["#1E6F8E", "#AA0000"] {
            let at = svg.find(color).unwrap_or_else(|| panic!("{color} missing\n{svg}"));
            let path = svg[..at].rfind("<path").unwrap();
            assert!(svg[..path].ends_with(&wrapped), "{color} not in the card's clip\n{svg}");
        }
    }
}

/// "ab" in black with a green second character, as outlines and as live text.
fn two_colour_text() -> (Document, figcraft_doc::NodeId) {
    use figcraft_doc::{StyleRun, TextData};
    figcraft_text::scan_system_fonts();
    let mut d = Document::new("t");
    let page = d.first_page().unwrap();
    let mut text = TextData::new("ab");
    text.style.font_size = 24.0;
    let mut style = text.style.clone();
    style.fills = Some(vec![Paint::solid(Color::from_hex("00FF00").unwrap())]);
    text.runs.push(StyleRun { start: 1, end: 2, style });
    text.normalize();
    let id = d.create(page, NodeKind::Text(text), "Text", 0.0, 0.0, 100.0, 40.0).unwrap();
    d.get_mut(id).unwrap().fills = vec![Paint::solid(Color::BLACK)];
    (d, id)
}

#[test]
fn text_run_fill_survives_outlined_export() {
    let (d, id) = two_colour_text();
    let svg = export_svg(&d, &[id], &SvgOptions { outline_text: true, ..SvgOptions::default() }).unwrap();
    assert!(svg.contains("#000000") && svg.contains("#00FF00"), "{svg}");
    assert_eq!(svg.matches("<path ").count(), 2, "{svg}");
}

#[test]
fn text_run_fill_survives_live_text_export() {
    let (d, id) = two_colour_text();
    let svg = export_svg(&d, &[id], &SvgOptions { outline_text: false, ..SvgOptions::default() }).unwrap();
    assert!(svg.contains("#000000"), "{svg}");
    assert!(svg.contains("fill=\"#00FF00\"") && svg.contains(">b</tspan>") && svg.contains(">a</tspan>"), "{svg}");
}
