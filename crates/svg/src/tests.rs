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

#[test]
fn title_case_does_not_restart_words_across_fill_runs_in_live_text() {
    use figcraft_doc::{StyleRun, TextCase, TextData};
    figcraft_text::scan_system_fonts();
    let mut d = Document::new("t");
    let page = d.first_page().unwrap();
    let mut text = TextData::new("hello");
    text.style.font_size = 24.0;
    text.style.case = TextCase::Title;
    let mut style = text.style.clone();
    style.fills = Some(vec![Paint::solid(Color::from_hex("00FF00").unwrap())]);
    text.runs.push(StyleRun { start: 2, end: 3, style });
    text.normalize();
    let id = d.create(page, NodeKind::Text(text), "Text", 0.0, 0.0, 200.0, 40.0).unwrap();
    d.get_mut(id).unwrap().fills = vec![Paint::solid(Color::BLACK)];
    let svg = export_svg(&d, &[id], &SvgOptions { outline_text: false, ..SvgOptions::default() }).unwrap();
    assert!(svg.contains(">He</tspan>") && svg.contains(">l</tspan>") && svg.contains(">lo</tspan>"), "{svg}");
}

/// Control characters in text (a pasted vertical-tab soft break, U+0001) are not allowed in XML:
/// live-text export must leave them out so the SVG still parses, for plain and per-run spans.
#[test]
fn live_text_export_drops_xml_forbidden_control_characters() {
    use figcraft_doc::{StyleRun, TextData};
    figcraft_text::scan_system_fonts();
    for styled in [false, true] {
        let mut d = Document::new("t");
        let page = d.first_page().unwrap();
        let mut text = TextData::new("line one\u{b}line two\u{1}");
        text.style.font_size = 24.0;
        if styled {
            let mut style = text.style.clone();
            style.fills = Some(vec![Paint::solid(Color::from_hex("00FF00").unwrap())]);
            text.runs.push(StyleRun { start: 5, end: 12, style });
            text.normalize();
        }
        let id = d.create(page, NodeKind::Text(text), "Text", 0.0, 0.0, 600.0, 40.0).unwrap();
        d.get_mut(id).unwrap().fills = vec![Paint::solid(Color::BLACK)];
        let svg = export_svg(&d, &[id], &SvgOptions { outline_text: false, ..SvgOptions::default() }).unwrap();
        assert!(!svg.contains('\u{b}') && !svg.contains('\u{1}'), "{svg:?}");
        assert!(svg.contains("line") && svg.contains("two"), "{svg}");
        let mut d2 = Document::new("t2");
        let p2 = d2.first_page().unwrap();
        assert!(import_svg(&mut d2, svg.as_bytes(), p2, 0.0, 0.0, "x").is_ok(), "{svg:?}");
    }
}

#[test]
fn side_strokes_on_rounded_corners_export_as_their_area() {
    // border-l-4 rounded-r-lg: the stroke path is the 4 px bar (60 px tall), drawn even-odd.
    use figcraft_geom::Shape;
    let mut d = Document::new("t");
    let p = d.first_page().unwrap();
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 120.0, 60.0).unwrap();
    let n = d.get_mut(f).unwrap();
    n.corners.radii = [0.0, 8.0, 8.0, 0.0];
    n.strokes = vec![Paint::solid(Color::from_hex("FF0000").unwrap())];
    n.stroke.align = figcraft_doc::StrokeAlign::Inside;
    n.stroke.sides = Some([0.0, 0.0, 0.0, 4.0]);
    let svg = export_svg(&d, &[f], &SvgOptions::default()).unwrap();
    let stroke = svg.split("<path").find(|e| e.contains("#FF0000")).unwrap();
    assert!(stroke.contains("fill-rule=\"evenodd\""), "{stroke}");
    let data = stroke.split("d=\"").nth(1).and_then(|s| s.split('"').next()).unwrap();
    let area = figcraft_geom::BezPath::from_svg(data).unwrap().area().abs();
    assert!((area - 240.0).abs() < 0.01, "{area}");
}

#[test]
fn point_joins_equal_to_the_layer_join_keep_a_plain_stroke() {
    use figcraft_doc::{StrokeJoin, VectorNetwork};
    use figcraft_geom::kurbo::Point;
    let svg_with = |join: StrokeJoin| {
        let mut network = VectorNetwork::from_points(&[Point::new(0.0, 0.0), Point::new(50.0, 50.0), Point::new(100.0, 0.0)], false);
        for v in &mut network.vertices {
            v.join = Some(StrokeJoin::Miter);
        }
        if let Some(v) = network.vertices.get_mut(1) {
            v.join = Some(join);
        }
        let mut d = Document::new("t");
        let p = d.first_page().unwrap();
        let v = d.create(p, NodeKind::Vector { network }, "V", 0.0, 0.0, 100.0, 50.0).unwrap();
        let n = d.get_mut(v).unwrap();
        n.strokes = vec![Paint::solid(Color::from_hex("000000").unwrap())];
        n.stroke.weight = 2.0;
        n.stroke.join = StrokeJoin::Miter;
        export_svg(&d, &[v], &SvgOptions::default()).unwrap()
    };
    assert!(svg_with(StrokeJoin::Miter).contains("stroke-width=\"2\""), "equal joins stay a stroke");
    assert!(!svg_with(StrokeJoin::Round).contains("stroke-width="), "a differing join is drawn as an outline");
}

#[test]
fn unique_id_is_linear_for_repeated_names() {
    let d = Document::new("t");
    let opts = SvgOptions::default();
    let mut w = W {
        doc: &d,
        roots: &[],
        page: Affine::IDENTITY,
        defs: String::new(),
        body: String::new(),
        next: 0,
        opts: &opts,
        mask: None,
        clip: None,
        used_ids: Default::default(),
        next_suffix: Default::default(),
    };
    assert_eq!(w.unique_id("a"), "a");
    assert_eq!(w.unique_id("a_3"), "a_3");
    assert_eq!(w.unique_id("a"), "a_2");
    assert_eq!(w.unique_id("a"), "a_4");
    // Unique as written, without what XML leaves out.
    assert_eq!(w.unique_id("b\u{fffe}"), "b");
    assert_eq!(w.unique_id("b\u{ffff}"), "b_2");
    let start = std::time::Instant::now();
    for _ in 0..30_000 {
        w.unique_id("Rectangle");
    }
    assert_eq!(w.unique_id("Rectangle"), "Rectangle_30001");
    assert!(start.elapsed() < std::time::Duration::from_secs(2), "unique_id is quadratic: {:?}", start.elapsed());
}

#[test]
fn layer_ids_never_clash_with_generated_ids() {
    let mut d = Document::new("t");
    let p = d.first_page().unwrap();
    let f = d.create(p, NodeKind::Frame, "clip1", 0.0, 0.0, 100.0, 100.0).unwrap();
    d.get_mut(f).unwrap().clip = true;
    d.create(f, NodeKind::Rectangle, "R", 0.0, 0.0, 50.0, 50.0).unwrap();
    let svg = export_svg(&d, &[f], &SvgOptions { include_ids: true, ..SvgOptions::default() }).unwrap();
    assert_eq!(svg.matches("id=\"clip1\"").count(), 1, "{svg}");
}

#[test]
fn a_filled_open_curve_is_closed_for_its_fill() {
    // SVG fills an open subpath as if closed: one curve and the line back enclose an area.
    let mut d = Document::new("t");
    let p = d.first_page().unwrap();
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50"><path fill="#000000" d="M0 0C0 50 100 50 100 0"/></svg>"##;
    import_svg(&mut d, svg.as_bytes(), p, 0.0, 0.0, "x").unwrap();
    let loops = |n: &figcraft_doc::Node| match &n.kind {
        NodeKind::Vector { network } => network.regions.iter().map(|r| r.loops.len()).sum::<usize>(),
        _ => 0,
    };
    assert_eq!(d.nodes().map(loops).sum::<usize>(), 1);
}
