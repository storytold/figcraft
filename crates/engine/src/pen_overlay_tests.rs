//! What the Pen tool shows while a path is drawn (`tool.overlay`): the handles shaping the segments
//! at the pen, and the rubber band to the pointer, which follows the last anchor's out handle.

use crate::Session;
use crate::tools::{PointerEvent, PointerKind, ToolId};

fn at(s: &mut Session, kind: PointerKind, x: f64, y: f64) {
    s.pointer(&PointerEvent::at(kind, x, y, 1.0)).unwrap();
}

/// A press at (x, y) dragged to (to_x, to_y) and released: a smooth point with out handle
/// (to - p).
fn drag_point(s: &mut Session, x: f64, y: f64, to_x: f64, to_y: f64) {
    at(s, PointerKind::Down, x, y);
    at(s, PointerKind::Move, to_x, to_y);
    at(s, PointerKind::Up, to_x, to_y);
}

#[test]
fn the_dragged_point_shows_its_handles_and_the_one_before_its_out_handle() {
    let mut s = Session::new();
    s.set_tool(ToolId::Pen);
    drag_point(&mut s, 100.0, 100.0, 140.0, 100.0);
    assert_eq!(s.tool.overlay.pen_handles, vec![[100.0, 100.0, 60.0, 100.0], [100.0, 100.0, 140.0, 100.0]], "in and out of the first point");
    // The second point, mid-drag.
    at(&mut s, PointerKind::Down, 200.0, 200.0);
    assert_eq!(s.tool.overlay.rubber, None, "the press shows the segment itself, not a straight band");
    at(&mut s, PointerKind::Move, 200.0, 240.0);
    assert_eq!(
        s.tool.overlay.pen_handles,
        vec![[100.0, 100.0, 140.0, 100.0], [200.0, 200.0, 200.0, 160.0], [200.0, 200.0, 200.0, 240.0]],
        "the one before's out handle, then the dragged point's in and out"
    );
    at(&mut s, PointerKind::Up, 200.0, 240.0);
    // Moving on: the band leaves along the last out handle.
    at(&mut s, PointerKind::Hover, 300.0, 200.0);
    assert_eq!(s.tool.overlay.rubber, Some([200.0, 200.0, 300.0, 200.0]));
    assert_eq!(s.tool.overlay.rubber_handle, Some([200.0, 240.0]));
}

#[test]
fn corner_points_have_no_handles_and_a_straight_band() {
    let mut s = Session::new();
    s.set_tool(ToolId::Pen);
    for (x, y) in [(100.0, 100.0), (200.0, 100.0)] {
        at(&mut s, PointerKind::Down, x, y);
        at(&mut s, PointerKind::Up, x, y);
    }
    at(&mut s, PointerKind::Hover, 250.0, 200.0);
    assert!(s.tool.overlay.pen_handles.is_empty());
    assert_eq!(s.tool.overlay.rubber, Some([200.0, 100.0, 250.0, 200.0]));
    assert_eq!(s.tool.overlay.rubber_handle, None, "straight");
}

#[test]
fn finishing_the_path_clears_its_handles_and_band() {
    let mut s = Session::new();
    s.set_tool(ToolId::Pen);
    drag_point(&mut s, 100.0, 100.0, 140.0, 100.0);
    drag_point(&mut s, 200.0, 200.0, 200.0, 240.0);
    at(&mut s, PointerKind::Hover, 300.0, 200.0);
    s.pen_finish().unwrap();
    let ov = &s.tool.overlay;
    assert!(ov.pen.is_empty() && ov.pen_handles.is_empty() && ov.rubber.is_none() && ov.rubber_handle.is_none());
}

#[test]
fn shift_keeps_the_band_and_the_next_point_at_45_degree_steps() {
    // #970: with Shift the band shows where the point will land, at a 45° step from the last
    // anchor (the same length as the pointer is away), not under the pointer.
    let mut s = Session::new();
    s.set_tool(ToolId::Pen);
    at(&mut s, PointerKind::Down, 100.0, 100.0);
    at(&mut s, PointerKind::Up, 100.0, 100.0);
    let shifted = |kind, x, y| PointerEvent { shift: true, ..PointerEvent::at(kind, x, y, 1.0) };
    s.pointer(&shifted(PointerKind::Hover, 300.0, 120.0)).unwrap();
    let [x0, y0, x1, y1] = s.tool.overlay.rubber.unwrap();
    assert_eq!([x0, y0, y1], [100.0, 100.0, 100.0], "level with the last anchor");
    assert!((x1 - (100.0 + 200f64.hypot(20.0))).abs() < 1e-9, "{x1}");
    s.pointer(&shifted(PointerKind::Down, 300.0, 120.0)).unwrap();
    s.pointer(&shifted(PointerKind::Up, 300.0, 120.0)).unwrap();
    s.pointer(&shifted(PointerKind::Hover, 400.0, 230.0)).unwrap();
    let [x0, y0, x1, y1] = s.tool.overlay.rubber.unwrap();
    assert_eq!(y0, 100.0, "the point landed on the band");
    assert!((x1 - x0 - (y1 - y0)).abs() < 1e-9, "45° down to the right: {:?}", [x0, y0, x1, y1]);
}
