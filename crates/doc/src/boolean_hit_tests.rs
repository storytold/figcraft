//! Clicking into boolean groups: a hit reaches the operand under the pointer, a plain click still
//! picks the group, and a double click goes one level deeper than the selection.

use crate::geometry::{enter_target, select_target};
use crate::{BoolOp, Document, NodeId, NodeKind, Region, Segment, VectorNetwork, Vertex, WindingRule};
use figcraft_geom::Point;

fn page(d: &Document) -> NodeId {
    d.first_page().unwrap()
}

/// A frame holding a group holding a boolean group of two overlapping squares.
fn setup() -> (Document, NodeId, NodeId, NodeId, NodeId, NodeId) {
    let mut d = Document::new("x");
    let p = page(&d);
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 300.0, 300.0).unwrap();
    let g = d.create(f, NodeKind::Group, "G", 0.0, 0.0, 0.0, 0.0).unwrap();
    let b = d.create(g, NodeKind::BooleanOp { op: BoolOp::Subtract }, "Subtract", 0.0, 0.0, 0.0, 0.0).unwrap();
    let r1 = d.create(b, NodeKind::Rectangle, "R1", 10.0, 10.0, 100.0, 100.0).unwrap();
    let r2 = d.create(b, NodeKind::Rectangle, "R2", 60.0, 60.0, 100.0, 100.0).unwrap();
    d.fit_to_children(b).unwrap();
    d.fit_to_children(g).unwrap();
    (d, f, g, b, r1, r2)
}

#[test]
fn hits_reach_operands_but_clicks_pick_the_group() {
    let (d, f, g, b, r1, r2) = setup();
    let p = page(&d);
    assert_eq!(d.hit_test(p, Point::new(20.0, 20.0), 0.0, None), Some(r1));
    assert_eq!(d.hit_test(p, Point::new(100.0, 100.0), 0.0, None), Some(r2), "the top operand where they overlap");
    // A plain click: the group in the frame; Cmd-click: the operand.
    assert_eq!(select_target(&d, r1, &[], false), g);
    assert_eq!(select_target(&d, r1, &[], true), r1);
    // With the boolean group selected, its siblings resolve as usual; with an operand selected, a
    // click on another operand picks that one.
    assert_eq!(select_target(&d, r2, &[r1], false), r2);
    // "Select layer" still lists the boolean group as one layer.
    let listed: Vec<NodeId> = crate::geometry::layers_at(&d, p, Point::new(20.0, 20.0), 0.0, None).into_iter().map(|(id, _)| id).collect();
    assert_eq!(listed, vec![f, g, b]);
}

#[test]
fn ordinary_hit_uses_region_paints_without_layer_fill() {
    let mut d = Document::new("region hit");
    let p = page(&d);
    let network = VectorNetwork {
        vertices: vec![Vertex::new(0.0, 0.0), Vertex::new(10.0, 0.0), Vertex::new(10.0, 10.0), Vertex::new(0.0, 10.0)],
        segments: (0..4).map(|i| Segment::line(i, (i + 1) % 4)).collect(),
        regions: vec![
            Region {
                loops: vec![vec![0, 1, 2, 3]],
                winding: WindingRule::NonZero,
                fills: Some(vec![figcraft_color::Paint::solid(figcraft_color::Color::BLACK)]),
                ..Default::default()
            },
            Region { loops: vec![vec![0, 1, 2, 3]], winding: WindingRule::NonZero, fills: Some(Vec::new()), ..Default::default() },
        ],
    };
    let id = d.create(p, NodeKind::Vector { network }, "V", 0.0, 0.0, 10.0, 10.0).unwrap();
    d.get_mut(id).unwrap().fills.clear();
    assert!(d.node(id).unwrap().fills.is_empty());
    assert_eq!(d.hit_test(p, Point::new(5.0, 5.0), 0.0, None), Some(id));
    assert_eq!(d.hit_test(p, Point::new(15.0, 5.0), 0.0, None), None);
}

#[test]
fn inherited_paint_frames_use_per_region_hit_testing() {
    let mut d = Document::new("inherited region hit");
    let p = page(&d);
    let mut network = VectorNetwork::from_points(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 10.0)], true);
    network.regions[0].winding = WindingRule::EvenOdd;
    network.regions.push(network.regions[0].clone());
    let id = d.create(p, NodeKind::Vector { network }, "V", 0.0, 0.0, 10.0, 10.0).unwrap();
    assert_eq!(d.hit_test(p, Point::new(5.0, 5.0), 0.0, None), None, "legacy even-odd loops cancel");
    let NodeKind::Vector { network } = &mut d.get_mut(id).unwrap().kind else { panic!("expected vector") };
    network.regions[0].paint_space = Some(crate::RegionPaintSpace { width: 10.0, height: 10.0, transform: figcraft_geom::Affine::IDENTITY });
    assert!(network.regions.iter().all(|region| region.fills.is_none()));
    assert_eq!(d.hit_test(p, Point::new(5.0, 5.0), 0.0, None), Some(id), "paint-framed regions inherit the node fill independently");
    assert_eq!(d.hit_test(p, Point::new(15.0, 5.0), 0.0, None), None);
}

#[test]
fn explicitly_unfilled_regions_keep_the_stroke_only_selection_band() {
    let mut d = Document::new("stroke selection");
    let p = page(&d);
    let mut network =
        VectorNetwork::from_points(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 100.0), Point::new(0.0, 100.0)], true);
    network.regions[0].fills = Some(Vec::new());
    let id = d.create(p, NodeKind::Vector { network }, "V", 0.0, 0.0, 100.0, 100.0).unwrap();
    let n = d.get_mut(id).unwrap();
    n.fills.clear();
    n.strokes = vec![figcraft_color::Paint::solid(figcraft_color::Color::BLACK)];
    n.stroke.weight = 1.0;
    assert_eq!(d.hit_test(p, Point::new(50.0, -4.0), 2.0, None), Some(id));
}

#[test]
fn double_clicks_go_one_level_deeper() {
    let (d, f, g, b, r1, _) = setup();
    assert_eq!(enter_target(&d, r1, &[]), g, "nothing selected: the plain click target");
    assert_eq!(enter_target(&d, r1, &[g]), b, "into the group");
    assert_eq!(enter_target(&d, r1, &[b]), r1, "into the boolean group");
    assert_eq!(enter_target(&d, r1, &[r1]), r1, "the leaf itself: nothing deeper, it stays");
    assert_eq!(enter_target(&d, r1, &[f]), g);
    // The deepest selected layer that holds it counts.
    assert_eq!(enter_target(&d, r1, &[f, b]), r1);
}

#[test]
fn a_top_level_group_is_entered_too() {
    let mut d = Document::new("x");
    let p = page(&d);
    let g = d.create(p, NodeKind::Group, "G", 0.0, 0.0, 0.0, 0.0).unwrap();
    let r = d.create(g, NodeKind::Rectangle, "R", 10.0, 10.0, 20.0, 20.0).unwrap();
    d.fit_to_children(g).unwrap();
    assert_eq!(select_target(&d, r, &[g], false), g, "a click keeps the group");
    assert_eq!(enter_target(&d, r, &[g]), r, "a double click enters it");
}
