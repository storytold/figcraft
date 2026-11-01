//! Vector behaviour suite, written from the public documentation of design tools: entering and
//! leaving vector edit mode, the pen, selecting points and segments, deleting vs. delete and heal,
//! handle mirroring, the bend, paint bucket, cut and eraser tools, joining, nudging, undo steps and
//! what converting a shape keeps. Each rule is a table row (or a step) with a one-line derivation;
//! where the documentation is silent the row says so and pins today's behaviour.

// The tables read best as tuples.
#![allow(clippy::type_complexity)]

use figcraft_doc::{HandleMirroring, NodeId, NodeKind, Region, Segment, VectorNetwork, WindingRule};
use figcraft_geom::kurbo::ParamCurve;
use figcraft_geom::{BezPath, Point, Shape};
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind};
use crate::vectoredit::{VectorEdit, network};

#[test]
fn region_paints_survive_simplify_offset_and_bucket_normalization() {
    let mut s = Session::new();
    let id = square(&mut s, false);
    ex_to_vector(s.doc_mut().unwrap(), id);
    let mut gradient = figcraft_color::Gradient::default_for(figcraft_color::GradientKind::Linear, figcraft_color::Color::BLACK);
    gradient.from = (0.2, 0.1);
    gradient.to = (0.8, 0.9);
    let fills = vec![
        figcraft_color::Paint::gradient(gradient),
        figcraft_color::Paint::image("test-image", figcraft_color::ImageScaleMode::Crop),
        figcraft_color::Paint::pattern(123),
    ];
    {
        let doc = s.doc_mut().unwrap();
        let NodeKind::Vector { network } = &mut doc.get_mut(id).unwrap().kind else { panic!("expected vector fixture") };
        network.regions[0].fills = Some(fills.clone());
    }
    ex(&mut s, "vector.simplify", json!({"ids": [id]}));
    assert_eq!(net(&s, id).regions[0].fills, Some(fills.clone()), "simplify keeps the full stack");
    ex(&mut s, "vector.offset", json!({"ids": [id], "offset": 2}));
    assert_eq!(net(&s, id).regions[0].fills, Some(fills.clone()), "offset keeps the full stack");

    // Add a divider so the legacy single region must split into independently painted faces.
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    let network = net(&s, id);
    let (topseg, botseg) = (
        network.segments.iter().position(|sg| network.vertices[sg.a].y == 0.0 && network.vertices[sg.b].y == 0.0).unwrap(),
        network.segments.iter().position(|sg| network.vertices[sg.a].y == 104.0 && network.vertices[sg.b].y == 104.0).unwrap(),
    );
    let a = ex(&mut s, "vector.edit.addVertex", json!({"segment": topseg, "t": 0.5}))["vertex"].as_u64().unwrap();
    let b = ex(&mut s, "vector.edit.addVertex", json!({"segment": botseg, "t": 0.5}))["vertex"].as_u64().unwrap();
    ex(&mut s, "vector.edit.connect", json!({"a": a, "b": b}));
    ex(&mut s, "vector.edit.paint", json!({"x": 25, "y": 52}));
    let normalized = net(&s, id);
    assert_eq!(normalized.regions.len(), 1, "unfilled face is removed while the other stays");
    assert_eq!(normalized.regions[0].fills, Some(fills.clone()), "untouched face keeps the original stack");
    assert!(!normalized.fill_contains(Point::new(27.0, 54.0)));
    assert!(normalized.fill_contains(Point::new(77.0, 54.0)));
    s.undo().unwrap();
    assert!(net(&s, id).fill_contains(Point::new(25.0, 52.0)), "bucket change is undoable");
    ex(&mut s, "vector.edit.end", json!({}));

    let group = id_of(&ex(&mut s, "object.group", json!({"ids": [id]}))["id"]).unwrap();
    let flattened = id_of(&ex(&mut s, "vector.flatten", json!({"ids": [group]}))["id"]).unwrap();
    let flattened_net = net(&s, flattened);
    assert!(flattened_net.regions.iter().all(|region| region.fills == Some(fills.clone())), "flatten keeps each source region stack");
    assert!(flattened_net.regions.iter().all(|region| region.paint_space.is_some()), "flatten keeps source paint frames");
    assert!(flattened_net.fill_contains(Point::new(25.0, 52.0)));
    let prior_frame =
        flattened_net.regions[0].paint_frame(s.doc().unwrap().get(flattened).unwrap().width, s.doc().unwrap().get(flattened).unwrap().height);
    let prior_world_frame = s.doc().unwrap().world_transform(flattened) * prior_frame.2;
    let regrouped = id_of(&ex(&mut s, "object.group", json!({"ids": [flattened]}))["id"]).unwrap();
    let flattened_again = id_of(&ex(&mut s, "vector.flatten", json!({"ids": [regrouped]}))["id"]).unwrap();
    let after = net(&s, flattened_again);
    let after_frame =
        after.regions[0].paint_frame(s.doc().unwrap().get(flattened_again).unwrap().width, s.doc().unwrap().get(flattened_again).unwrap().height);
    let after_world_frame = s.doc().unwrap().world_transform(flattened_again) * after_frame.2;
    assert!(
        prior_world_frame.as_coeffs().iter().zip(after_world_frame.as_coeffs()).all(|(before, after)| (before - after).abs() < 1e-8),
        "flattening an already framed gradient preserves its source coordinates"
    );
}

#[test]
fn explicit_and_inherited_region_paints_survive_refit() {
    let mut s = Session::new();
    let empty_id = square(&mut s, false);
    ex_to_vector(s.doc_mut().unwrap(), empty_id);
    {
        let doc = s.doc_mut().unwrap();
        if let NodeKind::Vector { network } = &mut doc.get_mut(empty_id).unwrap().kind {
            network.regions[0].fills = Some(Vec::new());
        }
    }
    ex(&mut s, "vector.simplify", json!({"ids": [empty_id]}));
    ex(&mut s, "vector.offset", json!({"ids": [empty_id], "offset": 2}));
    assert_eq!(net(&s, empty_id).regions[0].fills, Some(Vec::new()), "explicit empty paint stack survives refit");

    let inherited_id = square(&mut s, false);
    ex_to_vector(s.doc_mut().unwrap(), inherited_id);
    {
        let doc = s.doc_mut().unwrap();
        if let NodeKind::Vector { network } = &mut doc.get_mut(inherited_id).unwrap().kind {
            network.regions[0].fills = None;
        }
    }
    ex(&mut s, "vector.offset", json!({"ids": [inherited_id], "offset": 2}));
    let inherited_net = net(&s, inherited_id);
    let inherited = &inherited_net.regions[0];
    assert_eq!(inherited.fills, None, "inherited paints remain inherited after offset");
    let inherited_space = inherited.paint_space.expect("offset binds inherited paints to the original frame");
    assert_eq!((inherited_space.width, inherited_space.height), (100.0, 100.0));
}

#[test]
fn flatten_keeps_inherited_region_paints_inherited() {
    let mut s = Session::new();
    let inherited_flatten = square(&mut s, false);
    let inherited_fills = s.doc().unwrap().get(inherited_flatten).unwrap().fills.clone();
    ex_to_vector(s.doc_mut().unwrap(), inherited_flatten);
    if let NodeKind::Vector { network } = &mut s.doc_mut().unwrap().get_mut(inherited_flatten).unwrap().kind {
        network.regions[0].fills = None;
    }
    let group = id_of(&ex(&mut s, "object.group", json!({"ids": [inherited_flatten]}))["id"]).unwrap();
    let flattened = id_of(&ex(&mut s, "vector.flatten", json!({"ids": [group]}))["id"]).unwrap();
    assert!(
        net(&s, flattened).regions.iter().all(|region| region.fills.is_none()),
        "flatten keeps inherited paints inherited when output node has same fills"
    );
    assert_eq!(s.doc().unwrap().get(flattened).unwrap().fills, inherited_fills);

    let plain = id_of(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 80, "height": 60}))["id"]).unwrap();
    let plain_fills = s.doc().unwrap().get(plain).unwrap().fills.clone();
    let group = id_of(&ex(&mut s, "object.group", json!({"ids": [plain]}))["id"]).unwrap();
    let flattened_plain = id_of(&ex(&mut s, "vector.flatten", json!({"ids": [group]}))["id"]).unwrap();
    assert!(net(&s, flattened_plain).regions.iter().all(|region| region.fills.is_none()), "plain shapes inherit the flattened node's fill");
    assert_eq!(s.doc().unwrap().get(flattened_plain).unwrap().fills, plain_fills);
    let edited_fills = vec![figcraft_color::Paint::solid(figcraft_color::Color::BLACK)];
    s.doc_mut().unwrap().get_mut(flattened_plain).unwrap().fills = edited_fills.clone();
    assert_eq!(net(&s, flattened_plain).regions[0].fills, None, "changing the flattened node fill updates inherited geometry");

    let different_source = square(&mut s, false);
    s.doc_mut().unwrap().get_mut(different_source).unwrap().fills = vec![figcraft_color::Paint::solid(figcraft_color::Color::BLACK)];
    ex_to_vector(s.doc_mut().unwrap(), different_source);
    let source_fills = s.doc().unwrap().get(different_source).unwrap().fills.clone();
    if let NodeKind::Vector { network } = &mut s.doc_mut().unwrap().get_mut(different_source).unwrap().kind {
        network.regions[0].fills = None;
    }
    let group = id_of(&ex(&mut s, "object.group", json!({"ids": [different_source]}))["id"]).unwrap();
    s.doc_mut().unwrap().get_mut(group).unwrap().fills.clear();
    let flattened = id_of(&ex(&mut s, "vector.flatten", json!({"ids": [group]}))["id"]).unwrap();
    assert_eq!(net(&s, flattened).regions[0].fills, Some(source_fills), "flatten retains paints that differ from output node fills");
}

#[test]
fn legacy_offset_preserves_the_gradients_world_paint_frame() {
    let mut s = Session::new();
    let legacy_gradient = square(&mut s, false);
    let mut gradient = figcraft_color::Gradient::default_for(figcraft_color::GradientKind::Linear, figcraft_color::Color::BLACK);
    gradient.from = (0.2, 0.1);
    gradient.to = (0.8, 0.9);
    s.doc_mut().unwrap().get_mut(legacy_gradient).unwrap().fills = vec![figcraft_color::Paint::gradient(gradient)];
    ex_to_vector(s.doc_mut().unwrap(), legacy_gradient);
    if let NodeKind::Vector { network } = &mut s.doc_mut().unwrap().get_mut(legacy_gradient).unwrap().kind {
        network.regions[0].fills = None;
    }
    let before = net(&s, legacy_gradient).regions[0].paint_frame(100.0, 100.0).2;
    let before_world = s.doc().unwrap().world_transform(legacy_gradient) * before;
    ex(&mut s, "vector.offset", json!({"ids": [legacy_gradient], "offset": 2}));
    let after_net = net(&s, legacy_gradient);
    let after = after_net.regions[0]
        .paint_frame(s.doc().unwrap().get(legacy_gradient).unwrap().width, s.doc().unwrap().get(legacy_gradient).unwrap().height)
        .2;
    let after_world = s.doc().unwrap().world_transform(legacy_gradient) * after;
    assert!(
        before_world.as_coeffs().iter().zip(after_world.as_coeffs()).all(|(before, after)| (before - after).abs() < 1e-8),
        "legacy offset preserves the gradient's world paint frame"
    );
}

#[test]
fn bucket_fills_an_explicitly_empty_face_and_leaves_the_other_empty() {
    let mut s = Session::new();
    let empty_faces =
        id_of(&ex(&mut s, "vector.create", json!({"path": "M0 0 L100 0 L100 100 L0 100 Z M200 0 L300 0 L300 100 L200 100 Z"}))["id"]).unwrap();
    {
        let doc = s.doc_mut().unwrap();
        let node = doc.get_mut(empty_faces).unwrap();
        node.fills.clear();
        let NodeKind::Vector { network } = &mut node.kind else { panic!("expected vector fixture") };
        assert!(network.regions.iter().all(|region| region.fills.is_none()));
    }
    ex(&mut s, "vector.edit.begin", json!({"id": empty_faces}));
    ex(&mut s, "vector.edit.paint", json!({"x": 50, "y": 50}));
    let split = net(&s, empty_faces);
    let left =
        split.regions.iter().enumerate().find(|(index, _)| split.region_contains(*index, Point::new(50.0, 50.0))).map(|(_, region)| region).unwrap();
    let right =
        split.regions.iter().enumerate().find(|(index, _)| split.region_contains(*index, Point::new(250.0, 50.0))).map(|(_, region)| region).unwrap();
    assert_eq!(left.fills, None, "clicking an explicitly empty face fills it");
    assert_eq!(right.fills, Some(Vec::new()), "untouched inherited-empty face stays empty");
    assert!(!s.doc().unwrap().get(empty_faces).unwrap().fills.is_empty(), "the painted target gets the layer default fill");
}

#[test]
fn legacy_simplify_combines_mixed_winding_regions() {
    let mut s = Session::new();
    let id = id_of(&ex(&mut s, "vector.create", json!({"path": "M0 0 L100 0 L100 100 L0 100 Z M25 25 L75 25 L75 75 L25 75 Z"}))["id"]).unwrap();
    let mut second_path = BezPath::new();
    second_path.move_to((50.0, 0.0));
    second_path.line_to((150.0, 0.0));
    second_path.line_to((150.0, 100.0));
    second_path.line_to((50.0, 100.0));
    second_path.close_path();
    let second = VectorNetwork::from_path(&second_path);
    {
        let doc = s.doc_mut().unwrap();
        let NodeKind::Vector { network } = &mut doc.get_mut(id).unwrap().kind else { panic!("expected vector fixture") };
        network.regions[0].winding = WindingRule::EvenOdd;
        let vertex_offset = network.vertices.len();
        let segment_offset = network.segments.len();
        network.vertices.extend(second.vertices);
        network.segments.extend(second.segments.into_iter().map(|segment| Segment {
            a: segment.a + vertex_offset,
            b: segment.b + vertex_offset,
            ..segment
        }));
        network.regions.extend(second.regions.into_iter().map(|region| Region {
            loops: region.loops.into_iter().map(|lp| lp.into_iter().map(|segment| segment + segment_offset).collect()).collect(),
            ..region
        }));
        network.regions[1].winding = WindingRule::NonZero;
    }
    ex(&mut s, "vector.simplify", json!({"ids": [id]}));
    let simplified = net(&s, id);
    assert_eq!(simplified.regions.len(), 1, "legacy regions simplify as one combined fill");
    assert!(!simplified.fill_contains(Point::new(60.0, 10.0)), "overlap cancels under combined even-odd winding");
    assert!(!simplified.fill_contains(Point::new(40.0, 40.0)), "nested contour remains a hole");
    assert!(simplified.fill_contains(Point::new(120.0, 50.0)), "non-overlapping area remains filled");
    assert!(simplified.regions[0].fills.is_none());
    assert!(simplified.regions[0].paint_space.is_none());
}

#[test]
fn legacy_evenodd_holes_survive_bucket_offset_and_flatten() {
    let mut s = Session::new();
    let id = id_of(&ex(&mut s, "vector.create", json!({"path": "M0 0 L200 0 L200 200 L0 200 Z M50 50 L150 50 L150 150 L50 150 Z"}))["id"]).unwrap();
    {
        let doc = s.doc_mut().unwrap();
        let NodeKind::Vector { network } = &mut doc.get_mut(id).unwrap().kind else { panic!("expected vector fixture") };
        network.regions[0].winding = figcraft_doc::WindingRule::EvenOdd;
    }
    assert!(!net(&s, id).fill_contains(Point::new(100.0, 100.0)), "center starts as an even-odd hole");
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    ex(&mut s, "vector.edit.paint", json!({"x": 100, "y": 100}));
    assert!(net(&s, id).fill_contains(Point::new(100.0, 100.0)), "bucket fills a legacy even-odd hole");
    s.undo().unwrap();
    ex(&mut s, "vector.edit.end", json!({}));

    let flatten_source =
        id_of(&ex(&mut s, "vector.create", json!({"path": "M0 0 L200 0 L200 200 L0 200 Z M50 50 L150 50 L150 150 L50 150 Z"}))["id"]).unwrap();
    {
        let doc = s.doc_mut().unwrap();
        if let NodeKind::Vector { network } = &mut doc.get_mut(flatten_source).unwrap().kind {
            network.regions[0].winding = figcraft_doc::WindingRule::EvenOdd;
        }
    }
    let group = id_of(&ex(&mut s, "object.group", json!({"ids": [flatten_source]}))["id"]).unwrap();
    let flattened_legacy = id_of(&ex(&mut s, "vector.flatten", json!({"ids": [group]}))["id"]).unwrap();
    assert_eq!(net(&s, flattened_legacy).regions.len(), 1, "flatten combines unstyled legacy loops");
    assert!(!net(&s, flattened_legacy).fill_contains(Point::new(100.0, 100.0)), "flatten preserves the legacy even-odd hole");

    ex(&mut s, "vector.offset", json!({"ids": [id], "offset": 2}));
    assert!(!net(&s, id).fill_contains(Point::new(100.0, 100.0)), "offset retains the even-odd hole");
    let group = id_of(&ex(&mut s, "object.group", json!({"ids": [id]}))["id"]).unwrap();
    let flattened = id_of(&ex(&mut s, "vector.flatten", json!({"ids": [group]}))["id"]).unwrap();
    assert_eq!(net(&s, flattened).regions.len(), 1, "legacy region loops stay grouped");
    assert!(!net(&s, flattened).fill_contains(Point::new(100.0, 100.0)), "flatten retains the even-odd hole");
}

/// Give vector `id` a painted region, converting a plain shape to a vector first.
fn ex_to_vector(doc: &mut figcraft_doc::Document, id: NodeId) {
    let n = doc.get(id).unwrap();
    let (path, _) = figcraft_render::node_outline(doc, n).unwrap();
    let mut network = VectorNetwork::from_path(&path);
    network.regions[0].fills = Some(vec![figcraft_color::Paint::solid(figcraft_color::Color::BLACK)]);
    doc.get_mut(id).unwrap().kind = NodeKind::Vector { network };
}

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn net(s: &Session, id: NodeId) -> VectorNetwork {
    network(s.doc().unwrap(), id).unwrap().clone()
}

fn editing(s: &Session) -> Option<VectorEdit> {
    s.ds().unwrap().vector_edit.clone()
}

#[derive(Clone, Copy, Default)]
struct Mods {
    shift: bool,
    alt: bool,
    cmd: bool,
}

const NONE: Mods = Mods { shift: false, alt: false, cmd: false };
const SHIFT: Mods = Mods { shift: true, alt: false, cmd: false };

fn ev(kind: PointerKind, p: (f64, f64), m: Mods) -> PointerEvent {
    let mut e = PointerEvent::at(kind, p.0, p.1, 1.0);
    (e.shift, e.alt, e.cmd) = (m.shift, m.alt, m.cmd);
    e
}

fn click(s: &mut Session, p: (f64, f64), m: Mods) {
    s.pointer(&ev(PointerKind::Down, p, m)).unwrap();
    s.pointer(&ev(PointerKind::Up, p, m)).unwrap();
}

fn drag(s: &mut Session, a: (f64, f64), b: (f64, f64), m: Mods) {
    s.pointer(&ev(PointerKind::Down, a, m)).unwrap();
    s.pointer(&ev(PointerKind::Move, ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0), m)).unwrap();
    s.pointer(&ev(PointerKind::Move, b, m)).unwrap();
    s.pointer(&ev(PointerKind::Up, b, m)).unwrap();
}

fn key(s: &mut Session, name: &str, shift: bool) -> bool {
    s.tool_key(name, shift, false, false).unwrap()
}

/// A 100 × 100 rectangle at the origin, selected (and in edit mode when `edit`).
fn square(s: &mut Session, edit: bool) -> NodeId {
    let id = id_of(&ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 100}))["id"]).unwrap();
    if edit {
        ex(s, "vector.edit.begin", json!({"id": id}));
    }
    id
}

/// The point at page position `p` of vector `id`.
fn point_at(s: &Session, id: NodeId, p: (f64, f64)) -> usize {
    let w = s.doc().unwrap().world_transform(id);
    net(s, id).vertices.iter().position(|v| (w * v.point() - Point::new(p.0, p.1)).hypot() < 1e-6).unwrap()
}

/// A point's page position.
fn page_pos(s: &Session, id: NodeId, v: usize) -> Point {
    s.doc().unwrap().world_transform(id) * net(s, id).vertices[v].point()
}

/// Whether page point `p` is inside vector `id`'s fill.
fn filled(s: &Session, id: NodeId, p: (f64, f64)) -> bool {
    net(s, id).fill_contains(s.doc().unwrap().world_transform(id).inverse() * Point::new(p.0, p.1))
}

#[test]
fn entering_and_leaving_vector_edit_mode() {
    // (how, in edit mode after, derivation)
    let rows: Vec<(&str, bool, &str)> = vec![
        ("double-click", true, "double-clicking a selected shape edits it as a vector"),
        ("Enter", false, "Enter leaves edit mode"),
        ("Escape", false, "Escape leaves edit mode"),
        ("Escape with an open pen path", true, "Escape first ends the pen's path and stays in edit mode"),
        ("click outside", true, "silent: a click away from the shape only clears the points (pinned)"),
        ("double-click outside", false, "a double-click away from the shape leaves edit mode"),
    ];
    for (how, after, why) in rows {
        let mut s = Session::new();
        let id = square(&mut s, how != "double-click");
        match how {
            "double-click" => {
                s.set_selection(vec![id]);
                s.pointer(&ev(PointerKind::DoubleClick, (50.0, 50.0), NONE)).unwrap();
            }
            "Enter" | "Escape" => {
                key(&mut s, how, false);
            }
            "Escape with an open pen path" => {
                ex(&mut s, "vector.edit.tool", json!({"tool": "pen"}));
                click(&mut s, (300.0, 300.0), NONE);
                key(&mut s, "Escape", false);
                assert!(editing(&s).is_some_and(|e| e.anchor.is_none()), "{why}: the path ended");
            }
            "double-click outside" => s.pointer(&ev(PointerKind::DoubleClick, (600.0, 600.0), NONE)).unwrap(),
            _ => click(&mut s, (600.0, 600.0), NONE),
        }
        assert_eq!(editing(&s).is_some(), after, "{why}");
    }
    // Entering converts the shape to a vector that looks the same.
    let mut s = Session::new();
    let id = square(&mut s, false);
    ex(&mut s, "node.set", json!({"ids": [id], "props": {"cornerRadius": 20, "fill": "#336699", "stroke": "#000000"}}));
    let before = s.doc().unwrap().node(id).unwrap().clone();
    let outline = figcraft_doc::geometry::shape_path(&before).unwrap();
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    let after = s.doc().unwrap().node(id).unwrap().clone();
    assert!(matches!(after.kind, NodeKind::Vector { .. }), "the shape is a vector now");
    assert_eq!((after.fills.clone(), after.strokes.clone()), (before.fills, before.strokes), "fills and strokes are kept");
    let (path, _) = figcraft_render::node_outline(s.doc().unwrap(), &after).unwrap();
    assert!((path.area().abs() - outline.area().abs()).abs() < 0.5, "the outline is kept, rounded corners included");
    s.undo().unwrap();
    assert!(matches!(s.doc().unwrap().node(id).unwrap().kind, NodeKind::Rectangle), "converting is one undo step");
}

#[test]
fn thin_strokes_are_easy_to_enter_and_bend() {
    use crate::tools::ToolId;
    // A diagonal line from (100, 100) to (200, 50); `off` steps away from its middle, square to it.
    let at = |off: f64| (150.0 + off * 50.0 / 125f64.sqrt() / 10.0, 75.0 + off * 100.0 / 125f64.sqrt() / 10.0);
    // (layer, screen px off the stroke, in edit mode after a double-click, derivation)
    let rows: Vec<(&str, f64, bool, &str)> = vec![
        ("line", 0.0, true, "double-clicking a selected line edits it as a vector"),
        ("line", 4.0, true, "a 1 px line reaches a few pixels, so a double-click near it still lands (#646)"),
        ("line", 8.0, false, "a double-click well away from it does not"),
        ("path", 4.0, true, "a stroke-only path reaches as far as a line does"),
        ("path", 8.0, false, "and no further"),
    ];
    for (layer, off, after, why) in rows {
        let mut s = Session::new();
        let id = if layer == "line" {
            s.set_tool(ToolId::Line);
            drag(&mut s, (100.0, 100.0), (200.0, 50.0), NONE);
            *s.selection().first().unwrap()
        } else {
            let id = id_of(&ex(&mut s, "vector.create", json!({"points": [[100, 100], [200, 50]], "closed": false}))["id"]).unwrap();
            // Styled like a pen stroke: 1 px, no fill.
            ex(&mut s, "node.set", json!({"ids": [id], "props": {"fills": [], "strokes": ["#000000"], "strokeWeight": 1}}));
            s.set_selection(vec![id]);
            id
        };
        assert!(!s.doc().unwrap().node(id).unwrap().fills.iter().any(|f| f.visible), "{why}: the {layer} only has a stroke");
        let p = at(off);
        click(&mut s, p, NONE);
        s.pointer(&ev(PointerKind::Down, p, NONE)).unwrap();
        s.pointer(&ev(PointerKind::DoubleClick, p, NONE)).unwrap();
        s.pointer(&ev(PointerKind::Up, p, NONE)).unwrap();
        assert_eq!(editing(&s).is_some(), after, "{why}");
        if after {
            // Cmd/Ctrl then bends it through the pointer.
            drag(&mut s, at(0.0), (170.0, 115.0), Mods { cmd: true, ..NONE });
            let w = s.doc().unwrap().world_transform(id);
            let n = net(&s, id);
            assert!(!n.segments[0].is_straight(), "{why}: Cmd/Ctrl bends it");
            assert!((w * n.cubic(0).unwrap().eval(0.5) - Point::new(170.0, 115.0)).hypot() < 1.0, "{why}: through the pointer");
        }
    }
}

#[test]
fn the_pen_follows_the_documented_rules() {
    let mut s = Session::new();
    let id = id_of(&ex(&mut s, "vector.create", json!({"points": [[0, 0], [100, 0]], "closed": false}))["id"]).unwrap();
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    ex(&mut s, "vector.edit.tool", json!({"tool": "pen"}));
    // Clicking an end point continues the path from it.
    click(&mut s, (100.0, 0.0), NONE);
    assert!(editing(&s).unwrap().anchor.is_some(), "clicking an end point continues from it");
    click(&mut s, (100.0, 100.0), NONE);
    let n = net(&s, id);
    assert!(n.segments.iter().all(|sg| sg.is_straight()), "a click adds a corner point and a straight segment");
    // Dragging adds a smooth point: its handles mirror angle and length.
    drag(&mut s, (0.0, 100.0), (0.0, 140.0), NONE);
    let v = point_at(&s, id, (0.0, 100.0));
    assert_eq!(net(&s, id).mirroring(v), HandleMirroring::AngleAndLength, "dragging sets the curve's length and slope with mirrored handles");
    assert!(net(&s, id).segments.iter().any(|sg| !sg.is_straight()), "the segment into it curves");
    // Clicking another point of the network closes the path there, and the loop fills.
    click(&mut s, (0.0, 0.0), NONE);
    let n = net(&s, id);
    assert_eq!(n.regions.len(), 1, "closing a path makes a filled area");
    assert!(editing(&s).unwrap().anchor.is_none(), "closing ends the path");
    // A click on a middle point branches from it (a vector network, not a single path).
    click(&mut s, (100.0, 100.0), NONE);
    click(&mut s, (200.0, 200.0), NONE);
    let corner = point_at(&s, id, (100.0, 100.0));
    assert_eq!(net(&s, id).incident(corner).len(), 3, "silent: branching from a middle point (pinned)");
    // Shift keeps the new segment to 45° steps.
    let mut sh = ev(PointerKind::Down, (300.0, 210.0), SHIFT);
    s.pointer(&sh).unwrap();
    sh.kind = PointerKind::Up;
    s.pointer(&sh).unwrap();
    let last = net(&s, id).vertices.len() - 1;
    let p = page_pos(&s, id, last);
    assert!((p.y - 200.0).abs() < 1e-6, "silent: Shift snaps to 45° from the last point (pinned): {p:?}");
    // Escape leaves the path open: no new fill.
    key(&mut s, "Escape", false);
    assert_eq!(net(&s, id).regions.len(), 1, "an open path gets no fill region");
}

#[test]
fn selecting_points_and_segments() {
    let mut s = Session::new();
    let id = square(&mut s, true);
    let tl = point_at(&s, id, (0.0, 0.0));
    let tr = point_at(&s, id, (100.0, 0.0));
    // (gesture, modifiers, points selected after, segments selected after, derivation)
    let steps: Vec<(&str, Mods, Vec<usize>, usize, &str)> = vec![
        ("click tl", NONE, vec![tl], 0, "a click selects the point"),
        ("click tr", SHIFT, vec![tl, tr], 0, "Shift-click adds a point"),
        ("click tl", SHIFT, vec![tr], 0, "Shift-click on a selected point removes it"),
        ("click top", NONE, vec![], 1, "a click on a segment selects the segment"),
        ("marquee top", NONE, vec![tl, tr], 0, "a marquee selects the points inside it"),
        ("lasso left", SHIFT, vec![tl, tr, 99], 0, "Shift with the lasso adds the points it circles"),
    ];
    for (what, m, mut want, segs, why) in steps {
        match what {
            "click tl" => click(&mut s, (0.0, 0.0), m),
            "click tr" => click(&mut s, (100.0, 0.0), m),
            "click top" => click(&mut s, (30.0, 0.0), m),
            "marquee top" => drag(&mut s, (-20.0, -20.0), (110.0, 10.0), m),
            _ => {
                ex(&mut s, "vector.edit.tool", json!({"tool": "lasso"}));
                s.pointer(&ev(PointerKind::Down, (-10.0, 90.0), m)).unwrap();
                for p in [(10.0, 90.0), (10.0, 110.0), (-10.0, 110.0)] {
                    s.pointer(&ev(PointerKind::Move, p, m)).unwrap();
                }
                s.pointer(&ev(PointerKind::Up, (-10.0, 110.0), m)).unwrap();
                want = vec![tl, tr, point_at(&s, id, (0.0, 100.0))];
            }
        }
        let e = editing(&s).unwrap();
        let mut got = e.vertices.clone();
        got.sort_unstable();
        want.sort_unstable();
        assert_eq!((got, e.segments.len()), (want, segs), "{why}");
    }
    // Points nudge by 1 with the arrows, 10 with Shift, one undo step each.
    ex(&mut s, "vector.edit.tool", json!({"tool": "move"}));
    ex(&mut s, "vector.edit.select", json!({"vertices": [tl]}));
    key(&mut s, "Right", false);
    assert_eq!(page_pos(&s, id, tl), Point::new(1.0, 0.0), "an arrow nudges by 1");
    key(&mut s, "Down", true);
    assert_eq!(page_pos(&s, id, tl), Point::new(1.0, 10.0), "Shift nudges by 10");
    s.undo().unwrap();
    assert_eq!(page_pos(&s, id, tl), Point::new(1.0, 0.0), "each nudge is one undo step");
    // Dragging a point is one undo step.
    drag(&mut s, (1.0, 0.0), (-30.0, -20.0), NONE);
    assert_eq!(page_pos(&s, id, tl), Point::new(-30.0, -20.0), "a dragged point follows the pointer");
    s.undo().unwrap();
    assert_eq!(page_pos(&s, id, tl), Point::new(1.0, 0.0), "the drag is one undo step");
}

#[test]
fn a_marquee_starts_anywhere_on_the_canvas() {
    // #1508: a drag far from the edited shape, over another layer, marquees its points; a click
    // there still leaves edit mode and selects that layer.
    let mut s = Session::new();
    let bg = id_of(&ex(&mut s, "shape.rectangle", json!({"x": -500, "y": -500, "width": 1000, "height": 1000}))["id"]).unwrap();
    let id = square(&mut s, true);
    let (tl, tr) = (point_at(&s, id, (0.0, 0.0)), point_at(&s, id, (100.0, 0.0)));
    let bg_x = |s: &Session| s.doc().unwrap().world_transform(bg).translation().x;
    drag(&mut s, (-200.0, -200.0), (110.0, 10.0), NONE);
    let mut got = editing(&s).expect("a drag away from the shape stays in edit mode").vertices;
    got.sort_unstable();
    let mut want = vec![tl, tr];
    want.sort_unstable();
    assert_eq!(got, want, "the marquee selects the points inside it");
    assert_eq!(bg_x(&s), -500.0, "the layer under the drag stays put");
    click(&mut s, (-200.0, -200.0), NONE);
    assert!(editing(&s).is_none(), "a click away from the shape leaves edit mode");
    assert_eq!(s.selection(), vec![bg], "the click selects the layer under it");
}

#[test]
fn deleting_points_vs_delete_and_heal() {
    // (key, Shift, points, segments, closed afterwards, derivation)
    let rows: Vec<(&str, bool, usize, usize, bool, &str)> = vec![
        ("Delete", false, 3, 2, false, "Delete removes the point and the segments it joined: the path opens"),
        ("Backspace", false, 3, 2, false, "Backspace deletes too"),
        ("Delete", true, 3, 3, true, "Shift+Delete removes the point and heals the path between its neighbours"),
    ];
    for (k, shift, points, segments, closed, why) in rows {
        let mut s = Session::new();
        let id = square(&mut s, true);
        let tr = point_at(&s, id, (100.0, 0.0));
        ex(&mut s, "vector.edit.select", json!({"vertices": [tr]}));
        key(&mut s, k, shift);
        let n = net(&s, id);
        let ends = (0..n.vertices.len()).filter(|v| n.incident(*v).len() == 1).count();
        assert_eq!((n.vertices.len(), n.segments.len(), ends == 0), (points, segments, closed), "{why}");
        s.undo().unwrap();
        assert_eq!(net(&s, id).vertices.len(), 4, "{why}: one undo step");
    }
    // Healing over a curve recomputes it so it still runs through where the point was (it kept
    // the neighbours' quarter-circle handles and cut inside).
    let mut s = Session::new();
    let circle = "M50 0 C77.6 0 100 22.4 100 50 C100 77.6 77.6 100 50 100 C22.4 100 0 77.6 0 50 C0 22.4 22.4 0 50 0 Z";
    let id = id_of(&ex(&mut s, "vector.create", json!({"path": circle}))["id"]).unwrap();
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    let top = point_at(&s, id, (50.0, 0.0));
    ex(&mut s, "vector.deleteHeal", json!({"vertices": [top]}));
    let n = net(&s, id);
    let w = s.doc().unwrap().world_transform(id);
    let healed = n.segments.len() - 1;
    let c = n.cubic(healed).unwrap();
    let nearest = (0..=100).map(|i| (w * c.eval(f64::from(i) / 100.0) - Point::new(50.0, 0.0)).hypot()).fold(f64::MAX, f64::min);
    assert!(nearest < 0.5, "the healed curve passes where the deleted point was: {nearest}");
    // The side points keep their tangents (straight up), so the curve stays smooth there.
    let (h0, h1) = (c.p1 - c.p0, c.p2 - c.p3);
    assert!(h0.x.abs() < 1e-6 && h0.y < 0.0 && h1.x.abs() < 1e-6 && h1.y < 0.0, "the ends keep their tangents: {h0:?} {h1:?}");
    // Deleting a selected segment keeps its points when others use them.
    let mut s = Session::new();
    let id = square(&mut s, true);
    click(&mut s, (30.0, 0.0), NONE);
    key(&mut s, "Delete", false);
    let n = net(&s, id);
    assert_eq!((n.vertices.len(), n.segments.len()), (4, 3), "deleting a segment leaves its points");
}

#[test]
fn handles_follow_their_mirroring() {
    // A smooth middle point at (100, 0) on an open path; one handle is dragged to (+30, 10).
    // (mode, the other handle afterwards, derivation)
    let rows: Vec<(&str, (f64, f64), &str)> = vec![
        ("none", (-20.0, 0.0), "no mirroring: the other handle stays"),
        (
            "angle",
            (-20.0 * 30.0 / 10f64.hypot(30.0), -20.0 * 10.0 / 10f64.hypot(30.0)),
            "mirror angle: the other turns opposite and keeps its length",
        ),
        ("angleAndLength", (-30.0, -10.0), "mirror angle and length: the other is the dragged one's mirror image"),
    ];
    for (mode, want, why) in rows {
        let mut s = Session::new();
        let id = id_of(&ex(&mut s, "vector.create", json!({"points": [[0, 0], [100, 0], [200, 0]], "closed": false}))["id"]).unwrap();
        ex(&mut s, "vector.edit.begin", json!({"id": id}));
        let mid = point_at(&s, id, (100.0, 0.0));
        let n = net(&s, id);
        let into = n.incident(mid).iter().find(|(_, at_a)| !at_a).map(|x| x.0).unwrap();
        let out = n.incident(mid).iter().find(|(_, at_a)| *at_a).map(|x| x.0).unwrap();
        ex(&mut s, "vector.edit.setHandle", json!({"segment": into, "start": false, "x": 80, "y": 0, "independent": true}));
        ex(&mut s, "vector.edit.setHandle", json!({"segment": out, "start": true, "x": 120, "y": 0, "independent": true}));
        ex(&mut s, "vector.edit.mirroring", json!({"mode": mode, "vertices": [mid]}));
        let lp = net(&s, id).vertices[mid].point();
        ex(&mut s, "vector.edit.setHandle", json!({"segment": out, "start": true, "x": lp.x + 30.0, "y": lp.y + 10.0}));
        let got = net(&s, id).segments[into].tb;
        assert!((got.0 - want.0).abs() < 1e-6 && (got.1 - want.1).abs() < 1e-6, "{why}: {got:?}");
    }
}

#[test]
fn bend_paint_bucket_cut_join_and_eraser_tools() {
    // Bend: dragging a straight segment curves it through the pointer.
    let mut s = Session::new();
    let id = square(&mut s, true);
    let cmd = Mods { cmd: true, ..NONE };
    drag(&mut s, (50.0, 0.0), (50.0, -30.0), cmd);
    let w = s.doc().unwrap().world_transform(id);
    let n = net(&s, id);
    let top = n.segments.iter().position(|sg| !sg.is_straight()).unwrap();
    assert!(
        (w * n.cubic(top).unwrap().eval(0.5) - Point::new(50.0, -30.0)).hypot() < 1.0,
        "silent: Cmd/Ctrl holds the Bend tool; the curve passes under the pointer (pinned)"
    );

    // Paint bucket: a square split in two by a segment; both halves start filled.
    let mut s = Session::new();
    let id = square(&mut s, true);
    let n = net(&s, id);
    let (topseg, botseg) = (
        n.segments.iter().position(|sg| n.vertices[sg.a].y == 0.0 && n.vertices[sg.b].y == 0.0).unwrap(),
        n.segments.iter().position(|sg| n.vertices[sg.a].y == 100.0 && n.vertices[sg.b].y == 100.0).unwrap(),
    );
    let a = ex(&mut s, "vector.edit.addVertex", json!({"segment": topseg, "t": 0.5}))["vertex"].as_u64().unwrap();
    let b = ex(&mut s, "vector.edit.addVertex", json!({"segment": botseg, "t": 0.5}))["vertex"].as_u64().unwrap();
    ex(&mut s, "vector.edit.connect", json!({"a": a, "b": b}));
    ex(&mut s, "vector.edit.tool", json!({"tool": "paint"}));
    // (gesture, left filled, right filled, derivation)
    let steps: Vec<(&str, bool, bool, &str)> = vec![
        ("click left", false, true, "a click on a filled area removes its fill"),
        ("click left", true, true, "a click on an empty area fills it"),
        ("drag left to right", false, false, "a drag changes every area it crosses the way it changed the first"),
        ("drag right to left", true, true, "starting on an empty area, a drag fills them all"),
    ];
    for (what, left, right, why) in steps {
        match what {
            "click left" => click(&mut s, (25.0, 50.0), NONE),
            "drag left to right" => drag(&mut s, (25.0, 50.0), (75.0, 50.0), NONE),
            _ => drag(&mut s, (75.0, 50.0), (25.0, 50.0), NONE),
        }
        assert_eq!((filled(&s, id, (25.0, 50.0)), filled(&s, id, (75.0, 50.0))), (left, right), "{why}");
    }
    s.undo().unwrap();
    assert_eq!((filled(&s, id, (25.0, 50.0)), filled(&s, id, (75.0, 50.0))), (false, false), "a paint drag is one undo step");
    let r = ex(&mut s, "vector.edit.paint", json!({"points": [[25, 50], [75, 50]]}));
    assert_eq!(r["filled"], json!(true), "the command paints a drag's points the same way");

    // Cut: a click on a segment splits it into two ends at the same spot, keeping the fill.
    let mut s = Session::new();
    let id = square(&mut s, true);
    ex(&mut s, "vector.edit.tool", json!({"tool": "cut"}));
    click(&mut s, (50.0, 0.0), NONE);
    let e = editing(&s).unwrap();
    assert_eq!(e.vertices.len(), 2, "a click splits the path into two ends");
    assert_eq!(page_pos(&s, id, e.vertices[0]), page_pos(&s, id, e.vertices[1]), "the ends sit where the click was");
    assert!(filled(&s, id, (50.0, 50.0)), "silent: the fill stays (pinned)");
    // A drag across divides the shape; the part that comes apart moves to its own layer.
    let mut s = Session::new();
    let id = square(&mut s, true);
    let page = s.page().unwrap();
    ex(&mut s, "vector.edit.tool", json!({"tool": "cut"}));
    drag(&mut s, (-20.0, 40.0), (120.0, 40.0), NONE);
    let layers = s.doc().unwrap().node(page).unwrap().children.clone();
    assert_eq!(layers.len(), 2, "dragging across divides the shape into two layers");
    let other = *layers.iter().find(|l| **l != id).unwrap();
    let parts: Vec<(bool, bool)> = [id, other].iter().map(|l| (filled(&s, *l, (50.0, 20.0)), filled(&s, *l, (50.0, 70.0)))).collect();
    assert!(parts.contains(&(true, false)) && parts.contains(&(false, true)), "each part keeps its piece of the fill: {parts:?}");
    assert!(editing(&s).is_some(), "still editing");
    s.undo().unwrap();
    assert_eq!(s.doc().unwrap().node(page).unwrap().children, vec![id], "the cut is one undo step");
    assert!(s.execute("vector.edit.cut", &json!({"from": [300, 300], "to": [400, 300]})).is_err(), "a line crossing nothing cuts nothing");

    // Join: two points on top of each other merge into one.
    let mut s = Session::new();
    let id = id_of(&ex(&mut s, "vector.create", json!({"path": "M0 0 L50 0 M50 0 L100 0"}))["id"]).unwrap();
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    let before = net(&s, id).vertices.len();
    ex(&mut s, "vector.edit.selectAll", json!({}));
    let mids: Vec<usize> = (0..before).filter(|v| (page_pos(&s, id, *v) - Point::new(50.0, 0.0)).hypot() < 1e-6).collect();
    ex(&mut s, "vector.edit.select", json!({"vertices": mids}));
    assert!(s.tool_key("J", false, false, true).unwrap());
    assert_eq!(net(&s, id).vertices.len(), before - 1, "silent: Cmd/Ctrl+J merges overlapping points (pinned)");

    // Eraser: erasing through a path keeps one layer.
    let mut s = Session::new();
    let id = id_of(&ex(&mut s, "vector.create", json!({"points": [[0, 0], [100, 0]], "closed": false}))["id"]).unwrap();
    let page = s.page().unwrap();
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    ex(&mut s, "vector.edit.erase", json!({"points": [[50, 0]], "radius": 10}));
    assert_eq!(net(&s, id).segments.len(), 2, "the eraser removes what it passes over");
    assert_eq!(s.doc().unwrap().node(page).unwrap().children, vec![id], "and the pieces stay one layer");
}

#[test]
fn vector_paths_take_the_requested_size_and_refuse_empty_geometry() {
    let mut s = Session::new();
    let path = "M0 0 L24 0 L24 24 L0 24 Z";
    let intrinsic = id_of(&ex(&mut s, "vector.create", json!({"path": path}))["id"]).unwrap();
    let n = s.doc().unwrap().node(intrinsic).unwrap();
    assert_eq!((n.width, n.height), (24.0, 24.0), "a path without a size keeps its intrinsic box");
    // Both sides: each scales independently, like resizing a vector.
    let both = id_of(&ex(&mut s, "vector.create", json!({"path": path, "width": 16, "height": 8}))["id"]).unwrap();
    let n = s.doc().unwrap().node(both).unwrap();
    assert_eq!((n.width, n.height), (16.0, 8.0));
    let n = net(&s, both);
    let max = n.vertices.iter().fold((0.0f64, 0.0f64), |(x, y), v| (x.max(v.x), y.max(v.y)));
    assert!((max.0 - 16.0).abs() < 1e-9 && (max.1 - 8.0).abs() < 1e-9, "{max:?}: the geometry itself is scaled");
    // One side: the other keeps its proportion.
    let width = id_of(&ex(&mut s, "vector.create", json!({"path": path, "width": 16}))["id"]).unwrap();
    let n = s.doc().unwrap().node(width).unwrap();
    assert_eq!((n.width, n.height), (16.0, 16.0));
    // A path or point set that draws nothing is an error, not an invisible zero-size layer.
    let e = s.execute("vector.create", &json!({"path": "M5 5"})).unwrap_err().to_string();
    assert!(e.contains("draws nothing"), "{e}");
    let e = s.execute("vector.create", &json!({"points": [[5, 5], [5, 5]]})).unwrap_err().to_string();
    assert!(e.contains("draws nothing"), "{e}");
    // A size on an axis the geometry has none of is refused, not silently ignored.
    let e = s.execute("vector.create", &json!({"path": "M0 0 L0 24", "width": 16})).unwrap_err().to_string();
    assert!(e.contains("horizontal extent"), "{e}");
    let e = s.execute("vector.create", &json!({"path": "M0 0 L24 0", "height": 16})).unwrap_err().to_string();
    assert!(e.contains("vertical extent"), "{e}");
    // But 0 on an axis it has none of is what it already is: a horizontal line with height 0.
    let line = id_of(&ex(&mut s, "vector.create", json!({"path": "M0 0 L24 0", "width": 16, "height": 0}))["id"]).unwrap();
    let n = s.doc().unwrap().node(line).unwrap();
    assert_eq!((n.width, n.height), (16.0, 0.0));
    // A size of 0 or less on an axis it has would flatten it, so it is refused, alone or with the other.
    for p in [json!({"path": path, "width": 0}), json!({"path": path, "width": 16, "height": 0}), json!({"path": path, "width": -4, "height": 8})] {
        let e = s.execute("vector.create", &p).unwrap_err().to_string();
        assert!(e.contains("positive"), "{p}: {e}");
    }
}

#[test]
fn vector_points_take_the_requested_size() {
    let mut s = Session::new();
    let points = json!([[10, 20], [34, 20], [34, 32]]);
    let corners = |s: &Session, id: NodeId| net(s, id).vertices.iter().map(|v| (v.x, v.y)).collect::<Vec<_>>();
    // Both sides: each axis scales on its own; the vertices move with the box.
    let both = id_of(&ex(&mut s, "vector.create", json!({"points": points, "x": 10, "y": 20, "width": 48, "height": 6}))["id"]).unwrap();
    let n = s.doc().unwrap().node(both).unwrap();
    assert_eq!((n.width, n.height), (48.0, 6.0));
    assert_eq!(corners(&s, both), vec![(0.0, 0.0), (48.0, 0.0), (48.0, 6.0)]);
    // One side: the other keeps its proportion.
    let one = id_of(&ex(&mut s, "vector.create", json!({"points": points, "height": 24}))["id"]).unwrap();
    let n = s.doc().unwrap().node(one).unwrap();
    assert_eq!((n.width, n.height), (48.0, 24.0));
    assert_eq!(corners(&s, one), vec![(0.0, 0.0), (48.0, 0.0), (48.0, 24.0)]);
    let e = s.execute("vector.create", &json!({"points": points, "width": 16, "height": -1})).unwrap_err().to_string();
    assert!(e.contains("positive"), "{e}");
}
