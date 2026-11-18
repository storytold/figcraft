//! The vector edit panel's Corner radius for a point (#2334): a typed radius larger than half the
//! point's segments is kept and shown as typed, and the point is drawn rounded across the whole of
//! the segments whose other ends stay sharp, as Figma draws it.

use egui::Key;
use figcraft_geom::Point;
use figcraft_geom::kurbo::{ParamCurve, PathSeg};
use serde_json::json;

use crate::field_tests::{field, type_into};
use crate::vector_point_xy_tests::setup;

#[test]
fn a_typed_radius_is_kept_shown_and_drawn_across_the_sharp_neighbours_segments() {
    // The 100 × 60 rectangle in vector edit mode; point 2 is its bottom right corner.
    let (mut u, id) = setup();
    u.app.session.execute("vector.edit.select", &json!({"vertices": [2]})).unwrap();
    type_into(&mut u, "Corner radius", "100", Key::Enter);
    assert_eq!(field(&mut u, "Corner radius").1, "100", "the field shows the radius as typed");
    let doc = u.app.session.doc().unwrap();
    let net = figcraft_engine::vectoredit::network(doc, id).unwrap();
    assert_eq!(net.vertices.iter().map(|v| v.radius).collect::<Vec<_>>(), vec![0.0, 0.0, 100.0, 0.0], "only point 2 stores it, unclamped");
    assert_eq!(net.vertices[2].point(), Point::new(100.0, 60.0));
    // Both neighbours are sharp, so the arc reaches the whole 60 of the shorter side: from the top
    // right corner to 60 along the bottom, on a circle of radius 60 centred at (40, 0).
    let arcs: Vec<_> = net
        .fill_path()
        .segments()
        .filter_map(|s| match s {
            PathSeg::Cubic(c) => Some(c),
            _ => None,
        })
        .collect();
    assert_eq!(arcs.len(), 1, "{arcs:?}");
    let ends = [arcs[0].p0, arcs[0].p3];
    let close = |a: Point, b: Point| (a - b).hypot() < 1e-9;
    assert!(ends.iter().any(|e| close(*e, Point::new(100.0, 0.0))) && ends.iter().any(|e| close(*e, Point::new(40.0, 60.0))), "{ends:?}");
    let h = 60.0 / 2f64.sqrt();
    assert!(close(arcs[0].eval(0.5), Point::new(40.0 + h, h)), "{:?}", arcs[0].eval(0.5));
}
