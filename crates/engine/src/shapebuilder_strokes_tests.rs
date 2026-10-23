//! The shape builder with strokes and open paths: strokes take part as their outlined areas, open
//! paths split the regions they cross, merging a stroke's region fills it with the stroke's paints,
//! and the strokes of the shapes involved become outlined vectors.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use figcraft_doc::{NodeId, NodeKind};
use figcraft_geom::Shape;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn id(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

/// A layer's filled area in page units.
fn area(s: &Session, id: NodeId) -> f64 {
    let doc = s.doc().unwrap();
    let (mut p, _) = figcraft_render::node_outline(doc, doc.node(id).unwrap()).unwrap();
    p.apply_affine(doc.world_transform(id));
    p.area().abs()
}

fn fill_hex(s: &Session, id: NodeId) -> String {
    s.doc().unwrap().node(id).unwrap().fills[0].solid_color().unwrap().to_hex()
}

/// A red 100 × 100 square and, above it, a black 10 wide open path across its middle from x = -20
/// to 120; both selected.
fn square_and_path(s: &mut Session) -> (NodeId, NodeId) {
    let r = id(&ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 100, "props": {"fill": "#ff0000"}})));
    let l = id(&ex(s, "vector.create", json!({"points": [[-20, 50], [120, 50]], "closed": false})));
    ex(s, "node.set", json!({"id": l, "props": {"strokes": ["#000000"], "strokeWeight": 10, "strokeAlign": "center"}}));
    s.set_selection(vec![r, l]);
    (r, l)
}

#[test]
fn a_stroked_open_path_splits_the_regions_it_crosses() {
    let mut s = Session::new();
    let (r, l) = square_and_path(&mut s);
    let out = ex(&mut s, "shapeBuilder.regions", json!({}));
    assert_eq!(out["sources"], json!([r, l]));
    let regions = out["regions"].as_array().unwrap();
    // Above and below the path, the band across the square, and the path's two ends outside it.
    assert_eq!(regions.len(), 5, "{out}");
    let covered = |x: f64, y: f64| -> Value {
        regions
            .iter()
            .find(|g| {
                let b = &g["bounds"];
                let (x0, y0) = (b[0].as_f64().unwrap(), b[1].as_f64().unwrap());
                x >= x0 && y >= y0 && x <= x0 + b[2].as_f64().unwrap() && y <= y0 + b[3].as_f64().unwrap()
            })
            .map(|g| g["parts"].clone())
            .unwrap_or(Value::Null)
    };
    assert_eq!(covered(50.0, 20.0), json!([{"id": r, "stroke": false}]));
    assert_eq!(covered(50.0, 50.0), json!([{"id": l, "stroke": true}, {"id": r, "stroke": false}]), "the stroke is above the fill");
    assert_eq!(covered(-10.0, 50.0), json!([{"id": l, "stroke": true}]));
    // A path without a visible stroke paints nothing and takes no part.
    ex(&mut s, "node.set", json!({"id": l, "props": {"strokes": []}}));
    let out = ex(&mut s, "shapeBuilder.regions", json!({}));
    assert_eq!(out["sources"], json!([r]));
    assert_eq!(out["regions"].as_array().unwrap().len(), 1);
}

#[test]
fn merging_a_stroke_region_fills_it_with_the_stroke() {
    let mut s = Session::new();
    let (r, l) = square_and_path(&mut s);
    let out = ex(&mut s, "shapeBuilder.merge", json!({"points": [[50, 50]]}));
    let m = id(&out);
    // The band, black (the stroke's paint), no stroke of its own.
    assert!((area(&s, m) - 1000.0).abs() < 1.0, "{}", area(&s, m));
    assert_eq!(fill_hex(&s, m), "000000");
    assert!(s.doc().unwrap().node(m).unwrap().strokes.is_empty());
    // The square lost the band; the path (no fill) became an outline of what was left of its
    // stroke: the two ends outside the square.
    assert!((area(&s, r) - 9000.0).abs() < 1.0, "{}", area(&s, r));
    assert_eq!(out["removed"], json!([l]));
    let o = id_of(&out["outlines"][0]).unwrap();
    assert!((area(&s, o) - 400.0).abs() < 1.0, "{}", area(&s, o));
    assert_eq!(fill_hex(&s, o), "000000");
    assert!(matches!(s.doc().unwrap().node(o).unwrap().kind, NodeKind::Vector { .. }));
    // The selection keeps working: the square, the outline and the new shape.
    let sel = s.selection();
    assert!(sel.contains(&r) && sel.contains(&o) && sel.contains(&m), "{sel:?}");
    // One undo step.
    s.undo().unwrap();
    let doc = s.doc().unwrap();
    assert!(doc.contains(l) && !doc.contains(m) && !doc.contains(o));
    assert!((area(&s, r) - 10000.0).abs() < 1.0);
}

#[test]
fn removing_regions_spares_strokes_that_arent_involved() {
    let mut s = Session::new();
    let (r, l) = square_and_path(&mut s);
    let out = ex(&mut s, "shapeBuilder.remove", json!({"points": [[50, 20]]}));
    // The square keeps the band and the part below; the path isn't touched.
    assert!((area(&s, r) - 5500.0).abs() < 1.0, "{}", area(&s, r));
    assert_eq!(out["outlines"], json!([]));
    let n = s.doc().unwrap().node(l).unwrap();
    assert!(!n.strokes.is_empty() && matches!(n.kind, NodeKind::Vector { .. }));
    // Removing an end of the path leaves the rest of its stroke as an outline.
    let out = ex(&mut s, "shapeBuilder.remove", json!({"ids": [r, l], "points": [[-10, 50]]}));
    assert_eq!(out["removed"], json!([l]));
    let o = id_of(&out["outlines"][0]).unwrap();
    assert!((area(&s, o) - (1000.0 + 200.0)).abs() < 1.0, "the band and the other end: {}", area(&s, o));
}

#[test]
fn a_stroked_shape_gives_its_ring_and_keeps_its_look() {
    let mut s = Session::new();
    // An inside stroke 10 wide: a ring inside the square's edge.
    let r = id(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 100, "props": {"fill": "#ff0000"}})));
    ex(&mut s, "node.set", json!({"id": r, "props": {"strokes": ["#0000ff"], "strokeWeight": 10, "strokeAlign": "inside"}}));
    s.set_selection(vec![r]);
    let out = ex(&mut s, "shapeBuilder.regions", json!({}));
    assert_eq!(out["regions"].as_array().unwrap().len(), 2, "the middle and the ring: {out}");
    // Removing the middle leaves the fill under the ring and the ring as a blue outline above it.
    let out = ex(&mut s, "shapeBuilder.remove", json!({"points": [[50, 50]]}));
    assert_eq!(out["changed"], json!([r]));
    assert!((area(&s, r) - 3600.0).abs() < 1.0, "{}", area(&s, r));
    assert!(s.doc().unwrap().node(r).unwrap().strokes.is_empty());
    let o = id_of(&out["outlines"][0]).unwrap();
    assert!((area(&s, o) - 3600.0).abs() < 1.0);
    assert_eq!(fill_hex(&s, o), "0000FF");
    let doc = s.doc().unwrap();
    let parent = doc.node(r).unwrap().parent.unwrap();
    let kids = &doc.node(parent).unwrap().children;
    assert!(kids.iter().position(|k| *k == o) > kids.iter().position(|k| *k == r), "the outline sits above the shape");
}

#[test]
fn a_centred_stroke_on_a_sharp_star_makes_three_regions() {
    let mut s = Session::new();
    // Five sharp tips: a centred stroke's outline folds over itself at each one and meets the
    // star's fill, which used to give overlapping regions and a spiked outline along them.
    let mut pts = Vec::new();
    for i in 0..10 {
        let a = -std::f64::consts::FRAC_PI_2 + i as f64 * std::f64::consts::PI / 5.0;
        let r = if i % 2 == 0 { 100.0 } else { 40.0 };
        pts.push([150.0 + r * a.cos(), 150.0 + r * a.sin()]);
    }
    let v = id(&ex(&mut s, "vector.create", json!({"points": pts, "closed": true})));
    ex(&mut s, "node.set", json!({"id": v, "props": {"fill": "#ff0000", "strokes": ["#0000ff"], "strokeWeight": 8, "strokeAlign": "center"}}));
    s.set_selection(vec![v]);
    let out = ex(&mut s, "shapeBuilder.regions", json!({}));
    assert_eq!(out["regions"].as_array().unwrap().len(), 3, "the inside and the stroke's two halves: {out}");
    let a = crate::shapebuilder::arrangement(&mut s, &[v]).unwrap();
    for (i, r) in a.regions.iter().enumerate() {
        for (j, o) in a.regions.iter().enumerate() {
            if let (false, Some(p)) = (i == j, r.point) {
                assert_eq!(o.path.winding(p), 0, "regions {i} and {j} overlap at {p:?}");
            }
        }
    }
    let mut covers: Vec<(bool, bool)> = a.regions.iter().map(|r| (r.parts.iter().any(|p| p.stroke), r.parts.iter().any(|p| !p.stroke))).collect();
    covers.sort();
    assert_eq!(covers, vec![(false, true), (true, false), (true, true)], "fill only, stroke only, both");
}

#[test]
fn stroke_regions_survive_junk() {
    for j in [
        json!({"points": [[f64::MAX, 0]]}),
        json!({"regions": [-1, 1e20, "x"]}),
        json!({"ids": [123_456], "points": [[50, 50]]}),
        json!({"points": "x"}),
    ] {
        for cmd in ["shapeBuilder.regions", "shapeBuilder.merge", "shapeBuilder.remove"] {
            let mut s = Session::new();
            let _ = square_and_path(&mut s);
            if let Err(crate::EngineError::Internal { cmd, msg }) = s.execute(cmd, &j) {
                panic!("{cmd} panicked on {j}: {msg}");
            }
        }
    }
}
