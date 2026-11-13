//! The rotation origin (Alt+R) and placing an object with Enter (`tool.place`), the two shortcuts
//! the shortcut parity table listed as missing.

use serde_json::json;

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind, ToolId};
use figcraft_doc::NodeId;
use figcraft_geom::Point;

fn rect(s: &mut Session) -> NodeId {
    let r = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap()["id"]).unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    r
}

fn ptr(s: &mut Session, kind: PointerKind, x: f64, y: f64) {
    s.pointer(&PointerEvent::at(kind, x, y, 1.0)).unwrap();
}

fn near(a: Point, b: Point) -> bool {
    (a - b).hypot() < 1e-6
}

fn bounds(s: &Session, id: NodeId) -> [f64; 4] {
    let b = s.doc().unwrap().world_bounds(id).unwrap();
    [b.x0, b.y0, b.x1, b.y1]
}

fn close(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6)
}

#[test]
fn origin_handle_drags_snaps_and_turns_the_handles_rotation() {
    let mut s = Session::new();
    let r = rect(&mut s);
    assert_eq!(s.execute("transform.rotationOrigin", &json!({})).unwrap()["editing"], true, "Alt+R shows it");
    assert!(near(s.ds().unwrap().origin_handle().unwrap(), Point::new(50.0, 50.0)), "at the centre");
    // Dragged from the centre to near the top-left corner: it snaps there, and the layer stays.
    ptr(&mut s, PointerKind::Down, 50.0, 50.0);
    ptr(&mut s, PointerKind::Move, 3.0, 2.0);
    ptr(&mut s, PointerKind::Up, 3.0, 2.0);
    assert!(near(s.ds().unwrap().moved_origin().unwrap(), Point::ZERO));
    assert!(close(bounds(&s, r), [0.0, 0.0, 100.0, 100.0]), "the press was the handle's, not a move");
    // The corner rotation zone turns about it: a quarter turn clockwise leaves the corner in place.
    ptr(&mut s, PointerKind::Down, -10.0, -10.0);
    ptr(&mut s, PointerKind::Move, 10.0, -10.0);
    ptr(&mut s, PointerKind::Up, 10.0, -10.0);
    assert!(close(bounds(&s, r), [-100.0, 0.0, 0.0, 100.0]), "{:?}", bounds(&s, r));
    assert!(near(s.ds().unwrap().moved_origin().unwrap(), Point::ZERO), "it turned with the layer");
    // Moving the layer takes the origin along.
    s.execute("transform.move", &json!({"dx": 10, "dy": 0})).unwrap();
    assert!(near(s.ds().unwrap().moved_origin().unwrap(), Point::new(10.0, 0.0)));
    // A double-click on it puts it back at the centre; Escape hides it, then deselects as before.
    ptr(&mut s, PointerKind::DoubleClick, 10.0, 0.0);
    assert!(s.ds().unwrap().moved_origin().is_none());
    assert!(near(s.ds().unwrap().origin_handle().unwrap(), Point::new(-40.0, 50.0)));
    s.escape().unwrap();
    assert!(s.ds().unwrap().origin_handle().is_none());
    assert_eq!(s.selection(), vec![r], "the first Escape only hid the handle");
}

#[test]
fn rotation_field_and_rotate_commands_turn_about_a_moved_origin() {
    let mut s = Session::new();
    let r = rect(&mut s);
    s.execute("transform.setRotationOrigin", &json!({"x": 100, "y": 100})).unwrap();
    // The rotation field (node.set) keeps the bottom-right corner where it is.
    s.execute("node.set", &json!({"ids": [r], "props": {"rotation": 30}})).unwrap();
    let doc = s.doc().unwrap();
    assert!(near(doc.world_transform(r) * Point::new(100.0, 100.0), Point::new(100.0, 100.0)));
    assert!((figcraft_geom::rotation_degrees(doc.node(r).unwrap().transform) - 30.0).abs() < 1e-6);
    // So do Rotate 90° and transform.rotate without a pivot.
    s.execute("object.rotateLeft", &json!({})).unwrap();
    s.execute("transform.rotate", &json!({"degrees": 15})).unwrap();
    let doc = s.doc().unwrap();
    assert!(near(doc.world_transform(r) * Point::new(100.0, 100.0), Point::new(100.0, 100.0)));
    assert!((figcraft_geom::rotation_degrees(doc.node(r).unwrap().transform) - 135.0).abs() < 1e-6);
    // Another layer's field turns about its own centre, as before.
    let other = id_of(&s.execute("shape.rectangle", &json!({"x": 300, "y": 0, "width": 100, "height": 100})).unwrap()["id"]).unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    s.execute("node.set", &json!({"ids": [other], "props": {"rotation": 45}})).unwrap();
    let c = s.doc().unwrap().world_bounds(other).unwrap().center();
    assert!(near(c, Point::new(350.0, 50.0)));
    // Reset puts it back at the centre.
    let v = s.execute("transform.setRotationOrigin", &json!({"reset": true})).unwrap();
    assert_eq!(v["moved"], false);
    assert!(s.execute("transform.setRotationOrigin", &json!({"x": 1})).is_err(), "x without y");
}

#[test]
fn the_absolute_rotation_key_turns_about_a_moved_origin_too() {
    let mut s = Session::new();
    let r = rect(&mut s);
    s.execute("transform.setRotationOrigin", &json!({"x": 100, "y": 100})).unwrap();
    s.execute("node.set", &json!({"ids": [r], "props": {"absoluteRotation": 30}})).unwrap();
    let doc = s.doc().unwrap();
    assert!(near(doc.world_transform(r) * Point::new(100.0, 100.0), Point::new(100.0, 100.0)));
    assert!((figcraft_geom::rotation_degrees(doc.world_transform(r)) - 30.0).abs() < 1e-6);
}

#[test]
fn origin_lasts_as_long_as_its_selection() {
    let mut s = Session::new();
    let r = rect(&mut s);
    s.execute("transform.rotationOrigin", &json!({"on": true})).unwrap();
    s.execute("transform.setRotationOrigin", &json!({"x": 0, "y": 0})).unwrap();
    // Undo of an edit that keeps the selection keeps it.
    s.execute("transform.move", &json!({"dx": 5, "dy": 0})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(near(s.ds().unwrap().moved_origin().unwrap(), Point::ZERO));
    // Another selection drops it, and selecting the layer again doesn't bring it back.
    s.execute("select.set", &json!({"ids": []})).unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    assert!(s.ds().unwrap().moved_origin().is_none() && s.ds().unwrap().origin_handle().is_none());
    // Nothing selected: refused.
    s.execute("select.set", &json!({"ids": []})).unwrap();
    assert!(s.execute("transform.rotationOrigin", &json!({})).is_err());
}

#[test]
fn enter_places_a_click_sized_object_in_the_middle_of_the_view() {
    let mut s = Session::new();
    s.execute("view.set", &json!({"cx": 500, "cy": 300, "zoom": 1})).unwrap();
    s.execute("tool.set", &json!({"tool": "rectangle"})).unwrap();
    let undo = s.ds().unwrap().history.undo.len();
    let v = s.execute("tool.place", &json!({})).unwrap();
    let id = id_of(&v["ids"][0]).unwrap();
    assert_eq!(s.selection(), vec![id], "selected");
    assert!(close(bounds(&s, id), [450.0, 250.0, 550.0, 350.0]), "{:?}", bounds(&s, id));
    assert_eq!(s.ds().unwrap().history.undo.len(), undo + 1, "one undo step");
    assert_eq!(s.tool.tool, ToolId::Move, "back to Move, as after a click");
    // A frame at a given point; a line a click's length.
    let f = id_of(&s.execute("tool.place", &json!({"tool": "frame", "x": 0, "y": 0})).unwrap()["ids"][0]).unwrap();
    assert!(close(bounds(&s, f), [-50.0, -50.0, 50.0, 50.0]));
    assert!(s.doc().unwrap().node(f).unwrap().kind.is_frame_like());
    let l = id_of(&s.execute("tool.place", &json!({"tool": "line"})).unwrap()["ids"][0]).unwrap();
    let b = bounds(&s, l);
    assert!((b[0] - 450.0).abs() < 1e-6 && (b[2] - 550.0).abs() < 1e-6 && (b[1] - 300.0).abs() < 1e-6, "{b:?}");
    // Not a shape tool: refused, nothing made.
    let n = s.doc().unwrap().node_count();
    assert!(s.execute("tool.place", &json!({})).is_err());
    assert!(s.execute("tool.place", &json!({"tool": "text"})).is_err());
    assert_eq!(s.doc().unwrap().node_count(), n);
}
