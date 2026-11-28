//! Handle boxes for several layers that share a rotation.

use serde_json::json;

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind, selection_box, selection_frame};
use figcraft_doc::NodeId;
use figcraft_geom::Point;

fn rect(s: &mut Session, x: f64, y: f64, rotation: f64) -> NodeId {
    let id = id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": 100, "height": 50})).unwrap()["id"]).unwrap();
    s.execute("node.set", &json!({"id": id, "props": {"rotation": rotation}})).unwrap();
    id
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn shared_rotation_keeps_the_page_aligned_box() {
    let mut s = Session::new();
    let ids = [rect(&mut s, 0.0, 0.0, 30.0), rect(&mut s, 40.0, 120.0, 30.0)];
    let doc = s.doc().unwrap();
    let b = doc.bounds_of(&ids).unwrap();
    let (frame, w, h) = selection_frame(doc, &ids).unwrap();
    assert!(near(w, b.width()) && near(h, b.height()), "{w} {h} vs {} {}", b.width(), b.height());
    let q = selection_box(doc, &ids).unwrap();
    assert_eq!(q, [Point::new(b.x0, b.y0), Point::new(b.x1, b.y0), Point::new(b.x1, b.y1), Point::new(b.x0, b.y1)]);
    let c = frame.as_coeffs();
    assert!(near(c[0], 1.0) && near(c[1], 0.0) && near(c[2], 0.0) && near(c[3], 1.0), "{c:?}");
}

#[test]
fn different_or_right_angle_rotations_keep_the_page_aligned_box() {
    let mut s = Session::new();
    for (r0, r1) in [(30.0, 45.0), (90.0, 90.0), (0.0, 180.0)] {
        let ids = [rect(&mut s, 0.0, 0.0, r0), rect(&mut s, 200.0, 50.0, r1)];
        let doc = s.doc().unwrap();
        let b = doc.bounds_of(&ids).unwrap();
        let q = selection_box(doc, &ids).unwrap();
        assert_eq!(q, [Point::new(b.x0, b.y0), Point::new(b.x1, b.y0), Point::new(b.x1, b.y1), Point::new(b.x0, b.y1)], "{r0} {r1}");
    }
}

#[test]
fn resizing_a_rotated_multi_selection_scales_along_page_axes() {
    let mut s = Session::new();
    let ids = [rect(&mut s, 0.0, 0.0, 30.0), rect(&mut s, 40.0, 120.0, 30.0)];
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let (frame, w, h) = selection_frame(s.doc().unwrap(), &ids).unwrap();
    // Drag the bottom-right handle to make the page-aligned box 1.5× wider and 2× taller.
    let (from, to) = (frame * Point::new(w, h), frame * Point::new(w * 1.5, h * 2.0));
    s.pointer(&PointerEvent::at(PointerKind::Down, from.x, from.y, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, to.x, to.y, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, to.x, to.y, 1.0)).unwrap();
    let doc = s.doc().unwrap();
    let (_f2, w2, h2) = selection_frame(doc, &ids).unwrap();
    assert!(w2 > w && h2 > h, "{w2} {h2} vs {w} {h}");
    for i in ids {
        let n = doc.node(i).unwrap();
        assert!(n.width > 100.0 && n.height > 50.0, "{} {}", n.width, n.height);
        assert!(near(figcraft_geom::rotation_degrees(n.transform), 30.0));
    }
    // One undo step restores both.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(near(s.doc().unwrap().node(ids[0]).unwrap().width, 100.0));
}
