//! A layer's hover and selection outline follows its geometry (#1506): a rectangle or frame with
//! rounded corners is outlined along each corner's radius and its smoothing, whatever the stroke's
//! alignment; one with square corners keeps its box.

use figcraft_doc::{Document, NodeId, geometry::shape_path};
use figcraft_engine::Session;
use figcraft_geom::kurbo::{BezPath, PathEl, Shape};
use serde_json::{Value, json};

use super::outline::outline_path;

fn layer(s: &mut Session, kind: &str, props: Value) -> NodeId {
    let v = s.execute("node.create", &json!({"type": kind, "x": 10, "y": 20, "width": 465, "height": 254})).unwrap();
    let id = NodeId(v["id"].as_u64().unwrap());
    s.execute("node.set", &json!({"id": id, "props": props})).unwrap();
    id
}

fn path_of(doc: &Document, id: NodeId) -> Option<BezPath> {
    outline_path(doc, doc.node(id).unwrap())
}

#[test]
fn rounded_corners_are_outlined_along_the_geometry_for_every_stroke_alignment() {
    let mut s = Session::new();
    for align in ["inside", "center", "outside"] {
        let props = json!({"stroke": "#000000", "strokeWeight": 12, "strokeAlign": align, "cornerSmoothing": 0.6,
            "topLeftRadius": 0, "topRightRadius": 60, "bottomRightRadius": 20, "bottomLeftRadius": 43});
        for kind in ["rectangle", "frame"] {
            let id = layer(&mut s, kind, props.clone());
            let doc = s.doc().unwrap();
            let n = doc.node(id).unwrap();
            let p = path_of(doc, id).unwrap_or_else(|| panic!("{kind}, {align}: a rounded outline"));
            assert_eq!(Some(&p), shape_path(n).as_ref(), "{kind}, {align}: the layer's own geometry");
            // The geometry's box, not grown or shrunk by the stroke.
            let b = p.bounding_box();
            assert!([b.x0, b.y0, b.x1 - 465.0, b.y1 - 254.0].iter().all(|d| d.abs() < 1e-6), "{kind}, {align}: {b:?}");
            assert!(p.elements().iter().any(|e| matches!(e, PathEl::CurveTo(..))), "{kind}, {align}: curved corners");
        }
    }
}

#[test]
fn square_corners_keep_the_box() {
    let mut s = Session::new();
    let id = layer(&mut s, "rectangle", json!({"stroke": "#000000", "strokeWeight": 4, "strokeAlign": "outside"}));
    assert!(path_of(s.doc().unwrap(), id).is_none());
}

/// How far, in screen points, layer `id`'s outline strays from its path: the largest distance from
/// the middle of one of its straight pieces to the path.
fn largest_stray(u: &crate::layers_drag_tests::Ui, id: NodeId) -> f64 {
    use figcraft_geom::kurbo::ParamCurveNearest;
    let doc = u.app.session.doc().unwrap().clone();
    let path = path_of(&doc, id).unwrap();
    let to_local = doc.world_transform(id).inverse();
    let zoom = u.app.session.ds().unwrap().view.zoom;
    let mut largest: f64 = 0.0;
    for shape in super::outline::outline_shapes(&u.app, &doc, id, egui::Stroke::new(1.0, egui::Color32::WHITE)) {
        let egui::Shape::Path(line) = shape else { panic!("a polyline outline") };
        for piece in line.points.windows(2) {
            let middle = to_local * u.app.to_page(piece[0] + (piece[1] - piece[0]) * 0.5);
            let d = path.segments().map(|seg| seg.nearest(middle, 1e-9).distance_sq).fold(f64::MAX, f64::min).sqrt();
            largest = largest.max(d * zoom);
        }
    }
    largest
}

/// The outline is flattened to 0.02 pt on screen at any zoom, so its straight pieces don't show:
/// each stays well within 0.1 pt of the curve (0.5 pt left straight pieces visible on an ellipse).
#[test]
fn a_selected_ellipses_outline_stays_on_the_curve_at_any_zoom() {
    let mut s = Session::new();
    let ellipse =
        NodeId(s.execute("node.create", &json!({"type": "ellipse", "x": 0, "y": 0, "width": 65, "height": 50})).unwrap()["id"].as_u64().unwrap());
    let mut u = crate::layers_drag_tests::Ui::new(s);
    for zoom in [1.0, 3.5, 16.0] {
        u.app.session.execute("view.set", &json!({"zoom": zoom, "cx": 30, "cy": 25})).unwrap();
        u.frame(vec![]);
        let stray = largest_stray(&u, ellipse);
        assert!(stray < 0.1, "at {zoom}x the outline strays {stray:.3} pt from the curve");
    }
}

/// A vector of `rows` closed bands, each `waves` cubic waves 10 pt wide along its top.
fn waves(rows: usize, waves: usize) -> String {
    let mut d = String::new();
    for r in 0..rows {
        let y = r as f64 * 10.0;
        d += &format!("M0 {y} ");
        for k in 0..waves {
            let x = k as f64 * 10.0;
            d += &format!("C{} {} {} {} {} {y} ", x + 3.3, y - 8.0, x + 6.6, y + 8.0, x + 10.0);
        }
        d += &format!("L{} {} L0 {} Z ", waves as f64 * 10.0, y + 6.0, y + 6.0);
    }
    d
}

/// Heavy vectors are flattened coarser, so hovering, panning and zooming them stays as cheap as
/// before: a vector of 5,000 curves keeps the points 0.5 pt gave it (0.02 pt gives about five times
/// as many), and one of 200 still bends as smoothly as an ellipse.
#[test]
fn a_heavy_vectors_outline_keeps_the_points_it_had() {
    use super::outline::{SMOOTH_CURVES, outline_shapes, outline_tolerance};
    let mut s = Session::new();
    let vector = |s: &mut Session, d: String| {
        NodeId(s.execute("node.create", &json!({"type": "vector", "path": d, "closed": true, "x": 0, "y": 0})).unwrap()["id"].as_u64().unwrap())
    };
    let heavy = vector(&mut s, waves(50, 100));
    let light = vector(&mut s, waves(2, SMOOTH_CURVES / 2));
    let u = crate::layers_drag_tests::Ui::new(s);
    let doc = u.app.session.doc().unwrap().clone();
    let points = |id: NodeId| -> usize {
        outline_shapes(&u.app, &doc, id, egui::Stroke::new(1.0, egui::Color32::WHITE))
            .iter()
            .map(|shape| if let egui::Shape::Path(line) = shape { line.points.len() } else { 0 })
            .sum()
    };
    let zoom = u.app.session.ds().unwrap().view.zoom;
    let mut before = 0;
    figcraft_geom::kurbo::flatten(path_of(&doc, heavy).unwrap().iter(), 0.5 / zoom, |_| before += 1);
    assert!(points(heavy) <= before, "{} points, {before} at 0.5 pt", points(heavy));
    assert_eq!(outline_tolerance(SMOOTH_CURVES), 0.02);
    assert!(largest_stray(&u, light) < 0.1, "{SMOOTH_CURVES} curves bend smoothly");
}
