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
