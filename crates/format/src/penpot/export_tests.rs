use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};

use figcraft_doc::color::{Color, ColorStop, Gradient, GradientKind, ImageScaleMode, Paint, PaintKind};
use figcraft_doc::geom::{Affine, Point, Shape};
use figcraft_doc::proto::{Action, Direction, Easing, Flow, Interaction, OverlayPosition, Transition, Trigger};
use figcraft_doc::{
    AutoLayout, Blur, Collection, CrossAlign, Document, Effect, EffectKind, GridTrack, LayoutMode, LineHeight, MainAlign, Mode, NodeId, NodeKind,
    Shadow, Sizing, StrokeAlign, StrokeCap, StyleRun, TextAlignH, TextAutoResize, TextData, Variable, VariableType, VariableValue, VectorNetwork,
};
use serde_json::{Value, json};

use super::{ROOT_ID, export, import};

fn entries(bytes: &[u8]) -> BTreeMap<String, Value> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut out = BTreeMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let name = f.name().to_owned();
        if name.ends_with(".json") {
            let mut text = String::new();
            f.read_to_string(&mut text).unwrap();
            out.insert(name, serde_json::from_str(&text).unwrap());
        }
    }
    out
}

fn shape<'a>(entries: &'a BTreeMap<String, Value>, name: &str) -> &'a Value {
    entries
        .iter()
        .filter(|(path, _)| path.matches('/').count() == 4)
        .map(|(_, v)| v)
        .find(|v| v["name"] == name)
        .unwrap_or_else(|| panic!("no shape named {name}"))
}

fn named(doc: &Document, name: &str) -> NodeId {
    doc.nodes().find(|n| n.name == name).unwrap_or_else(|| panic!("no layer named {name}")).id
}

fn roundtrip(doc: &Document) -> (Document, BTreeMap<String, Value>, Vec<u8>) {
    let (bytes, _) = export(doc).unwrap();
    let (back, _) = import(&bytes).unwrap();
    (back, entries(&bytes), bytes)
}

fn page(doc: &Document) -> NodeId {
    doc.first_page().unwrap()
}

#[test]
fn automatic_wrapped_line_alignment_follows_the_cross_axis() {
    for (cross_align, expected) in [(CrossAlign::Min, "start"), (CrossAlign::Center, "center"), (CrossAlign::Max, "end")] {
        let mut doc = Document::new("Wrapped alignment");
        let p = page(&doc);
        let frame = add(&mut doc, p, NodeKind::Frame, "Wrapped", 0.0, 0.0, 100.0, 100.0);
        doc.node_mut(frame).unwrap().layout = Some(AutoLayout { wrap: true, cross_align, ..AutoLayout::default() });
        let exported = entries(&export(&doc).unwrap().0);
        assert_eq!(shape(&exported, "Wrapped")["layoutAlignContent"], expected);
    }
}

fn add(doc: &mut Document, parent: NodeId, kind: NodeKind, name: &str, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let id = doc.create(parent, kind, name, x, y, w, h).unwrap();
    doc.node_mut(id).unwrap().fills.clear();
    id
}

fn world_bounds(doc: &Document, id: NodeId) -> [f64; 4] {
    let n = doc.node(id).unwrap();
    let xf = doc.world_transform(id);
    let pts = [Point::new(0.0, 0.0), Point::new(n.width, 0.0), Point::new(n.width, n.height), Point::new(0.0, n.height)].map(|p| xf * p);
    let xs = pts.map(|p| p.x);
    let ys = pts.map(|p| p.y);
    [
        xs.iter().cloned().fold(f64::MAX, f64::min),
        ys.iter().cloned().fold(f64::MAX, f64::min),
        xs.iter().cloned().fold(f64::MIN, f64::max),
        ys.iter().cloned().fold(f64::MIN, f64::max),
    ]
}

fn outline_bounds(doc: &Document, id: NodeId) -> [f64; 4] {
    let n = doc.node(id).unwrap();
    let path = match &n.kind {
        NodeKind::Vector { network } => network.stroke_path(),
        _ => figcraft_doc::geometry::shape_path(n).unwrap(),
    };
    let b = (doc.world_transform(id) * path).bounding_box();
    [b.x0, b.y0, b.x1, b.y1]
}

fn close(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01)
}

#[test]
fn pages_boards_and_groups_keep_their_tree_and_ids() {
    let mut doc = Document::new("Tree");
    let p1 = page(&doc);
    if let NodeKind::Page { background, .. } = &mut doc.node_mut(p1).unwrap().kind {
        *background = Color::from_hex("#123456").unwrap();
    }
    let p2 = doc.add_page("Second");
    let board = add(&mut doc, p1, NodeKind::Frame, "Board", 10.0, 20.0, 300.0, 200.0);
    doc.node_mut(board).unwrap().clip = false;
    let inner = add(&mut doc, board, NodeKind::Frame, "Inner", 10.0, 10.0, 100.0, 100.0);
    let group = add(&mut doc, inner, NodeKind::Group, "Group", 0.0, 0.0, 50.0, 50.0);
    add(&mut doc, group, NodeKind::Rectangle, "Rect", 5.0, 5.0, 40.0, 30.0);
    add(&mut doc, p2, NodeKind::Rectangle, "Loose", 0.0, 0.0, 10.0, 10.0);

    let (back, e, bytes) = roundtrip(&doc);
    assert_eq!(e["manifest.json"]["type"], "penpot/export-files");
    let file = e.iter().find(|(k, _)| k.matches('/').count() == 1).unwrap().1;
    assert_eq!(file["version"], 67);
    assert!(file["migrations"].as_array().unwrap().iter().any(|m| m == "0026-fix-svg-raw-shapes-uuids"));
    let root = e.iter().find(|(k, _)| k.ends_with(&format!("{ROOT_ID}.json"))).unwrap().1;
    assert_eq!(root["fills"][0]["fillColor"], "#123456");
    let rect = shape(&e, "Rect");
    assert_eq!(rect["parentId"], shape(&e, "Group")["id"]);
    assert_eq!(rect["frameId"], shape(&e, "Inner")["id"]);
    assert_eq!(shape(&e, "Inner")["frameId"], shape(&e, "Board")["id"]);
    assert_eq!(shape(&e, "Board")["showContent"], true);

    assert_eq!(back.pages.len(), 2);
    assert_eq!(back.node(back.pages[1]).unwrap().name, "Second");
    assert!(close(world_bounds(&back, named(&back, "Rect")), world_bounds(&doc, named(&doc, "Rect"))));
    assert!(matches!(back.node(named(&back, "Group")).unwrap().kind, NodeKind::Group));
    assert!(!back.node(named(&back, "Board")).unwrap().clip);

    let (again, _) = export(&back).unwrap();
    let (twice, _) = export(&back).unwrap();
    assert_eq!(again, twice);
    let ids_before: BTreeSet<String> = e.keys().cloned().collect();
    let ids_after: BTreeSet<String> = entries(&again).keys().cloned().collect();
    assert_eq!(ids_before, ids_after, "re-exporting an imported file keeps every page and shape id");
    assert!(!bytes.is_empty());
}

#[test]
fn rectangles_ellipses_and_paths_keep_their_geometry() {
    let mut doc = Document::new("Shapes");
    let p = page(&doc);
    let rect = add(&mut doc, p, NodeKind::Rectangle, "Rotated", 100.0, 100.0, 80.0, 40.0);
    {
        let n = doc.node_mut(rect).unwrap();
        n.transform = Affine::translate((100.0, 100.0)) * Affine::rotate(30f64.to_radians());
        n.corners.radii = [1.0, 2.0, 3.0, 4.0];
    }
    add(&mut doc, p, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "Circle", 0.0, 0.0, 30.0, 20.0);
    add(&mut doc, p, NodeKind::Star { count: 5, inner_ratio: 0.4 }, "Star", 200.0, 0.0, 50.0, 50.0);
    let network = VectorNetwork::from_points(&[Point::new(0.0, 0.0), Point::new(40.0, 0.0), Point::new(40.0, 30.0)], false);
    let line = add(&mut doc, p, NodeKind::Vector { network }, "Open", 300.0, 0.0, 40.0, 30.0);
    doc.node_mut(line).unwrap().strokes = vec![Paint::solid(Color::BLACK)];

    let (back, e, _) = roundtrip(&doc);
    assert_eq!(shape(&e, "Rotated")["type"], "rect");
    assert_eq!(shape(&e, "Rotated")["r3"], 3);
    assert!((shape(&e, "Rotated")["rotation"].as_f64().unwrap() - 30.0).abs() < 1e-6);
    assert_eq!(shape(&e, "Circle")["type"], "circle");
    assert_eq!(shape(&e, "Star")["type"], "path");
    assert_eq!(shape(&e, "Open")["content"].as_array().unwrap().len(), 3);
    for name in ["Rotated", "Circle", "Star", "Open"] {
        assert!(close(outline_bounds(&back, named(&back, name)), outline_bounds(&doc, named(&doc, name))), "{name} moved");
    }
    let r = back.node(named(&back, "Rotated")).unwrap();
    assert_eq!(r.corners.radii, [1.0, 2.0, 3.0, 4.0]);
    let [a, b, ..] = doc.world_transform(rect).as_coeffs();
    let [a2, b2, ..] = back.world_transform(r.id).as_coeffs();
    assert!((a - a2).abs() < 1e-6 && (b - b2).abs() < 1e-6);
    assert!(matches!(back.node(named(&back, "Circle")).unwrap().kind, NodeKind::Ellipse { .. }));
}

#[test]
fn fills_and_strokes_keep_their_order_and_styles() {
    let mut doc = Document::new("Paints");
    let p = page(&doc);
    let id = add(&mut doc, p, NodeKind::Rectangle, "Painted", 0.0, 0.0, 100.0, 50.0);
    let mut linear = Gradient {
        kind: GradientKind::Linear,
        stops: vec![
            ColorStop { position: 0.0, color: Color::from_hex("#ff0000").unwrap() },
            ColorStop { position: 1.0, color: Color::from_hex("#0000ff80").unwrap() },
        ],
        from: (0.0, 0.5),
        to: (1.0, 0.5),
        width: (0.0, 0.0),
    };
    linear.square_linear_width(1.0, 1.0);
    let radial = Gradient { kind: GradientKind::Radial, from: (0.5, 0.5), to: (1.0, 0.5), width: (0.5, 0.75), ..linear.clone() };
    {
        let n = doc.node_mut(id).unwrap();
        let mut bottom = Paint::solid(Color::from_hex("#00ff00").unwrap());
        bottom.opacity = 0.5;
        n.fills = vec![bottom, Paint::gradient(linear), Paint::gradient(radial)];
        n.strokes = vec![Paint::solid(Color::BLACK)];
        n.stroke.weight = 2.0;
        n.stroke.align = StrokeAlign::Outside;
        n.stroke.dashes = vec![12.0, 12.0];
    }

    let (back, e, _) = roundtrip(&doc);
    let s = shape(&e, "Painted");
    assert_eq!(s["fills"][0]["fillColorGradient"]["type"], "radial");
    assert_eq!(s["fills"][2]["fillColor"], "#00ff00");
    assert_eq!(s["fills"][2]["fillOpacity"], 0.5);
    assert_eq!(s["strokes"][0]["strokeStyle"], "dashed");
    assert_eq!(s["strokes"][0]["strokeAlignment"], "outer");
    assert!(s["strokes"][0].get("strokeJoin").is_none(), "Penpot strokes are a closed schema with no join");

    let n = back.node(named(&back, "Painted")).unwrap();
    let kinds: Vec<&str> = n
        .fills
        .iter()
        .map(|f| match &f.kind {
            PaintKind::Solid { .. } => "solid",
            PaintKind::Gradient(g) if g.kind == GradientKind::Radial => "radial",
            PaintKind::Gradient(_) => "linear",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["solid", "linear", "radial"]);
    assert!((n.fills[0].opacity - 0.5).abs() < 1e-6);
    if let PaintKind::Gradient(g) = &n.fills[2].kind {
        assert!((g.width.0 - 0.5).abs() < 1e-6 && (g.width.1 - 0.75).abs() < 1e-6);
    }
    if let PaintKind::Gradient(g) = &n.fills[1].kind {
        assert!((g.stops[1].color.a - 128.0 / 255.0).abs() < 0.01);
    }
    assert_eq!(n.stroke.weight, 2.0);
    assert_eq!(n.stroke.align, StrokeAlign::Outside);
    assert_eq!(n.stroke.dashes, vec![12.0, 12.0]);
}

#[test]
fn open_path_caps_and_dotted_strokes_survive() {
    let mut doc = Document::new("Caps");
    let p = page(&doc);
    let network = VectorNetwork::from_points(&[Point::new(0.0, 0.0), Point::new(50.0, 0.0)], false);
    let id = add(&mut doc, p, NodeKind::Vector { network }, "Arrow", 0.0, 0.0, 50.0, 1.0);
    {
        let n = doc.node_mut(id).unwrap();
        n.strokes = vec![Paint::solid(Color::BLACK)];
        n.stroke.weight = 1.0;
        n.stroke.cap_end = StrokeCap::TriangleArrow;
        n.stroke.dashes = vec![0.0, 6.0];
    }
    let (back, e, _) = roundtrip(&doc);
    let stroke = &shape(&e, "Arrow")["strokes"][0];
    assert_eq!(stroke["strokeStyle"], "dotted");
    assert_eq!(stroke["strokeCapEnd"], "triangle-arrow");
    let n = back.node(named(&back, "Arrow")).unwrap();
    assert_eq!(n.stroke.cap_end, StrokeCap::TriangleArrow);
    assert_eq!(n.stroke.dashes, vec![0.0, 6.0]);
}

#[test]
fn shadows_and_blurs_round_trip() {
    let mut doc = Document::new("Effects");
    let p = page(&doc);
    let id = add(&mut doc, p, NodeKind::Rectangle, "Shadowed", 0.0, 0.0, 50.0, 50.0);
    let shadow = Shadow { color: Color::from_hex("#00000040").unwrap(), offset: (2.0, 4.0), radius: 8.0, spread: 1.0, ..Shadow::default() };
    doc.node_mut(id).unwrap().effects = vec![
        Effect { kind: EffectKind::DropShadow(shadow.clone()), visible: true },
        Effect { kind: EffectKind::InnerShadow(shadow), visible: false },
        Effect { kind: EffectKind::LayerBlur(Blur::uniform(3.0)), visible: true },
        Effect { kind: EffectKind::BackgroundBlur(Blur::uniform(6.0)), visible: true },
    ];
    let (back, e, _) = roundtrip(&doc);
    let s = shape(&e, "Shadowed");
    assert_eq!(s["shadow"][1]["style"], "inner-shadow");
    assert_eq!(s["blur"]["type"], "layer-blur");
    assert_eq!(s["backgroundBlur"]["type"], "background-blur");
    let effects = &back.node(named(&back, "Shadowed")).unwrap().effects;
    assert_eq!(effects.len(), 4);
    assert!(matches!(&effects[0].kind, EffectKind::DropShadow(s) if s.offset == (2.0, 4.0) && s.radius == 8.0 && (s.color.a - 0.25).abs() < 0.01));
    assert!(matches!(&effects[1].kind, EffectKind::InnerShadow(_)) && !effects[1].visible);
    assert!(matches!(&effects[2].kind, EffectKind::LayerBlur(b) if b.radius == 3.0));
    assert!(matches!(&effects[3].kind, EffectKind::BackgroundBlur(b) if b.radius == 6.0));
}

#[test]
fn flex_layout_round_trips_with_its_child_order() {
    let mut doc = Document::new("Flex");
    let p = page(&doc);
    let frame = add(&mut doc, p, NodeKind::Frame, "Row", 0.0, 0.0, 300.0, 100.0);
    for name in ["First", "Second", "Third"] {
        add(&mut doc, frame, NodeKind::Rectangle, name, 0.0, 0.0, 40.0, 40.0);
    }
    let second = named(&doc, "Second");
    doc.node_mut(second).unwrap().child_layout.sizing_h = Sizing::Fill;
    doc.node_mut(frame).unwrap().layout = Some(AutoLayout {
        mode: LayoutMode::Horizontal,
        gap: 12.0,
        cross_gap: 6.0,
        padding: [1.0, 2.0, 3.0, 4.0],
        main_align: MainAlign::Center,
        cross_align: CrossAlign::Center,
        wrap: true,
        strokes_included: false,
        ..AutoLayout::default()
    });

    let (back, e, _) = roundtrip(&doc);
    let s = shape(&e, "Row");
    assert_eq!(s["layout"], "flex");
    assert_eq!(s["layoutFlexDir"], "row");
    assert_eq!(s["layoutGap"], json!({"rowGap": 6, "columnGap": 12}));
    assert_eq!(s["layoutPadding"], json!({"p1": 1, "p2": 2, "p3": 3, "p4": 4}));
    assert_eq!(shape(&e, "Second")["layoutItemHSizing"], "fill");

    let row = back.node(named(&back, "Row")).unwrap();
    let al = row.layout.as_ref().unwrap();
    assert_eq!((al.mode, al.gap, al.cross_gap, al.padding, al.wrap), (LayoutMode::Horizontal, 12.0, 6.0, [1.0, 2.0, 3.0, 4.0], true));
    assert_eq!((al.main_align, al.cross_align), (MainAlign::Center, CrossAlign::Center));
    let order: Vec<String> = row.children.iter().map(|c| back.node(*c).unwrap().name.clone()).collect();
    assert_eq!(order, ["First", "Second", "Third"]);
    let second = back.node(named(&back, "Second")).unwrap();
    assert_eq!(second.child_layout.sizing_h, Sizing::Fill);
}

#[test]
fn grid_layout_keeps_tracks_and_cells() {
    let mut doc = Document::new("Grid");
    let p = page(&doc);
    let frame = add(&mut doc, p, NodeKind::Frame, "Grid", 0.0, 0.0, 300.0, 200.0);
    let a = add(&mut doc, frame, NodeKind::Rectangle, "A", 0.0, 0.0, 10.0, 10.0);
    add(&mut doc, frame, NodeKind::Rectangle, "B", 0.0, 0.0, 10.0, 10.0);
    {
        let n = doc.node_mut(a).unwrap();
        n.child_layout.grid_row = Some(1);
        n.child_layout.grid_column = Some(0);
        n.child_layout.grid_column_span = 2;
    }
    doc.node_mut(frame).unwrap().layout = Some(AutoLayout {
        mode: LayoutMode::Grid,
        grid_columns: 3,
        grid_rows: 2,
        column_tracks: vec![GridTrack::fixed(50.0), GridTrack::fill(2.0), GridTrack::fixed(75.0)],
        row_tracks: vec![GridTrack::hug(), GridTrack::fixed(40.0)],
        gap: 5.0,
        cross_gap: 7.0,
        ..AutoLayout::default()
    });
    let (back, e, _) = roundtrip(&doc);
    let s = shape(&e, "Grid");
    assert_eq!(s["layoutGridColumns"], json!([{"type": "fixed", "value": 50}, {"type": "flex", "value": 2}, {"type": "fixed", "value": 75}]));
    assert_eq!(s["layoutGridCells"].as_object().unwrap().len(), 5, "one spanning cell and four free ones");
    let al = back.node(named(&back, "Grid")).unwrap().layout.clone().unwrap();
    assert_eq!((al.mode, al.grid_columns, al.grid_rows, al.gap, al.cross_gap), (LayoutMode::Grid, 3, 2, 5.0, 7.0));
    assert_eq!(al.column_tracks[2], GridTrack::fixed(75.0));
    let a = &back.node(named(&back, "A")).unwrap().child_layout;
    assert_eq!((a.grid_row, a.grid_column, a.grid_column_span), (Some(1), Some(0), 2));
    let b = &back.node(named(&back, "B")).unwrap().child_layout;
    assert_eq!((b.grid_row, b.grid_column), (Some(0), Some(0)));
}

#[test]
fn edited_text_is_rebuilt_with_runs_and_styles() {
    let mut doc = Document::new("Text");
    let p = page(&doc);
    let mut text = TextData::new("Hello bold\nSecond line");
    text.style.font_family = "Inter".into();
    text.style.font_size = 18.0;
    text.style.line_height = LineHeight::Percent(150.0);
    text.align_h = TextAlignH::Center;
    text.auto_resize = TextAutoResize::Height;
    let mut bold = text.style.clone();
    bold.font_style = "Bold Italic".into();
    bold.fills = Some(vec![Paint::solid(Color::from_hex("#ff0000").unwrap())]);
    text.runs.push(StyleRun { start: 6, end: 10, style: bold });
    let id = add(&mut doc, p, NodeKind::Text(text), "Copy", 0.0, 0.0, 200.0, 60.0);
    doc.node_mut(id).unwrap().fills = vec![Paint::solid(Color::from_hex("#333333").unwrap())];

    let (back, e, _) = roundtrip(&doc);
    let s = shape(&e, "Copy");
    assert_eq!(s["growType"], "auto-height");
    let paragraphs = s["content"]["children"][0]["children"].as_array().unwrap();
    assert_eq!(paragraphs.len(), 2);
    let leaves = paragraphs[0]["children"].as_array().unwrap();
    assert_eq!(leaves[1]["text"], "bold");
    assert_eq!(leaves[1]["fontWeight"], "700");
    assert_eq!(leaves[1]["fontStyle"], "italic");
    assert_eq!(leaves[1]["fontId"], "gfont-inter");
    assert_eq!(leaves[0]["textAlign"], "center");

    let t = back.node(named(&back, "Copy")).unwrap().kind.text().unwrap().clone();
    assert_eq!(t.characters, "Hello bold\nSecond line");
    assert_eq!(t.style.font_size, 18.0);
    assert_eq!(t.style.line_height, LineHeight::Percent(150.0));
    assert_eq!(t.align_h, TextAlignH::Center);
    assert_eq!(t.auto_resize, TextAutoResize::Height);
    let run = t.runs.iter().find(|r| r.start == 6).unwrap();
    assert_eq!((run.end, run.style.font_style.as_str()), (10, "Bold Italic"));
    assert_eq!(run.style.fills.as_ref().and_then(|f| f[0].solid_color()), Color::from_hex("#ff0000"));
}

fn penpot_text_archive() -> Vec<u8> {
    let file = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let page = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    let text = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
    let content = json!({"type": "root", "children": [{"type": "paragraph-set", "children": [{"type": "paragraph", "key": "k1", "fontId": "gfont-dm-sans", "fontFamily": "DM Sans", "fontSize": "20", "fontWeight": "600", "fontVariantId": "600", "children": [{"text": "Kept as written", "key": "k2", "fontId": "gfont-dm-sans", "fontFamily": "DM Sans", "fontSize": "20", "fontWeight": "600", "fontVariantId": "600", "fills": [{"fillColor": "#09071f", "fillOpacity": 1}]}]}]}]});
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    let files = [
        ("manifest.json".to_owned(), json!({"type": "penpot/export-files", "version": 1, "files": [{"id": file, "name": "Kept"}]})),
        (format!("files/{file}.json"), json!({"id": file, "name": "Kept", "version": 67, "features": ["components/v2"], "migrations": ["legacy-2"]})),
        (format!("files/{file}/pages/{page}.json"), json!({"id": page, "name": "Page", "index": 0})),
        (format!("files/{file}/pages/{page}/{ROOT_ID}.json"), json!({"id": ROOT_ID, "type": "frame", "shapes": [text]})),
        (
            format!("files/{file}/pages/{page}/{text}.json"),
            json!({"id": text, "name": "Title", "type": "text", "x": 10, "y": 20, "width": 200, "height": 24, "parentId": ROOT_ID, "frameId": ROOT_ID, "growType": "auto-width", "content": content, "positionData": [{"x": 10, "y": 44, "width": 150, "height": 24, "text": "Kept as written", "fills": []}]}),
        ),
    ];
    for (name, value) in files {
        zip::ZipWriter::start_file(&mut z, name, opts).unwrap();
        std::io::Write::write_all(&mut z, value.to_string().as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

#[test]
fn unchanged_penpot_text_and_file_header_are_written_back() {
    let (doc, _) = import(&penpot_text_archive()).unwrap();
    let (bytes, _) = export(&doc).unwrap();
    let e = entries(&bytes);
    let s = shape(&e, "Title");
    assert_eq!(s["id"], "cccccccc-cccc-4ccc-8ccc-cccccccccccc");
    assert_eq!(s["content"]["children"][0]["children"][0]["key"], "k1");
    assert_eq!(s["positionData"][0]["text"], "Kept as written");
    let file = &e["files/aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa.json"];
    assert_eq!(file["migrations"], json!(["legacy-2"]));
    assert_eq!(file["features"], json!(["components/v2"]));
    assert!(e.contains_key("files/aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa/pages/bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb.json"));

    let mut edited = doc.clone();
    let id = named(&edited, "Title");
    if let NodeKind::Text(t) = &mut edited.node_mut(id).unwrap().kind {
        t.characters = "Edited".into();
    }
    let e = entries(&export(&edited).unwrap().0);
    let s = shape(&e, "Title");
    assert_eq!(s["content"]["children"][0]["children"][0]["children"][0]["text"], "Edited");
    assert_eq!(s["content"]["children"][0]["children"][0]["children"][0]["fontId"], "gfont-dm-sans");
    assert!(s.get("positionData").is_none());
}

#[test]
fn components_instances_and_variants_round_trip() {
    let mut doc = Document::new("Library");
    let p = page(&doc);
    let main = add(&mut doc, p, NodeKind::Component { description: String::new(), props: BTreeMap::new() }, "Button", 0.0, 0.0, 120.0, 40.0);
    let label = add(&mut doc, main, NodeKind::Rectangle, "Fill", 0.0, 0.0, 120.0, 40.0);
    doc.node_mut(label).unwrap().fills = vec![Paint::solid(Color::BLACK)];
    let copy = add(&mut doc, p, NodeKind::Instance { component: main, props: BTreeMap::new() }, "Button copy", 0.0, 100.0, 120.0, 40.0);
    let inner = add(&mut doc, copy, NodeKind::Rectangle, "Fill", 0.0, 0.0, 120.0, 40.0);
    {
        let n = doc.node_mut(inner).unwrap();
        n.fills = vec![Paint::solid(Color::WHITE)];
        n.source = Some(label);
        n.overrides.insert("fills".into());
    }
    doc.node_mut(copy).unwrap().source = Some(main);
    let set = add(&mut doc, p, NodeKind::ComponentSet { description: String::new() }, "Size", 300.0, 0.0, 200.0, 100.0);
    for (name, y) in [("size=small", 0.0), ("size=large", 50.0)] {
        add(&mut doc, set, NodeKind::Component { description: String::new(), props: BTreeMap::new() }, name, 0.0, y, 100.0, 40.0);
    }

    let (back, e, _) = roundtrip(&doc);
    let components: Vec<&Value> = e.iter().filter(|(k, _)| k.contains("/components/")).map(|(_, v)| v).collect();
    assert_eq!(components.len(), 3);
    let variant = components.iter().find(|c| c["variantProperties"].is_array()).unwrap();
    assert_eq!(variant["variantId"], shape(&e, "Size")["id"]);
    assert_eq!(shape(&e, "Size")["isVariantContainer"], true);
    let copy_json = shape(&e, "Button copy");
    assert_eq!(copy_json["componentRoot"], true);
    assert_eq!(copy_json["shapeRef"], shape(&e, "Button")["id"]);
    let inner_json = e.values().find(|v| v["name"] == "Fill" && v["shapeRef"].is_string()).unwrap();
    assert_eq!(inner_json["touched"], json!(["fill-group"]));

    let copy = back.node(named(&back, "Button copy")).unwrap();
    assert!(matches!(copy.kind, NodeKind::Instance { component, .. } if component == named(&back, "Button")));
    assert!(copy.children.iter().all(|c| back.node(*c).unwrap().source.is_some()));
    let set = back.node(named(&back, "Size")).unwrap();
    assert!(matches!(set.kind, NodeKind::ComponentSet { .. }));
    let names: BTreeSet<String> = set.children.iter().map(|c| back.node(*c).unwrap().name.clone()).collect();
    assert_eq!(names, BTreeSet::from(["size=small".to_owned(), "size=large".to_owned()]));
}

fn variable(id: &str, name: &str, collection: &str, kind: VariableType, values: &[(&str, VariableValue)]) -> Variable {
    Variable {
        id: id.into(),
        name: name.into(),
        collection: collection.into(),
        kind,
        values: values.iter().map(|(m, v)| ((*m).to_owned(), v.clone())).collect(),
        description: String::new(),
        scopes: None,
        hidden_from_publishing: false,
        extends: None,
        overrides: BTreeSet::new(),
        code_syntax: BTreeMap::new(),
    }
}

#[test]
fn native_variables_become_token_sets_themes_and_applied_tokens() {
    let mut doc = Document::new("Tokens");
    doc.variables.collections.push(Collection {
        id: "c1".into(),
        name: "Color".into(),
        modes: vec![Mode { id: "light".into(), name: "Light".into() }, Mode { id: "dark".into(), name: "Dark".into() }],
        extends: None,
    });
    doc.variables.collections.push(Collection {
        id: "c2".into(),
        name: "Size".into(),
        modes: vec![Mode { id: "one".into(), name: "Default".into() }],
        extends: None,
    });
    doc.variables.variables.push(variable(
        "v1",
        "brand/primary",
        "c1",
        VariableType::Color,
        &[("light", VariableValue::Color(Color::from_hex("#ff0000").unwrap())), ("dark", VariableValue::Color(Color::from_hex("#880000").unwrap()))],
    ));
    doc.variables.variables.push(variable("v2", "space/m", "c2", VariableType::Number, &[("one", VariableValue::Number(16.0))]));
    doc.variables.variables.push(variable(
        "v3",
        "brand/alias",
        "c1",
        VariableType::Color,
        &[("light", VariableValue::Alias("v1".into())), ("dark", VariableValue::Alias("v1".into()))],
    ));
    let p = page(&doc);
    let id = add(&mut doc, p, NodeKind::Rectangle, "Bound", 0.0, 0.0, 16.0, 10.0);
    doc.node_mut(id).unwrap().fills = vec![Paint::solid(Color::from_hex("#ff0000").unwrap())];
    doc.node_mut(id).unwrap().bound.insert("fills/0/color".into(), "v1".into());
    doc.node_mut(id).unwrap().bound.insert("width".into(), "v2".into());

    let (bytes, _) = export(&doc).unwrap();
    let mut z = zip::ZipArchive::new(Cursor::new(&bytes[..])).unwrap();
    let tokens_path = z.file_names().find(|n| n.ends_with("tokens.json")).unwrap().to_owned();
    let mut text = String::new();
    z.by_name(&tokens_path).unwrap().read_to_string(&mut text).unwrap();
    let tokens: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(tokens["Color/Light"]["brand"]["primary"]["$value"], "#ff0000");
    assert_eq!(tokens["Color/Dark"]["brand"]["primary"]["$value"], "#880000");
    assert_eq!(tokens["Color/Light"]["brand"]["alias"]["$value"], "{brand.primary}");
    assert_eq!(tokens["Size"]["space"]["m"]["$value"], "16");
    assert_eq!(tokens["$themes"].as_array().unwrap().len(), 2);
    assert_eq!(tokens["$metadata"]["activeThemes"], json!(["Color/Light"]));
    let e = entries(&bytes);
    assert_eq!(shape(&e, "Bound")["appliedTokens"], json!({"fill": "brand.primary", "width": "space.m"}));

    let (back, _) = import(&bytes).unwrap();
    assert!(back.variables.variables.iter().any(|v| v.name.ends_with("brand.primary")));
    let bound = back.node(named(&back, "Bound")).unwrap();
    assert!(bound.bound.contains_key("width"));
}

#[test]
fn imported_penpot_tokens_are_written_back_verbatim() {
    let mut doc = Document::new("Kept tokens");
    let p = page(&doc);
    let source = r#"{"Base":{"radius":{"$value":"4","$type":"borderRadius","$description":""}},"$themes":[],"$metadata":{"tokenSetOrder":["Base"]}}"#;
    doc.node_mut(p).unwrap().plugin_data.insert("penpot.tokens.f".into(), source.into());
    let fp = super::tokens_fingerprint(&doc).unwrap();
    doc.node_mut(p).unwrap().plugin_data.insert("penpot.fp.tokens".into(), fp);
    let (bytes, warnings) = export(&doc).unwrap();
    let mut z = zip::ZipArchive::new(Cursor::new(&bytes[..])).unwrap();
    let path = z.file_names().find(|n| n.ends_with("tokens.json")).unwrap().to_owned();
    let mut text = String::new();
    z.by_name(&path).unwrap().read_to_string(&mut text).unwrap();
    assert_eq!(text, source);
    assert!(warnings.is_empty());
}

#[test]
fn prototype_interactions_and_flows_round_trip() {
    let mut doc = Document::new("Proto");
    let p = page(&doc);
    let home = add(&mut doc, p, NodeKind::Frame, "Home", 0.0, 0.0, 100.0, 100.0);
    let next = add(&mut doc, p, NodeKind::Frame, "Next", 200.0, 0.0, 100.0, 100.0);
    let menu = add(&mut doc, p, NodeKind::Frame, "Menu", 400.0, 0.0, 50.0, 50.0);
    let button = add(&mut doc, home, NodeKind::Rectangle, "Go", 10.0, 10.0, 30.0, 30.0);
    doc.node_mut(button).unwrap().interactions = vec![
        Interaction {
            trigger: Trigger::OnClick,
            actions: vec![Action::Navigate { dest: next, preserve_scroll: true }],
            transition: Transition::SlideIn { direction: Direction::Right },
            duration_ms: 300,
            easing: Easing::EaseOut,
            delay_ms: 0,
        },
        Interaction {
            trigger: Trigger::MouseEnter,
            actions: vec![Action::OpenOverlay {
                anchor: Default::default(),
                dest: menu,
                position: OverlayPosition::Center,
                close_on_outside: true,
                background: Some(Color::BLACK),
                offset: None,
            }],
            transition: Transition::Dissolve,
            duration_ms: 200,
            easing: Easing::Linear,
            delay_ms: 0,
        },
        Interaction {
            trigger: Trigger::AfterDelay { ms: 1500 },
            actions: vec![Action::Back],
            transition: Transition::Instant,
            duration_ms: 0,
            easing: Easing::Linear,
            delay_ms: 0,
        },
    ];
    if let NodeKind::Page { flows, .. } = &mut doc.node_mut(p).unwrap().kind {
        flows.push(Flow { name: "Main".into(), start: home, description: String::new() });
    }
    let (back, e, _) = roundtrip(&doc);
    let items = shape(&e, "Go")["interactions"].as_array().unwrap().clone();
    assert_eq!(items[0]["destination"], shape(&e, "Next")["id"]);
    assert_eq!(items[0]["animation"]["animationType"], "slide");
    assert_eq!(items[1]["actionType"], "open-overlay");
    assert_eq!(items[2]["actionType"], "prev-screen");
    let page_json = e.iter().find(|(k, _)| k.matches('/').count() == 3 && k.contains("/pages/")).unwrap().1;
    assert_eq!(page_json["flows"].as_object().unwrap().values().next().unwrap()["startingFrame"], shape(&e, "Home")["id"]);

    let go = back.node(named(&back, "Go")).unwrap();
    assert_eq!(go.interactions.len(), 3);
    assert!(matches!(go.interactions[0].actions[0], Action::Navigate { dest, preserve_scroll: true } if dest == named(&back, "Next")));
    assert_eq!(go.interactions[0].transition, Transition::SlideIn { direction: Direction::Right });
    assert_eq!((go.interactions[0].duration_ms, go.interactions[0].easing), (300, Easing::EaseOut));
    assert!(matches!(go.interactions[1].actions[0], Action::OpenOverlay { dest, close_on_outside: true, .. } if dest == named(&back, "Menu")));
    assert!(matches!(go.interactions[2].trigger, Trigger::AfterDelay { ms: 1500 }));
    let NodeKind::Page { flows, .. } = &back.node(back.pages[0]).unwrap().kind else { panic!() };
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0].start, named(&back, "Home"));
    if let Action::OpenOverlay { anchor, .. } = &mut doc.node_mut(button).unwrap().interactions[1].actions[0] {
        *anchor = figcraft_doc::proto::OverlayAnchor::TriggerOutside;
    }
    let (_, warnings) = export(&doc).unwrap();
    assert!(warnings.iter().any(|warning| warning.contains("Overlay anchor") && warning.contains("screen")), "{warnings:?}");
}

#[test]
fn image_fills_write_media_and_storage_objects() {
    let mut doc = Document::new("Images");
    let mut png = Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(3, 2, image::Rgba([10, 20, 30, 255])).write_to(&mut png, image::ImageFormat::Png).unwrap();
    let (hash, _, _) = crate::add_image(&mut doc, png.into_inner()).unwrap();
    let p = page(&doc);
    let id = add(&mut doc, p, NodeKind::Rectangle, "Picture", 0.0, 0.0, 30.0, 20.0);
    doc.node_mut(id).unwrap().fills = vec![Paint::image(hash.clone(), ImageScaleMode::Fill)];
    let (back, e, bytes) = roundtrip(&doc);
    let fill = &shape(&e, "Picture")["fills"][0]["fillImage"];
    assert_eq!((fill["width"].as_u64(), fill["height"].as_u64(), fill["mtype"].as_str()), (Some(3), Some(2), Some("image/png")));
    let media = e.iter().find(|(k, _)| k.contains("/media/")).unwrap().1;
    assert_eq!(media["id"], fill["id"]);
    let object = &e[&format!("objects/{}.json", media["mediaId"].as_str().unwrap())];
    let mut z = zip::ZipArchive::new(Cursor::new(&bytes[..])).unwrap();
    let blob = z.by_name(&format!("objects/{}.png", media["mediaId"].as_str().unwrap())).unwrap().size();
    assert_eq!(object["size"].as_u64(), Some(blob));
    assert_eq!(object["bucket"], "file-media-object");
    let n = back.node(named(&back, "Picture")).unwrap();
    assert!(matches!(&n.fills[0].kind, PaintKind::Image(img) if img.image == hash));
}

#[test]
fn duplicated_penpot_ids_are_made_unique_per_page() {
    let mut doc = Document::new("Dupes");
    let p = page(&doc);
    for name in ["One", "Two"] {
        let id = add(&mut doc, p, NodeKind::Rectangle, name, 0.0, 0.0, 10.0, 10.0);
        doc.node_mut(id).unwrap().plugin_data.insert("penpot.id".into(), "dddddddd-dddd-4ddd-8ddd-dddddddddddd".into());
    }
    let (back, e, _) = roundtrip(&doc);
    assert_eq!(shape(&e, "One")["id"], "dddddddd-dddd-4ddd-8ddd-dddddddddddd");
    assert_ne!(shape(&e, "Two")["id"], shape(&e, "One")["id"]);
    assert_eq!(back.nodes().filter(|n| matches!(n.kind, NodeKind::Rectangle)).count(), 2);
}

#[test]
fn unsupported_paints_and_effects_are_reported() {
    let mut doc = Document::new("Lossy");
    let p = page(&doc);
    let id = add(&mut doc, p, NodeKind::Rectangle, "Lossy", 0.0, 0.0, 10.0, 10.0);
    let mut hidden = Paint::solid(Color::BLACK);
    hidden.visible = false;
    doc.node_mut(id).unwrap().fills = vec![hidden];
    let (_, warnings) = export(&doc).unwrap();
    assert!(warnings.iter().any(|w| w.contains("Hidden fills")));
}

#[test]
fn a_coloured_run_does_not_recolour_the_rest_of_the_text() {
    let mut text = TextData::new("plain red");
    text.style.fills = Some(vec![Paint::solid(Color::BLACK)]);
    let mut red = text.style.clone();
    red.fills = Some(vec![Paint::solid(Color::from_hex("#ff0000").unwrap())]);
    text.runs.push(StyleRun { start: 6, end: 9, style: red });
    assert_eq!(super::uniform_text_fills(&text), None);
    text.runs[0].start = 0;
    assert!(super::uniform_text_fills(&text).is_some());
}

#[test]
fn display_grids_and_guides_round_trip() {
    let mut doc = Document::new("Grids");
    let p = page(&doc);
    doc.node_mut(p).unwrap().guides.push(figcraft_doc::Guide { vertical: true, offset: 64.0 });
    let frame = add(&mut doc, p, NodeKind::Frame, "Board", 100.0, 50.0, 400.0, 300.0);
    {
        let n = doc.node_mut(frame).unwrap();
        n.grids = vec![
            figcraft_doc::LayoutGrid { count: 12, gutter: 16.0, offset: 24.0, ..figcraft_doc::LayoutGrid::columns() },
            figcraft_doc::LayoutGrid { size: 8.0, ..figcraft_doc::LayoutGrid::default() },
        ];
        n.guides = vec![figcraft_doc::Guide { vertical: false, offset: 10.0 }];
    }
    let (back, e, _) = roundtrip(&doc);
    let grids = &shape(&e, "Board")["grids"];
    assert_eq!(grids[0]["type"], "column");
    assert_eq!(grids[0]["params"]["size"], 12);
    assert_eq!(grids[1]["params"]["size"], 8);
    let page_json = e.iter().find(|(k, _)| k.matches('/').count() == 3 && k.contains("/pages/")).unwrap().1;
    let guides: Vec<&Value> = page_json["guides"].as_object().unwrap().values().collect();
    assert!(guides.iter().any(|g| g["axis"] == "x" && g["position"] == 64 && g.get("frameId").is_none()));
    assert!(guides.iter().any(|g| g["axis"] == "y" && g["position"] == 60 && g["frameId"] == shape(&e, "Board")["id"]));
    let n = back.node(named(&back, "Board")).unwrap();
    assert_eq!((n.grids[0].count, n.grids[0].gutter, n.grids[0].offset), (12, 16.0, 24.0));
    assert_eq!(n.grids[1].size, 8.0);
    let guides = &back.node(back.pages[0]).unwrap().guides;
    assert!(guides.iter().any(|g| g.vertical && g.offset == 64.0));
}

#[test]
fn raw_svg_layers_go_back_as_svg_raw_without_their_expansion() {
    let mut doc = Document::new("Svg");
    let p = page(&doc);
    let raw = add(&mut doc, p, NodeKind::Frame, "Logo", 0.0, 0.0, 24.0, 24.0);
    add(&mut doc, raw, NodeKind::Frame, "Expanded", 0.0, 0.0, 24.0, 24.0);
    {
        let n = doc.node_mut(raw).unwrap();
        n.plugin_data.insert("penpot.id".into(), "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee".into());
        n.plugin_data.insert("penpot.svgRawStatus".into(), "expanded".into());
        n.plugin_data.insert("penpot.source.content".into(), r#"{"tag":"svg","attrs":{"viewBox":"0 0 24 24"},"content":[]}"#.into());
    }
    let (bytes, _) = export(&doc).unwrap();
    let e = entries(&bytes);
    let s = shape(&e, "Logo");
    assert_eq!(s["type"], "svg-raw");
    assert_eq!(s["content"]["tag"], "svg");
    assert_eq!(s["shapes"], json!([]));
    assert!(!e.values().any(|v| v["name"] == "Expanded"));
}

#[test]
fn grid_children_beyond_the_declared_rows_get_implicit_rows() {
    let mut doc = Document::new("Overflow");
    let p = page(&doc);
    let frame = add(&mut doc, p, NodeKind::Frame, "Grid", 0.0, 0.0, 200.0, 200.0);
    for i in 0..5 {
        add(&mut doc, frame, NodeKind::Rectangle, &format!("Item {i}"), 0.0, 0.0, 10.0, 10.0);
    }
    doc.node_mut(frame).unwrap().layout = Some(AutoLayout { mode: LayoutMode::Grid, grid_columns: 2, grid_rows: 1, ..AutoLayout::default() });
    let (bytes, warnings) = export(&doc).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let e = entries(&bytes);
    let s = shape(&e, "Grid");
    assert_eq!(s["layoutGridRows"].as_array().unwrap().len(), 3);
    let filled = s["layoutGridCells"].as_object().unwrap().values().filter(|c| !c["shapes"].as_array().unwrap().is_empty()).count();
    assert_eq!(filled, 5);
}

fn penpot_archive(shapes: &[Value]) -> Vec<u8> {
    let file = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let page = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    let top: Vec<&Value> = shapes.iter().filter(|s| s["parentId"] == ROOT_ID).map(|s| &s["id"]).collect();
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    let mut files = vec![
        ("manifest.json".to_owned(), json!({"type": "penpot/export-files", "version": 1, "files": [{"id": file, "name": "Kept"}]})),
        (format!("files/{file}/pages/{page}.json"), json!({"id": page, "name": "Page", "index": 0})),
        (format!("files/{file}/pages/{page}/{ROOT_ID}.json"), json!({"id": ROOT_ID, "type": "frame", "shapes": top})),
    ];
    for s in shapes {
        files.push((format!("files/{file}/pages/{page}/{}.json", s["id"].as_str().unwrap()), s.clone()));
    }
    for (name, value) in files {
        zip::ZipWriter::start_file(&mut z, name, opts).unwrap();
        std::io::Write::write_all(&mut z, value.to_string().as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

#[test]
fn penpot_only_layer_data_survives_unchanged_layers() {
    let board = "11111111-1111-4111-8111-111111111111";
    let ring = "22222222-2222-4222-8222-222222222222";
    let label = "33333333-3333-4333-8333-333333333333";
    let strokes = json!([
        {"strokeColor": "#000000", "strokeOpacity": 1, "strokeWidth": 4, "strokeAlignment": "outer", "strokeStyle": "solid"},
        {"strokeColor": "#00ffff", "strokeOpacity": 1, "strokeWidth": 1, "strokeAlignment": "inner", "strokeStyle": "solid"}
    ]);
    let position: Vec<Value> =
        (0..400).map(|i| json!({"x": 0, "y": i, "width": 100, "height": 20, "text": "a line of positioned text", "fills": []})).collect();
    let content = json!({"type": "root", "children": [{"type": "paragraph-set", "children": [{"type": "paragraph", "children": [{"text": "Label", "fontSize": "14"}]}, {"type": "paragraph", "children": [{"text": ""}]}]}]});
    let bytes = penpot_archive(&[
        json!({"id": board, "name": "Board", "type": "frame", "x": 0, "y": 0, "width": 200, "height": 100, "parentId": ROOT_ID, "frameId": ROOT_ID, "shapes": [ring, label], "layout": "flex", "layoutFlexDir": "row", "exports": [{"type": "png", "suffix": "", "scale": 2}]}),
        json!({"id": ring, "name": "Ring", "type": "circle", "x": 10, "y": 10, "width": 40, "height": 40, "parentId": board, "frameId": board, "strokes": strokes, "layoutItemZIndex": 2}),
        json!({"id": label, "name": "Label", "type": "text", "x": 60, "y": 10, "width": 100, "height": 40, "parentId": board, "frameId": board, "content": content, "positionData": position, "hideInViewer": true}),
    ]);
    let (doc, _) = import(&bytes).unwrap();
    let e = entries(&export(&doc).unwrap().0);
    assert_eq!(shape(&e, "Ring")["strokes"], strokes);
    assert_eq!(shape(&e, "Ring")["layoutItemZIndex"], 2);
    assert_eq!(shape(&e, "Board")["exports"][0]["scale"], 2);
    assert_eq!(shape(&e, "Label")["hideInViewer"], true);
    assert_eq!(shape(&e, "Label")["positionData"].as_array().unwrap().len(), 400);
    assert_eq!(shape(&e, "Label")["fills"], json!([]));

    let mut edited = doc.clone();
    let id = named(&edited, "Ring");
    edited.node_mut(id).unwrap().stroke.weight = 6.0;
    let e = entries(&export(&edited).unwrap().0);
    let regenerated = shape(&e, "Ring")["strokes"].as_array().unwrap().clone();
    assert!(regenerated.iter().all(|s| s["strokeWidth"] == 6));
}

#[test]
fn a_penpot_header_without_migrations_lets_penpot_run_them() {
    let mut doc = Document::new("Old");
    let p = page(&doc);
    doc.node_mut(p).unwrap().plugin_data.insert("penpot.source.file".into(), r#"{"version":58,"features":["components/v2"]}"#.into());
    let e = entries(&export(&doc).unwrap().0);
    let file = e.iter().find(|(k, _)| k.matches('/').count() == 1).unwrap().1;
    assert_eq!(file["version"], 58);
    assert!(file.get("migrations").is_none());
}

#[test]
fn legacy_image_shapes_stretch_over_their_fills_like_penpot() {
    let file = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let page = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    let photo_id = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
    let media = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
    let object = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee";
    let mut png = Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(4, 2, image::Rgba([200, 10, 10, 255])).write_to(&mut png, image::ImageFormat::Png).unwrap();
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    let members: Vec<(String, Vec<u8>)> = vec![
        ("manifest.json".into(), json!({"type": "penpot/export-files", "version": 1, "files": [{"id": file, "name": "Legacy"}]}).to_string().into_bytes()),
        (format!("files/{file}/pages/{page}.json"), json!({"id": page, "name": "Page", "index": 0}).to_string().into_bytes()),
        (format!("files/{file}/pages/{page}/{ROOT_ID}.json"), json!({"id": ROOT_ID, "type": "frame", "shapes": [photo_id]}).to_string().into_bytes()),
        (
            format!("files/{file}/pages/{page}/{photo_id}.json"),
            json!({"id": photo_id, "type": "image", "name": "Photo", "width": 100, "height": 100, "metadata": {"id": media, "width": 4, "height": 2, "mtype": "image/png"}, "fills": [{"fillColor": "#00ff00", "fillOpacity": 1}]})
                .to_string()
                .into_bytes(),
        ),
        (format!("files/{file}/media/{media}.json"), json!({"id": media, "mtype": "image/png", "mediaId": object}).to_string().into_bytes()),
        (format!("objects/{object}.png"), png.into_inner()),
    ];
    for (name, data) in members {
        zip::ZipWriter::start_file(&mut z, name, opts).unwrap();
        std::io::Write::write_all(&mut z, &data).unwrap();
    }
    let (doc, _) = import(&z.finish().unwrap().into_inner()).unwrap();
    let photo = doc.node(named(&doc, "Photo")).unwrap();
    assert_eq!(photo.fills.len(), 2);
    assert!(matches!(&photo.fills[1].kind, PaintKind::Image(img) if img.scale_mode == ImageScaleMode::Crop));
    let e = entries(&export(&doc).unwrap().0);
    assert_eq!(shape(&e, "Photo")["fills"][0]["fillImage"]["keepAspectRatio"], false);
    assert_eq!(shape(&e, "Photo")["fills"][1]["fillColor"], "#00ff00");
}

#[test]
fn page_guides_reloaded_from_a_figcraft_file_are_exported() {
    let mut doc = Document::new("Reloaded");
    let p = page(&doc);
    doc.node_mut(p).unwrap().guides = vec![figcraft_doc::Guide { vertical: false, offset: 410.0 }];
    let e = entries(&export(&doc).unwrap().0);
    let page_json = e.iter().find(|(k, _)| k.matches('/').count() == 3 && k.contains("/pages/")).unwrap().1;
    assert_eq!(page_json["guides"].as_object().unwrap().values().next().unwrap()["position"], 410);
}
