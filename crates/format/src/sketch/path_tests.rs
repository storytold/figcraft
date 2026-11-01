//! Sketch paths → editable vector networks; shape groups → shapes and boolean operations; stars
//! and polygons.

use figcraft_doc::{BoolOp, Document, NodeKind};
use serde_json::{Value, json};

use super::fixture::*;
use super::import;
use super::path::parse_point;

fn open(layers: Vec<Value>) -> Document {
    let z = sketch_zip("50.2", 103, document(&["P"]), &[page("P", "Page", vec![artboard("A", "Board", 0.0, 0.0, 400.0, 400.0, layers)])], &[]);
    import(&z, "f").unwrap().0
}

fn node<'d>(doc: &'d Document, name: &str) -> &'d figcraft_doc::Node {
    doc.nodes().find(|n| n.name == name).unwrap_or_else(|| panic!("no layer named {name}"))
}

fn with_op(mut l: Value, op: i64) -> Value {
    l["booleanOperation"] = json!(op);
    l
}

#[test]
fn parse_point_accepts_sketch_strings() {
    assert_eq!(parse_point("{0.5, 1}"), Some((0.5, 1.0)));
    assert_eq!(parse_point("{-0.25,2e-1}"), Some((-0.25, 0.2)));
    assert_eq!(parse_point("{1e400, 0}"), None);
    assert_eq!(parse_point("garbage"), None);
    assert_eq!(parse_point("{0.1}"), None);
    assert_eq!(parse_point(""), None);
}

#[test]
fn triangle_path_is_editable_vector() {
    let tri = shape_path("Tri", 10.0, 10.0, 100.0, 100.0, &[(0.5, 0.0), (1.0, 1.0), (0.0, 1.0)], true);
    let open_line = shape_path("Line", 10.0, 200.0, 100.0, 50.0, &[(0.0, 0.0), (1.0, 1.0)], false);
    let doc = open(vec![tri, open_line]);
    let NodeKind::Vector { network } = &node(&doc, "Tri").kind else { panic!("not a vector") };
    let pts: Vec<(f64, f64)> = network.vertices.iter().map(|v| (v.x, v.y)).collect();
    assert_eq!(pts, [(50.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
    assert_eq!(network.segments.len(), 3);
    assert_eq!(network.regions.len(), 1);
    let NodeKind::Vector { network } = &node(&doc, "Line").kind else { panic!("not a vector") };
    assert_eq!(network.segments.len(), 1);
    assert!(network.regions.is_empty());
}

#[test]
fn curve_handles_and_corner_radius_are_kept() {
    let mut p = shape_path("Curve", 0.0, 0.0, 100.0, 100.0, &[(0.0, 0.0), (1.0, 0.0)], false);
    p["points"][0]["curveMode"] = json!(2);
    p["points"][0]["hasCurveFrom"] = json!(true);
    p["points"][0]["curveFrom"] = json!("{0.25, 0.5}");
    p["points"][1]["curveMode"] = json!(2);
    p["points"][1]["hasCurveTo"] = json!(true);
    p["points"][1]["curveTo"] = json!("{0.75, 0.5}");
    p["points"][1]["cornerRadius"] = json!(6);
    let doc = open(vec![p]);
    let NodeKind::Vector { network } = &node(&doc, "Curve").kind else { panic!("not a vector") };
    let s = network.segments[0];
    assert_eq!((s.ta, s.tb), ((25.0, 50.0), (-25.0, 50.0)));
    assert_eq!(network.vertices[1].radius, 6.0);
}

#[test]
fn single_child_shape_group_collapses() {
    let inner = with_op(rect("Inner", 0.0, 0.0, 50.0, 30.0), -1);
    let doc = open(vec![shape_group("Button", 20.0, 40.0, 50.0, 30.0, vec![inner])]);
    let b = node(&doc, "Button");
    assert!(matches!(b.kind, NodeKind::Rectangle));
    assert!(doc.nodes().all(|n| n.name != "Inner"));
    let bb = doc.world_bounds(b.id).unwrap();
    assert_eq!((bb.x0, bb.y0, bb.width(), bb.height()), (20.0, 40.0, 50.0, 30.0));
}

#[test]
fn mixed_boolean_ops_chain_left() {
    let a = with_op(rect("A", 0.0, 0.0, 50.0, 50.0), -1);
    let b = with_op(rect("B", 10.0, 10.0, 20.0, 20.0), 1);
    let c = with_op(rect("C", 60.0, 0.0, 20.0, 20.0), 0);
    let doc = open(vec![shape_group("Combined", 0.0, 0.0, 80.0, 50.0, vec![a, b, c])]);
    let outer = node(&doc, "Combined");
    assert!(matches!(outer.kind, NodeKind::BooleanOp { op: BoolOp::Union }));
    let kids: Vec<&figcraft_doc::Node> = outer.children.iter().map(|c| doc.node(*c).unwrap()).collect();
    assert_eq!(kids.len(), 2);
    assert!(matches!(kids[0].kind, NodeKind::BooleanOp { op: BoolOp::Subtract }));
    assert_eq!(kids[1].name, "C");
    let inner: Vec<&str> = kids[0].children.iter().map(|c| doc.node(*c).unwrap().name.as_str()).collect();
    assert_eq!(inner, ["A", "B"]);
    let bb = doc.world_bounds(node(&doc, "B").id).unwrap();
    assert_eq!((bb.x0, bb.y0), (10.0, 10.0));
}

#[test]
fn intersect_and_difference_ops() {
    let a = with_op(rect("A", 0.0, 0.0, 50.0, 50.0), -1);
    let b = with_op(rect("B", 10.0, 10.0, 50.0, 50.0), 2);
    let doc = open(vec![shape_group("Both", 0.0, 0.0, 60.0, 60.0, vec![a.clone(), b])]);
    assert!(matches!(node(&doc, "Both").kind, NodeKind::BooleanOp { op: BoolOp::Intersect }));
    let b = with_op(rect("B", 10.0, 10.0, 50.0, 50.0), 3);
    let doc = open(vec![shape_group("Either", 0.0, 0.0, 60.0, 60.0, vec![a, b])]);
    assert!(matches!(node(&doc, "Either").kind, NodeKind::BooleanOp { op: BoolOp::Exclude }));
}

#[test]
fn standalone_shapes_52_plus() {
    let doc = open(vec![shape_path("Free", 5.0, 5.0, 10.0, 10.0, &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)], true)]);
    assert!(matches!(node(&doc, "Free").kind, NodeKind::Vector { .. }));
}

#[test]
fn star_and_polygon() {
    let unit = |pts: Vec<figcraft_doc::geom::Point>, w: f64, h: f64| -> Vec<(f64, f64)> { pts.iter().map(|p| (p.x / w, p.y / h)).collect() };
    let mut star = shape_path("Star", 0.0, 0.0, 100.0, 80.0, &unit(figcraft_doc::geom::star_points(100.0, 80.0, 5, 0.5), 100.0, 80.0), true);
    star["_class"] = json!("star");
    star["numberOfPoints"] = json!(5);
    star["radius"] = json!(0.5);
    let mut poly = shape_path("Hexagon", 0.0, 100.0, 60.0, 60.0, &unit(figcraft_doc::geom::polygon_points(60.0, 60.0, 6), 60.0, 60.0), true);
    poly["_class"] = json!("polygon");
    poly["numberOfPoints"] = json!(6);
    let mut edited = star.clone();
    edited["do_objectID"] = json!("Edited");
    edited["name"] = json!("Edited");
    edited["points"][2]["point"] = json!("{0.9, 0.9}");
    let mut tri = shape_path("Triangle", 200.0, 0.0, 50.0, 50.0, &[(0.5, 0.0), (1.0, 1.0), (0.0, 1.0)], true);
    tri["_class"] = json!("triangle");
    let doc = open(vec![star, poly, edited, tri]);
    assert!(matches!(node(&doc, "Star").kind, NodeKind::Star { count: 5, .. }));
    if let NodeKind::Star { inner_ratio, .. } = node(&doc, "Star").kind {
        assert!((inner_ratio - 0.5).abs() < 1e-9);
    }
    assert!(matches!(node(&doc, "Hexagon").kind, NodeKind::Polygon { count: 6 }));
    assert!(matches!(node(&doc, "Edited").kind, NodeKind::Vector { .. }));
    assert!(matches!(node(&doc, "Triangle").kind, NodeKind::Vector { .. }));
}

#[test]
fn points_outside_unit_are_kept_finite() {
    let doc = open(vec![shape_path("Over", 0.0, 0.0, 10.0, 10.0, &[(-0.5, 2.0), (1.0, 0.0), (0.0, 0.0)], true)]);
    let NodeKind::Vector { network } = &node(&doc, "Over").kind else { panic!("not a vector") };
    assert!(network.vertices.iter().all(|v| v.x.is_finite() && v.y.is_finite()));
    assert_eq!((network.vertices[0].x, network.vertices[0].y), (-5.0, 20.0));
}

#[test]
fn even_odd_winding() {
    let mut p = shape_path("Holes", 0.0, 0.0, 10.0, 10.0, &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)], true);
    p["style"]["windingRule"] = json!(1);
    let doc = open(vec![p]);
    let NodeKind::Vector { network } = &node(&doc, "Holes").kind else { panic!("not a vector") };
    assert_eq!(network.regions[0].winding, figcraft_doc::WindingRule::EvenOdd);
}

#[test]
fn huge_points_stay_finite() {
    let mut p = shape_path("Huge", 0.0, 0.0, 1e6, 1e6, &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)], true);
    p["points"][1]["point"] = json!("{1e308, 0}");
    p["points"][1]["curveMode"] = json!(2);
    p["points"][1]["curveFrom"] = json!("{-1e308, 1e308}");
    let doc = open(vec![p]);
    if let NodeKind::Vector { network } = &node(&doc, "Huge").kind {
        assert!(network.vertices.iter().all(|v| v.x.is_finite() && v.y.is_finite()));
        assert!(network.segments.iter().all(|s| [s.ta.0, s.ta.1, s.tb.0, s.tb.1].iter().all(|c| c.is_finite())));
    }
}

/// Sketch 51 and older: a shape group whose children have no operation (−1) is one compound path,
/// filled by the group's winding rule (stored on the layer itself).
#[test]
fn compound_shape_group_has_holes() {
    let outer = with_op(rect("Outer", 0.0, 0.0, 100.0, 100.0), -1);
    let inner = with_op(rect("Inner", 25.0, 25.0, 50.0, 50.0), -1);
    let mut g = shape_group("Ring", 10.0, 10.0, 100.0, 100.0, vec![outer, inner]);
    g["windingRule"] = json!(1);
    let doc = open(vec![g]);
    let n = node(&doc, "Ring");
    let NodeKind::Vector { network } = &n.kind else { panic!("not one vector: {:?}", n.kind) };
    assert_eq!(network.regions.len(), 1);
    assert_eq!(network.regions[0].loops.len(), 2);
    assert_eq!(network.regions[0].winding, figcraft_doc::WindingRule::EvenOdd);
    let b = doc.world_bounds(n.id).unwrap();
    assert_eq!((b.x0, b.y0, b.width(), b.height()), (10.0, 10.0, 100.0, 100.0));
}

#[test]
fn compound_runs_are_boolean_operands() {
    let a = with_op(rect("A", 0.0, 0.0, 50.0, 50.0), -1);
    let b = with_op(rect("B", 60.0, 0.0, 20.0, 20.0), -1);
    let c = with_op(rect("C", 10.0, 10.0, 10.0, 10.0), 1);
    let doc = open(vec![shape_group("Mixed", 0.0, 0.0, 80.0, 50.0, vec![a, b, c])]);
    let n = node(&doc, "Mixed");
    assert!(matches!(n.kind, NodeKind::BooleanOp { op: BoolOp::Subtract }), "{:?}", n.kind);
    let first = doc.node(n.children[0]).unwrap();
    let NodeKind::Vector { network } = &first.kind else { panic!("first operand isn't the compound path") };
    assert_eq!(network.regions[0].loops.len(), 2);
}

#[test]
fn layer_winding_rule_counts() {
    let mut p = shape_path("Wound", 0.0, 0.0, 10.0, 10.0, &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)], true);
    p["windingRule"] = json!(1);
    let doc = open(vec![p]);
    let NodeKind::Vector { network } = &node(&doc, "Wound").kind else { panic!("not a vector") };
    assert_eq!(network.regions[0].winding, figcraft_doc::WindingRule::EvenOdd);
}

/// Sketch fills an open path as if it were closed.
#[test]
fn filled_open_paths_are_closed() {
    let open_path = shape_path("Lid", 0.0, 0.0, 40.0, 20.0, &[(0.0, 1.0), (0.5, 0.0), (1.0, 1.0)], false);
    let filled = styled(shape_group("Filled", 0.0, 0.0, 40.0, 20.0, vec![open_path.clone()]), "fills", json!([solid_fill(0.0, 0.0, 0.0, 1.0)]));
    let stroked = styled(
        shape_group("Stroked", 0.0, 50.0, 40.0, 20.0, vec![open_path]),
        "borders",
        json!([{"_class": "border", "isEnabled": true, "fillType": 0, "color": color(0.0, 0.0, 0.0, 1.0), "position": 0, "thickness": 1}]),
    );
    let doc = open(vec![filled, stroked]);
    let NodeKind::Vector { network } = &node(&doc, "Filled").kind else { panic!("not a vector") };
    assert_eq!(network.regions.len(), 1);
    assert_eq!(network.segments.len(), 3);
    let NodeKind::Vector { network } = &node(&doc, "Stroked").kind else { panic!("not a vector") };
    assert!(network.regions.is_empty());
    assert_eq!(network.segments.len(), 2);
}

#[test]
fn open_operands_of_booleans_are_closed() {
    let a = with_op(rect("A", 0.0, 0.0, 50.0, 50.0), -1);
    let b = with_op(shape_path("Snout", 40.0, 10.0, 30.0, 20.0, &[(0.0, 0.0), (1.0, 0.5), (0.0, 1.0)], false), 0);
    let doc = open(vec![styled(shape_group("Head", 0.0, 0.0, 70.0, 50.0, vec![a, b]), "fills", json!([solid_fill(0.0, 0.0, 0.0, 1.0)]))]);
    let NodeKind::Vector { network } = &node(&doc, "Snout").kind else { panic!("not a vector") };
    assert_eq!(network.regions.len(), 1);
}

/// Sketch draws a shape group in the group's style only: its operands' own borders go, so a
/// FigCraft operand gives its area to the operation, not a stroke's band.
#[test]
fn shape_group_operands_lose_their_borders() {
    let border = json!([{"_class": "border", "isEnabled": true, "fillType": 0, "color": color(0.0, 0.0, 0.0, 1.0), "position": 0, "thickness": 8}]);
    let outer = styled(with_op(rect("Outer", 0.0, 0.0, 100.0, 100.0), 1), "borders", border.clone());
    let hole = styled(with_op(rect("Hole", 25.0, 25.0, 50.0, 50.0), 1), "borders", border);
    let g = styled(shape_group("Pin", 0.0, 0.0, 100.0, 100.0, vec![outer, hole]), "fills", json!([solid_fill(0.0, 0.0, 1.0, 1.0)]));
    let doc = open(vec![g]);
    let pin = node(&doc, "Pin");
    assert!(matches!(pin.kind, NodeKind::BooleanOp { op: BoolOp::Subtract }), "{:?}", pin.kind);
    assert_eq!(pin.fills.len(), 1);
    for name in ["Outer", "Hole"] {
        assert!(node(&doc, name).strokes.is_empty(), "{name}: {:?}", node(&doc, name).strokes);
    }
}
