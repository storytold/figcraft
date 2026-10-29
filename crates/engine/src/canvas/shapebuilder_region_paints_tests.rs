use figcraft_color::{Color, Gradient, GradientKind, Paint};
use figcraft_doc::{NodeId, NodeKind, Region, VectorNetwork, WindingRule};
use figcraft_geom::{Affine, Point, Shape};
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::shapebuilder::arrangement;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn id(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn rect(s: &mut Session, x: f64, y: f64, width: f64, height: f64, fill: &str) -> NodeId {
    id(&ex(s, "shape.rectangle", json!({"x": x, "y": y, "width": width, "height": height, "props": {"fill": fill}})))
}

fn append(into: &mut VectorNetwork, mut part: VectorNetwork) {
    let (vertices, segments) = (into.vertices.len(), into.segments.len());
    for segment in &mut part.segments {
        segment.a += vertices;
        segment.b += vertices;
    }
    for region in &mut part.regions {
        for lp in &mut region.loops {
            for segment in lp {
                *segment += segments;
            }
        }
    }
    into.vertices.extend(part.vertices);
    into.segments.extend(part.segments);
    into.regions.extend(part.regions);
}

fn box_path(x0: f64, y0: f64, x1: f64, y1: f64) -> VectorNetwork {
    VectorNetwork::from_points(&[Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)], true)
}

fn split_vector(s: &mut Session) -> NodeId {
    let id = rect(s, 0.0, 0.0, 100.0, 100.0, "#00ff00");
    let mut network = VectorNetwork::default();
    append(&mut network, box_path(0.0, 0.0, 50.0, 100.0));
    append(&mut network, box_path(50.0, 0.0, 100.0, 100.0));
    network.regions[0].fills = Some(vec![Paint::solid(Color::from_hex("#ff0000").unwrap())]);
    network.regions[1].fills = Some(vec![Paint::solid(Color::from_hex("#0000ff").unwrap())]);
    s.doc_mut().unwrap().get_mut(id).unwrap().kind = NodeKind::Vector { network };
    id
}

fn network(s: &Session, id: NodeId) -> &VectorNetwork {
    let NodeKind::Vector { network } = &s.doc().unwrap().node(id).unwrap().kind else { panic!("expected vector") };
    // This helper's borrow is tied to the session document.
    network
}

fn world_region_area(s: &Session, id: NodeId, index: usize) -> f64 {
    let doc = s.doc().unwrap();
    let mut path = network(s, id).region_path(index).unwrap();
    path.apply_affine(doc.world_transform(id));
    path.area().abs()
}

fn normalized_paint_world(s: &Session, id: NodeId, index: usize) -> Affine {
    let doc = s.doc().unwrap();
    let node = doc.node(id).unwrap();
    let (width, height, transform) = network(s, id).regions[index].paint_frame(node.width, node.height);
    doc.world_transform(id) * transform * Affine::scale_non_uniform(width, height)
}

fn assert_same_transform(a: Affine, b: Affine) {
    for (x, y) in a.as_coeffs().into_iter().zip(b.as_coeffs()) {
        assert!((x - y).abs() < 1e-8, "paint frame changed: {a:?} vs {b:?}");
    }
}

#[test]
fn arrangement_keeps_internal_paint_boundaries_when_another_shape_crosses_them() {
    let mut s = Session::new();
    let vector = split_vector(&mut s);
    let crossing = rect(&mut s, 25.0, -10.0, 50.0, 120.0, "#ffffff");
    s.set_selection(vec![vector, crossing]);
    let a = arrangement(&mut s, &[vector, crossing]).unwrap();
    let left = a.region_at(Point::new(35.0, 50.0)).unwrap();
    let right = a.region_at(Point::new(65.0, 50.0)).unwrap();
    assert_ne!(left, right, "the old paint boundary must split the crossing shape's overlap");
    assert!(a.regions[left].parts.iter().any(|p| p.id == vector && p.region == Some(0)));
    assert!(a.regions[right].parts.iter().any(|p| p.id == vector && p.region == Some(1)));
}

#[test]
fn legacy_unstyled_nested_evenodd_regions_keep_the_hole_unpainted() {
    let mut s = Session::new();
    let vector = rect(&mut s, 0.0, 0.0, 100.0, 100.0, "#00ff00");
    let mut net = box_path(0.0, 0.0, 100.0, 100.0);
    let inner = VectorNetwork::from_points(&[Point::new(25.0, 25.0), Point::new(25.0, 75.0), Point::new(75.0, 75.0), Point::new(75.0, 25.0)], true);
    append(&mut net, inner);
    for region in &mut net.regions {
        region.winding = WindingRule::EvenOdd;
        region.fills = None;
        region.paint_space = None;
    }
    s.doc_mut().unwrap().get_mut(vector).unwrap().kind = NodeKind::Vector { network: net };
    let crossing = rect(&mut s, -5.0, 45.0, 15.0, 10.0, "#ffffff");
    s.set_selection(vec![vector, crossing]);
    let a = arrangement(&mut s, &[vector, crossing]).unwrap();
    assert!(a.region_at(Point::new(50.0, 50.0)).is_none(), "the legacy combined even-odd fill leaves its nested center empty");

    ex(&mut s, "shapeBuilder.remove", json!({"points": [[5, 50]]}));
    let NodeKind::Vector { network } = &s.doc().unwrap().node(vector).unwrap().kind else { panic!("expected vector") };
    assert!(!network.fill_contains(Point::new(50.0, 50.0)), "the nested hole survives removal");
    assert!(network.fill_contains(Point::new(85.0, 50.0)), "unselected outer area remains filled");
}

#[test]
fn merge_all_painted_regions_uses_the_first_picked_region_style() {
    let mut s = Session::new();
    let vector = split_vector(&mut s);
    s.set_selection(vec![vector]);
    let result = ex(&mut s, "shapeBuilder.merge", json!({"points": [[25, 50], [75, 50]]}));
    let merged = id(&result);
    assert!(!s.doc().unwrap().contains(vector));
    assert_eq!(network(&s, merged).regions[0].fills.as_ref().unwrap()[0].solid_color().unwrap().to_hex(), "FF0000");
    assert!((world_region_area(&s, merged, 0) - 10_000.0).abs() < 0.1);
    s.undo().unwrap();
    assert_eq!(network(&s, vector).regions.len(), 2);
}

#[test]
fn merge_all_inherited_gradient_regions_with_distinct_frames_is_not_a_noop() {
    let mut s = Session::new();
    let vector = split_vector(&mut s);
    {
        let doc = s.doc_mut().unwrap();
        let node = doc.get_mut(vector).unwrap();
        node.fills = vec![Paint::gradient(Gradient::default_for(GradientKind::Linear, Color::from_hex("#ff0000").unwrap()))];
        let NodeKind::Vector { network } = &mut node.kind else { panic!("expected vector") };
        for (region, offset) in network.regions.iter_mut().zip([0.0, 12.0]) {
            region.fills = None;
            region.paint_space = Some(figcraft_doc::RegionPaintSpace { width: 100.0, height: 100.0, transform: Affine::translate((offset, 0.0)) });
        }
    }
    let before_a = normalized_paint_world(&s, vector, 0);
    let before_b = normalized_paint_world(&s, vector, 1);
    assert_ne!(before_a, before_b, "source regions use distinct inherited-gradient frames");

    s.set_selection(vec![vector]);
    let result = ex(&mut s, "shapeBuilder.merge", json!({"points": [[25, 50], [75, 50]]}));
    let merged = id(&result);
    assert!(!s.doc().unwrap().contains(vector), "the selected regions are merged into a new vector");
    assert_eq!(network(&s, merged).regions.len(), 1);
    assert_eq!(network(&s, merged).regions[0].fills, None, "node gradient remains inherited");
    assert!(matches!(s.doc().unwrap().node(merged).unwrap().fills[0].kind, figcraft_color::PaintKind::Gradient(_)));
    assert_same_transform(normalized_paint_world(&s, merged, 0), before_a);
}

#[test]
fn exceeding_arrangement_capacity_never_drops_unprocessed_regions() {
    let mut s = Session::new();
    let vector = split_vector(&mut s);
    let mut net = VectorNetwork::default();
    for i in 0..=figcraft_pathops::MAX_ARRANGEMENT_INPUTS {
        let mut part = box_path(i as f64 * 12.0, 0.0, i as f64 * 12.0 + 10.0, 10.0);
        part.regions[0].fills = Some(vec![Paint::solid(Color::BLACK)]);
        append(&mut net, part);
    }
    s.doc_mut().unwrap().get_mut(vector).unwrap().kind = NodeKind::Vector { network: net.clone() };
    s.set_selection(vec![vector]);
    let error = s.execute("shapeBuilder.remove", &json!({"points": [[5, 5]]})).unwrap_err();
    assert!(error.to_string().contains("too many fill regions"));
    assert_eq!(network(&s, vector), &net);
}

#[test]
fn remove_changes_only_the_picked_painted_face() {
    let mut s = Session::new();
    let vector = split_vector(&mut s);
    let crossing = rect(&mut s, 25.0, -10.0, 50.0, 120.0, "#ffffff");
    s.set_selection(vec![vector, crossing]);
    ex(&mut s, "shapeBuilder.remove", json!({"points": [[35, 50]]}));
    assert!((world_region_area(&s, vector, 0) - 2500.0).abs() < 0.1);
    assert!((world_region_area(&s, vector, 1) - 5000.0).abs() < 0.1, "the untouched blue region keeps its area");
    let net = network(&s, vector);
    assert_eq!(net.regions[0].fills.as_ref().unwrap()[0].solid_color().unwrap().to_hex(), "FF0000");
    assert_eq!(net.regions[1].fills.as_ref().unwrap()[0].solid_color().unwrap().to_hex(), "0000FF");
}

#[test]
fn reshape_keeps_inherited_and_explicitly_empty_region_fills_distinct() {
    let mut s = Session::new();
    let vector = split_vector(&mut s);
    {
        let doc = s.doc_mut().unwrap();
        let NodeKind::Vector { network } = &mut doc.get_mut(vector).unwrap().kind else { panic!("expected vector") };
        network.regions[0].fills = Some(Vec::new());
        network.regions[1].fills = None;
    }
    let crossing = rect(&mut s, 25.0, -10.0, 50.0, 120.0, "#ffffff");
    s.set_selection(vec![vector, crossing]);
    ex(&mut s, "shapeBuilder.remove", json!({"points": [[35, 50]]}));
    assert_eq!(network(&s, vector).regions[0].fills, Some(Vec::new()));
    assert_eq!(network(&s, vector).regions[1].fills, None);
    assert_eq!(s.doc().unwrap().node(vector).unwrap().fills[0].solid_color().unwrap().to_hex(), "00FF00");
}

#[test]
fn merge_uses_selected_region_paints_and_retains_other_region_stack() {
    let mut s = Session::new();
    // Create crossing first, then vector above it so vector region supplies merge appearance.
    let crossing = rect(&mut s, 25.0, -10.0, 50.0, 120.0, "#ffffff");
    let vector = split_vector(&mut s);
    s.set_selection(vec![crossing, vector]);
    let result = ex(&mut s, "shapeBuilder.merge", json!({"points": [[35, 50]]}));
    let merged = id(&result);
    let merged_net = network(&s, merged);
    assert_eq!(merged_net.regions[0].fills.as_ref().unwrap()[0].solid_color().unwrap().to_hex(), "FF0000");
    assert!((world_region_area(&s, vector, 1) - 5000.0).abs() < 0.1, "unselected blue region survives");
    assert_eq!(network(&s, vector).regions[1].fills.as_ref().unwrap()[0].solid_color().unwrap().to_hex(), "0000FF");
    assert!((world_region_area(&s, merged, 0) - 2500.0).abs() < 0.1);
    assert!(s.undo().unwrap());
    assert_eq!(network(&s, vector).regions[0].fills.as_ref().unwrap()[0].solid_color().unwrap().to_hex(), "FF0000");
    assert!(s.redo().unwrap());
    assert_eq!(network(&s, merged).regions[0].fills.as_ref().unwrap()[0].solid_color().unwrap().to_hex(), "FF0000");
}

#[test]
fn merged_region_keeps_a_transformed_source_gradient_frame() {
    let mut s = Session::new();
    let crossing = rect(&mut s, 0.0, 0.0, 300.0, 200.0, "#ffffff");
    let vector = split_vector(&mut s);
    let mut transform = Affine::translate((100.0, 20.0)) * Affine::rotate(0.2);
    transform *= Affine::scale_non_uniform(1.2, 0.8);
    {
        let doc = s.doc_mut().unwrap();
        let node = doc.get_mut(vector).unwrap();
        node.transform = transform;
        let NodeKind::Vector { network } = &mut node.kind else { panic!("expected vector") };
        network.regions[0].fills = Some(vec![Paint::gradient(Gradient::default_for(GradientKind::Linear, Color::from_hex("#ff0000").unwrap()))]);
    }
    let before = normalized_paint_world(&s, vector, 0);
    let point = s.doc().unwrap().world_transform(vector) * Point::new(35.0, 50.0);
    s.set_selection(vec![crossing, vector]);
    let result = ex(&mut s, "shapeBuilder.merge", json!({"points": [[point.x, point.y]]}));
    let merged = id(&result);
    assert!(matches!(&network(&s, merged).regions[0].fills.as_ref().unwrap()[0].kind, figcraft_color::PaintKind::Gradient(_)));
    assert_same_transform(before, normalized_paint_world(&s, merged, 0));
}

#[test]
fn removal_preserves_holes_and_transformed_gradient_frames() {
    let mut s = Session::new();
    let vector = rect(&mut s, 0.0, 0.0, 100.0, 100.0, "#00ff00");
    let mut outer = box_path(0.0, 0.0, 100.0, 100.0);
    let inner = VectorNetwork::from_points(&[Point::new(25.0, 25.0), Point::new(25.0, 75.0), Point::new(75.0, 75.0), Point::new(75.0, 25.0)], true);
    let outer_loop = outer.regions[0].loops[0].clone();
    let inner_loop: Vec<usize> = inner.regions[0].loops[0].iter().map(|i| i + outer.segments.len()).collect();
    append(&mut outer, inner);
    outer.regions = vec![Region {
        loops: vec![outer_loop, inner_loop],
        winding: WindingRule::EvenOdd,
        fills: Some(vec![Paint::gradient(Gradient::default_for(GradientKind::Linear, Color::from_hex("#ff0000").unwrap()))]),
        paint_space: None,
    }];
    let transform = Affine::translate((120.0, 20.0)) * Affine::rotate(0.2);
    {
        let doc = s.doc_mut().unwrap();
        let node = doc.get_mut(vector).unwrap();
        node.kind = NodeKind::Vector { network: outer };
        node.transform = transform;
    }
    let before = normalized_paint_world(&s, vector, 0);
    let world = s.doc().unwrap().world_transform(vector) * Point::new(10.0, 50.0);
    let overlay = rect(&mut s, world.x - 5.0, world.y - 5.0, 10.0, 10.0, "#ffffff");
    s.set_selection(vec![overlay, vector]);
    ex(&mut s, "shapeBuilder.remove", json!({"points": [[world.x, world.y]]}));
    let net = network(&s, vector);
    assert_eq!(net.regions.len(), 1);
    assert_eq!(net.regions[0].loops.len(), 3, "the original hole and removed face stay separate loops");
    assert!(!net.region_contains(0, Point::new(50.0, 50.0)), "the hole remains empty");
    assert!(!net.region_contains(0, Point::new(10.0, 50.0)), "the removed face remains empty");
    assert!(net.region_contains(0, Point::new(90.0, 50.0)), "the unselected ring remains filled");
    assert_same_transform(before, normalized_paint_world(&s, vector, 0));
}

#[test]
fn removing_the_left_region_keeps_the_right_region_paint_frame() {
    let mut s = Session::new();
    let vector = split_vector(&mut s);
    {
        let NodeKind::Vector { network } = &mut s.doc_mut().unwrap().get_mut(vector).unwrap().kind else { panic!("expected vector") };
        network.regions[1].fills = Some(vec![Paint::gradient(Gradient::default_for(GradientKind::Linear, Color::from_hex("#0000ff").unwrap()))]);
    }
    let before = normalized_paint_world(&s, vector, 1);
    s.set_selection(vec![vector]);
    ex(&mut s, "shapeBuilder.remove", json!({"points": [[25, 50]]}));
    let node = s.doc().unwrap().node(vector).unwrap();
    assert!((node.width - 50.0).abs() < 1e-6, "the layer is reboxed to the surviving region: {}", node.width);
    let net = network(&s, vector);
    assert_eq!(net.regions.len(), 1);
    assert!(matches!(&net.regions[0].fills.as_ref().unwrap()[0].kind, figcraft_color::PaintKind::Gradient(_)));
    assert!((world_region_area(&s, vector, 0) - 5000.0).abs() < 1e-6);
    assert_same_transform(before, normalized_paint_world(&s, vector, 0));
}
