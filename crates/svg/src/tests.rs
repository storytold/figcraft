use super::*;
use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, Effect, NodeKind, Region, Segment, VectorNetwork, Vertex, WindingRule};

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
fn export_vector_region_paints_independently() {
    let mut d = Document::new("regions");
    let p = d.first_page().unwrap();
    let network = VectorNetwork {
        vertices: [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (10.0, 0.0), (20.0, 0.0), (20.0, 10.0), (10.0, 10.0)]
            .into_iter()
            .map(|(x, y)| Vertex::new(x, y))
            .collect(),
        segments: (0..8).map(|i| Segment::line(i, (i + 1) % 4 + (i / 4) * 4)).collect(),
        regions: vec![
            Region {
                loops: vec![vec![0, 1, 2, 3]],
                winding: WindingRule::NonZero,
                fills: Some(vec![Paint::solid(Color::from_hex("FF0000").unwrap())]),
                ..Default::default()
            },
            Region {
                loops: vec![vec![4, 5, 6, 7]],
                winding: WindingRule::NonZero,
                fills: Some(vec![Paint::solid(Color::from_hex("0000FF").unwrap())]),
                ..Default::default()
            },
            Region { loops: vec![vec![0, 1, 2, 3]], winding: WindingRule::NonZero, fills: Some(Vec::new()), ..Default::default() },
        ],
    };
    let id = d.create(p, NodeKind::Vector { network }, "V", 0.0, 0.0, 20.0, 10.0).unwrap();
    let svg = export_svg(&d, &[id], &SvgOptions::default()).unwrap();
    assert!(svg.contains("#FF0000") && svg.contains("#0000FF"), "{svg}");
    assert_eq!(svg.matches("<path ").count(), 2, "empty region emits no path");
}

#[test]
fn import_rejects_garbage() {
    let mut d = Document::new("t");
    let p = d.first_page().unwrap();
    assert!(import_svg(&mut d, b"not svg", p, 0.0, 0.0, "x").is_err());
}

#[test]
fn region_gradient_export_keeps_the_original_paint_box() {
    use figcraft_color::{Gradient, GradientKind};
    use figcraft_doc::RegionPaintSpace;
    use figcraft_geom::{Affine, Rect, Shape};
    let mut d = Document::new("region gradient");
    let page = d.first_page().unwrap();
    let mut network = VectorNetwork::from_path(&Rect::new(20.0, 10.0, 80.0, 40.0).to_path(0.1));
    network.regions[0].fills = Some(vec![Paint::gradient(Gradient::default_for(GradientKind::Linear, Color::BLACK))]);
    network.regions[0].paint_space = Some(RegionPaintSpace { width: 60.0, height: 30.0, transform: Affine::translate((20.0, 10.0)) });
    let id = d.create(page, NodeKind::Vector { network }, "V", 0.0, 0.0, 120.0, 100.0).unwrap();
    let svg = export_svg(&d, &[id], &SvgOptions::default()).unwrap();
    assert!(svg.contains("x1=\"30\" y1=\"0\" x2=\"30\" y2=\"30\""), "{svg}");
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

/// #1124: a font family and a layer name (written as an id) with characters XML forbids leave them
/// out too, whatever text itself does, so the file parses.
#[test]
fn ids_and_font_families_drop_xml_forbidden_control_characters() {
    use figcraft_doc::TextData;
    figcraft_text::scan_system_fonts();
    let forbidden = ['\u{0}', '\u{1}', '\u{8}', '\u{b}', '\u{c}', '\u{e}', '\u{1f}', '\u{fffe}', '\u{ffff}'];
    let junk: String = forbidden.iter().flat_map(|c| ['x', *c]).collect();
    let mut d = Document::new("t");
    let page = d.first_page().unwrap();
    let mut text = TextData::new(format!("a{junk}b"));
    text.style.font_size = 24.0;
    text.style.font_family = format!("Font{junk}<&\"");
    let id = d.create(page, NodeKind::Text(text), &format!("Layer{junk}"), 0.0, 0.0, 600.0, 40.0).unwrap();
    d.get_mut(id).unwrap().fills = vec![Paint::solid(Color::BLACK)];
    let svg = export_svg(&d, &[id], &SvgOptions { outline_text: false, include_ids: true, ..SvgOptions::default() }).unwrap();
    assert!(!svg.contains(forbidden), "{svg:?}");
    assert!(svg.contains("font-family=\"Fontxxxxxxxxx&lt;&amp;&quot;\""), "{svg}");
    let mut d2 = Document::new("t2");
    let p2 = d2.first_page().unwrap();
    assert!(import_svg(&mut d2, svg.as_bytes(), p2, 0.0, 0.0, "x").is_ok(), "{svg:?}");
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

#[test]
fn matrix_keeps_small_scale_precision() {
    assert_eq!(matrix(Affine::scale(0.0375)), "matrix(0.0375 0 0 0.0375 0 0)");
    assert_eq!(matrix(Affine::scale(0.0001)), "matrix(0.0001 0 0 0.0001 0 0)");
}

#[test]
fn matrix_snaps_rotation_noise_and_tiny_values() {
    let r = Affine::rotate(std::f64::consts::FRAC_PI_2);
    assert_eq!(matrix(r), "matrix(0 1 -1 0 0 0)");
    assert_eq!(matrix(Affine::scale(1e-320)), "matrix(0 0 0 0 0 0)");
}

/// A dashed stroke's `<path>`s in `svg`, each as (dash pattern, dash offset).
fn dash_attrs(svg: &str) -> Vec<(String, Option<String>)> {
    let attr = |e: &str, name: &str| e.split(&format!(" {name}=\"")).nth(1).and_then(|r| r.split('"').next()).map(str::to_string);
    svg.split("<path").filter_map(|e| Some((attr(e, "stroke-dasharray")?, attr(e, "stroke-dashoffset")))).collect()
}

/// A page with one centred dashed stroke along `network`'s path or around a rectangle.
fn dashed(kind: NodeKind, dashes: Vec<f64>) -> String {
    let mut d = Document::new("dashes");
    let p = d.first_page().unwrap();
    let n = d.create(p, kind, "Dashed", 0.0, 0.0, 100.0, 50.0).unwrap();
    let node = d.get_mut(n).unwrap();
    node.fills.clear();
    node.strokes = vec![Paint::solid(Color::from_hex("000000").unwrap())];
    node.stroke.align = figcraft_doc::StrokeAlign::Center;
    node.stroke.dashes = dashes;
    export_svg(&d, &[n], &SvgOptions { simplify_stroke: true, ..SvgOptions::default() }).unwrap()
}

/// The `stroke-linecap` a dashed or solid line with a round start cap and `dash_cap` writes.
fn linecap(dashes: Vec<f64>, dash_cap: Option<figcraft_doc::StrokeCap>) -> Option<String> {
    let mut d = Document::new("dash cap");
    let p = d.first_page().unwrap();
    let n = d.create(p, NodeKind::Line, "Line", 0.0, 0.0, 100.0, 0.0).unwrap();
    let node = d.get_mut(n).unwrap();
    node.strokes = vec![Paint::solid(Color::from_hex("000000").unwrap())];
    node.stroke.align = figcraft_doc::StrokeAlign::Center;
    (node.stroke.cap_start, node.stroke.cap_end) = (figcraft_doc::StrokeCap::Round, figcraft_doc::StrokeCap::Round);
    (node.stroke.dashes, node.stroke.dash_cap) = (dashes, dash_cap);
    let svg = export_svg(&d, &[n], &SvgOptions { simplify_stroke: true, ..SvgOptions::default() }).unwrap();
    assert!(svg.contains("stroke-width"), "a stroke, not an outline: {svg}");
    svg.split(" stroke-linecap=\"").nth(1).and_then(|r| r.split('"').next()).map(str::to_string)
}

#[test]
fn a_dash_cap_writes_the_line_cap_of_a_dashed_stroke() {
    use figcraft_doc::StrokeCap::{None as Butt, Square};
    assert_eq!(linecap(vec![4.0, 4.0], None).as_deref(), Some("round"), "unset: the end point cap");
    assert_eq!(linecap(vec![4.0, 4.0], Some(Square)).as_deref(), Some("square"));
    assert_eq!(linecap(vec![4.0, 4.0], Some(Butt)), None);
    assert_eq!(linecap(vec![], Some(Square)).as_deref(), Some("round"), "solid: it doesn't apply");
}

/// A viewer draws what the canvas does: an open line's dashes fitted and started half a dash in.
#[test]
fn a_dashed_line_exports_as_the_canvas_draws_it() {
    let line = Segment::line(0, 1);
    let open = VectorNetwork { vertices: vec![Vertex::new(0.0, 0.0), Vertex::new(107.0, 0.0)], segments: vec![line], ..Default::default() };
    let svg = dashed(NodeKind::Vector { network: open }, vec![10.0, 10.0]);
    assert_eq!(dash_attrs(&svg), [("10.7 10.7".to_string(), Some("5.35".to_string()))], "{svg}");
}

#[test]
fn a_closed_dashed_path_exports_its_pattern_as_it_is() {
    let svg = dashed(NodeKind::Rectangle, vec![10.0, 7.0]);
    assert_eq!(dash_attrs(&svg), [("10 7".to_string(), None)], "{svg}");
}

/// Each subpath fits the pattern to its own length, so each is a `<path>` of its own.
#[test]
fn each_subpath_of_a_dashed_vector_exports_its_own_fit() {
    let two = VectorNetwork {
        vertices: vec![Vertex::new(0.0, 0.0), Vertex::new(100.0, 0.0), Vertex::new(0.0, 20.0), Vertex::new(107.0, 20.0)],
        segments: vec![Segment::line(0, 1), Segment::line(2, 3)],
        ..Default::default()
    };
    let svg = dashed(NodeKind::Vector { network: two }, vec![10.0, 10.0]);
    assert_eq!(dash_attrs(&svg).len(), 2, "{svg}");
}

/// What export writes opens as the same line, the offset not counted twice.
#[test]
fn an_exported_dashed_line_opens_as_the_same_line() {
    let src = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="20"><line x2="107" stroke="#000" stroke-dasharray="10.7 10.7" stroke-dashoffset="5.35"/></svg>"##;
    let mut d = Document::new("back");
    let p = d.first_page().unwrap();
    let frame = import_svg(&mut d, src.as_bytes(), p, 0.0, 0.0, "Back").unwrap();
    let dashes: Vec<Vec<f64>> =
        d.subtree(frame).iter().filter_map(|i| d.node(*i)).filter(|n| !n.stroke.dashes.is_empty()).map(|n| n.stroke.dashes.clone()).collect();
    assert!(matches!(dashes.as_slice(), [d] if d.len() == 2 && d.iter().all(|v| (v - 10.7).abs() < 1e-4)), "{dashes:?}");
}
