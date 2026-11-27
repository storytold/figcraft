//! An export is as big as what it shows: strokes count as drawn, not grown by how far a stroke
//! could reach at a sharp corner.

use figcraft_doc::NodeId;
use figcraft_geom::Rect;
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn made(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

/// A 24 by 24 frame at (100, 100) that doesn't clip, as icons are drawn.
fn icon_frame(s: &mut Session) -> NodeId {
    let f = made(&ex(s, "frame.create", json!({"x": 100, "y": 100, "width": 24, "height": 24, "name": "icon"})));
    ex(s, "node.set", json!({"id": f, "props": {"clip": false, "fills": []}}));
    f
}

fn stroked(s: &mut Session, id: NodeId, weight: f64) {
    ex(s, "node.set", json!({"id": id, "props": {"stroke": "#000000", "strokeWeight": weight}}));
}

fn flatten(s: &mut Session, id: NodeId) -> NodeId {
    ex(s, "vector.flatten", json!({"ids": [id]}));
    s.selection()[0]
}

fn near(a: Rect, b: Rect) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(x, y)| (x - y).abs() < 0.06)
}

fn svg_size(s: &Session, id: NodeId) -> (String, String) {
    let svg = figcraft_svg::export_svg(s.doc().unwrap(), &[id], &figcraft_svg::SvgOptions::default()).unwrap();
    let attr = |name: &str| svg.split(&format!(" {name}=\"")).nth(1).and_then(|r| r.split('"').next()).unwrap().to_string();
    (attr("width"), attr("height"))
}

/// An icon whose strokes stay inside its frame exports at the frame's size, in SVG and PNG
/// alike: an outlined circle (a vector, as icons are) and a line from the frame's left edge.
#[test]
fn an_icon_with_strokes_inside_its_frame_exports_at_the_frames_size() {
    let mut s = Session::new();
    let f = icon_frame(&mut s);
    let ring = made(&ex(&mut s, "shape.ellipse", json!({"x": 3, "y": 3, "width": 18, "height": 18, "parent": f})));
    stroked(&mut s, ring, 1.0);
    flatten(&mut s, ring);
    let line = made(&ex(&mut s, "shape.line", json!({"x": 0, "y": 12, "width": 22, "parent": f})));
    stroked(&mut s, line, 1.0);
    let doc = s.doc().unwrap();
    let frame = Rect::new(100.0, 100.0, 124.0, 124.0);
    assert!(doc.world_render_bounds(f).unwrap().width() > 24.0, "the renderer's bounds keep their room for sharp corners");
    assert!(near(figcraft_render::export_bounds(doc, f).unwrap(), frame), "{:?}", figcraft_render::export_bounds(doc, f));
    assert_eq!(svg_size(&s, f), ("24".to_string(), "24".to_string()));
    let png = figcraft_render::render_node(s.doc().unwrap(), f, 1.0).unwrap();
    assert_eq!((png.width(), png.height()), (24, 24));
}

/// Alone, a stroked shape exports at its stroke as drawn: a centred stroke half its weight past
/// the outline, an outside one all of it, an inside one none.
#[test]
fn a_stroked_shape_exports_at_its_stroke_as_drawn() {
    let mut s = Session::new();
    let ring = made(&ex(&mut s, "shape.ellipse", json!({"x": 0, "y": 0, "width": 18, "height": 18})));
    stroked(&mut s, ring, 2.0);
    let ring = flatten(&mut s, ring);
    for (align, out) in [("center", 1.0), ("outside", 2.0), ("inside", 0.0)] {
        ex(&mut s, "node.set", json!({"id": ring, "props": {"strokeAlign": align}}));
        let b = figcraft_render::export_bounds(s.doc().unwrap(), ring).unwrap();
        assert!(near(b, Rect::new(-out, -out, 18.0 + out, 18.0 + out)), "{align}: {b:?}");
    }
}

/// A sharp corner's mitre still counts: a triangle's 60° apex, within the miter limit, reaches
/// twice as far as half the stroke's weight, and the export keeps it, within what the renderer
/// allows for.
#[test]
fn a_sharp_corners_mitre_is_kept() {
    let mut s = Session::new();
    let spike = made(&ex(&mut s, "node.create", json!({"type": "polygon", "x": 0, "y": 0, "width": 40, "height": 40})));
    stroked(&mut s, spike, 2.0);
    ex(&mut s, "node.set", json!({"id": spike, "props": {"strokeAlign": "center"}}));
    let doc = s.doc().unwrap();
    let b = figcraft_render::export_bounds(doc, spike).unwrap();
    assert!(b.y0 < -1.5, "the apex's mitre reaches past half the weight: {b:?}");
    let room = doc.world_render_bounds(spike).unwrap();
    assert!(b.y0 >= room.y0 && b.x0 >= room.x0 && b.x1 <= room.x1 && b.y1 <= room.y1, "{b:?} within {room:?}");
}
