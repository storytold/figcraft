//! Guides snap to layer edges and centres when dragged out of a ruler or moved, on the guide's own
//! axis only, within five screen pixels at any zoom (page guides see the page's top-level layers;
//! frame guides see the frame's box, layout grids and every layer in it).

use figcraft_doc::NodeId;
use figcraft_geom::Affine;
use serde_json::json;

use crate::Session;
use crate::params::id_of;
use crate::snap::{Targets, snap_guide};

fn id(v: serde_json::Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn page_snap(s: &Session, vertical: bool, offset: f64, zoom: f64) -> Option<f64> {
    let (doc, page) = (s.doc().unwrap(), s.page().unwrap());
    snap_guide(doc, page, page, vertical, offset, zoom)
}

#[test]
fn a_page_guide_snaps_to_a_top_level_layers_edges_and_centre() {
    let mut s = Session::new();
    s.execute("node.create", &json!({"type": "rectangle", "x": 100, "y": 300, "width": 50, "height": 40})).unwrap();
    // Edges at 100 and 150, centre at 125; within five pixels at zoom 1.
    assert_eq!(page_snap(&s, true, 102.0, 1.0), Some(100.0));
    assert_eq!(page_snap(&s, true, 124.0, 1.0), Some(125.0));
    assert_eq!(page_snap(&s, true, 148.0, 1.0), Some(150.0));
    // Out of reach.
    assert_eq!(page_snap(&s, true, 106.0, 1.0), None);
    // The guide's own axis only: x 302 is far from the x edges, y 302 is a pixel above 300.
    assert_eq!(page_snap(&s, true, 302.0, 1.0), None);
    assert_eq!(page_snap(&s, false, 302.0, 1.0), Some(300.0));
}

#[test]
fn the_reach_is_five_screen_pixels_at_any_zoom() {
    let mut s = Session::new();
    s.execute("node.create", &json!({"type": "rectangle", "x": 100, "y": 300, "width": 50, "height": 40})).unwrap();
    // Six page pixels away: out of reach at zoom 2 (2.5), in reach at zoom 0.5 (10).
    assert_eq!(page_snap(&s, true, 106.0, 2.0), None);
    assert_eq!(page_snap(&s, true, 106.0, 0.5), Some(100.0));
    // A zoom of zero is clamped rather than dividing by zero.
    assert_eq!(page_snap(&s, true, 100.0, 0.0), Some(100.0));
}

#[test]
fn a_frame_guide_snaps_to_its_box_its_children_and_nested_layers() {
    let mut s = Session::new();
    let frame = id(s.execute("node.create", &json!({"type": "frame", "x": 100, "y": 50, "width": 300, "height": 200})).unwrap());
    let inner = id(s.execute("node.create", &json!({"type": "frame", "parent": frame, "x": 10, "y": 10, "width": 60, "height": 60})).unwrap());
    s.execute("node.create", &json!({"type": "rectangle", "parent": inner, "x": 20, "y": 20, "width": 20, "height": 20})).unwrap();
    let outside = id(s.execute("node.create", &json!({"type": "rectangle", "x": 1000, "y": 1000, "width": 20, "height": 20})).unwrap());
    let page = s.page().unwrap();
    let snap = |s: &Session, vertical: bool, offset: f64| snap_guide(s.doc().unwrap(), page, frame, vertical, offset, 1.0);
    // The frame's own right edge is 300.
    assert_eq!(snap(&s, true, 299.0), Some(300.0));
    // The nested rectangle sits at 30 in the frame; its left edge and centre (40) are targets.
    assert_eq!(snap(&s, true, 31.0), Some(30.0));
    assert_eq!(snap(&s, true, 41.0), Some(40.0));
    // The layer outside the frame is not.
    assert_eq!(snap(&s, true, 999.0), None);
    // Hiding the nested rectangle takes its lines away.
    let inner_rect = s.doc().unwrap().node(inner).unwrap().children[0];
    s.execute("node.set", &json!({"ids": [inner_rect], "props": {"visible": false}})).unwrap();
    assert_eq!(snap(&s, true, 31.0), None);
    // A layer outside the frame still doesn't matter, and `outside` exists to prove the scope.
    assert!(s.doc().unwrap().node(outside).is_some());
}

#[test]
fn a_frame_guide_snaps_along_the_frames_own_axes_when_it_is_turned() {
    let mut s = Session::new();
    let frame = id(s.execute("node.create", &json!({"type": "frame", "x": 100, "y": 100, "width": 300, "height": 200})).unwrap());
    s.execute("node.create", &json!({"type": "rectangle", "parent": frame, "x": 40, "y": 20, "width": 20, "height": 20})).unwrap();
    s.execute("node.set", &json!({"ids": [frame], "props": {"rotation": 30.0}})).unwrap();
    let (doc, page) = (s.doc().unwrap(), s.page().unwrap());
    let t = Targets::gather_for_guide(doc, page, frame);
    assert_ne!(t.space, Affine::IDENTITY, "guides snap in the frame's space");
    assert!(t.xs.iter().any(|(x, _, _)| near(*x, 40.0)), "{:?}", t.xs);
    // The rotation rounds the offsets in the last bits, so compare with a tolerance.
    assert!(snap_guide(doc, page, frame, true, 41.0, 1.0).is_some_and(|v| near(v, 40.0)));
    assert!(snap_guide(doc, page, frame, false, 21.0, 1.0).is_some_and(|v| near(v, 20.0)));
}

#[test]
fn a_hidden_container_hides_its_children_from_snapping() {
    let mut s = Session::new();
    let frame = id(s.execute("node.create", &json!({"type": "frame", "x": 100, "y": 50, "width": 300, "height": 200})).unwrap());
    let inner = id(s.execute("node.create", &json!({"type": "frame", "parent": frame, "x": 10, "y": 10, "width": 60, "height": 60})).unwrap());
    let child = id(s.execute("node.create", &json!({"type": "rectangle", "parent": inner, "x": 20, "y": 20, "width": 20, "height": 20})).unwrap());
    let page = s.page().unwrap();
    let snap = |s: &Session, offset: f64| snap_guide(s.doc().unwrap(), page, frame, true, offset, 1.0);
    // The inner frame's left edge is 10 in the frame; the nested rectangle's is 30.
    assert_eq!(snap(&s, 11.0), Some(10.0));
    assert_eq!(snap(&s, 31.0), Some(30.0));
    // Hiding the inner frame hides its branch: the child's own flag stays true, but its edge is
    // not a target any more (nor is the hidden frame's).
    s.execute("node.set", &json!({"ids": [inner], "props": {"visible": false}})).unwrap();
    assert!(s.doc().unwrap().node(child).unwrap().visible);
    assert_eq!(snap(&s, 11.0), None);
    assert_eq!(snap(&s, 31.0), None);
}

#[test]
fn the_reach_is_five_screen_pixels_across_a_scaled_frame() {
    let mut s = Session::new();
    let frame = id(s.execute("node.create", &json!({"type": "frame", "x": 100, "y": 50, "width": 300, "height": 200})).unwrap());
    s.execute("node.create", &json!({"type": "rectangle", "parent": frame, "x": 100, "y": 10, "width": 20, "height": 20})).unwrap();
    // The frame is drawn at twice its own size, so an owner unit is two screen pixels at zoom 1.
    s.doc_mut().unwrap().node_mut(frame).unwrap().transform = Affine::scale(2.0);
    let page = s.page().unwrap();
    let snap = |s: &Session, offset: f64, zoom: f64| snap_guide(s.doc().unwrap(), page, frame, true, offset, zoom);
    // Five screen pixels are 2.5 frame units: two snaps to the edge at 100, three doesn't.
    assert_eq!(snap(&s, 102.0, 1.0), Some(100.0));
    assert_eq!(snap(&s, 103.0, 1.0), None);
    // Zooming in halves the owner-space reach again (1.25 frame units at 2x zoom).
    assert_eq!(snap(&s, 102.0, 2.0), None);
}

#[test]
fn a_guide_with_nothing_near_stays_where_it_is() {
    let s = Session::new();
    let page = s.page().unwrap();
    let doc = s.doc().unwrap();
    assert_eq!(snap_guide(doc, page, page, true, 12.5, 1.0), None);
    assert_eq!(snap_guide(doc, page, page, false, 12.5, 1.0), None);
    // Non-finite input is refused rather than spread.
    assert_eq!(snap_guide(doc, page, page, true, f64::NAN, 1.0), None);
}
