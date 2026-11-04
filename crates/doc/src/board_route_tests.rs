//! Connector routes (`board_route`): where an end attached to a side starts (#1916).

use figcraft_geom::{Point, Rect};

use crate::board::{Routing, Side};
use crate::board_route::{END_GAP, RouteEnd, route_points};

fn shape(x: f64, y: f64) -> Rect {
    Rect::new(x, y, x + 100.0, y + 60.0)
}

/// An end attached to a side starts `END_GAP` outside the middle of that side, not on the border,
/// so the stroke never lies on the fill (#1916: FigJam starts the line a few pixels outside).
#[test]
fn an_attached_end_starts_outside_the_border() {
    let (a, b) = (shape(0.0, 0.0), shape(300.0, 0.0));
    for routing in [Routing::Straight, Routing::Elbow, Routing::Curved] {
        let pts = route_points(RouteEnd::Box(a, Side::Auto), RouteEnd::Box(b, Side::Auto), routing).unwrap();
        assert_eq!(pts.first().copied(), Some(Point::new(100.0 + END_GAP, 30.0)), "{routing:?}: leaves the right side");
        assert_eq!(pts.last().copied(), Some(Point::new(300.0 - END_GAP, 30.0)), "{routing:?}: arrives at the left side");
    }
    // A named side, and the gap along its own normal.
    let pts = route_points(RouteEnd::Box(a, Side::Bottom), RouteEnd::Point(Point::new(50.0, 200.0)), Routing::Straight).unwrap();
    assert_eq!(pts.first().copied(), Some(Point::new(50.0, 60.0 + END_GAP)));
}

/// Elbows leaving one side share their first run along its normal, and curves leave tangent to it:
/// the second point of each lies straight out of the side.
#[test]
fn connectors_from_one_side_leave_together() {
    let a = shape(0.0, 0.0);
    let (up, down) = (RouteEnd::Box(shape(300.0, -200.0), Side::Auto), RouteEnd::Box(shape(300.0, 200.0), Side::Auto));
    for routing in [Routing::Elbow, Routing::Curved] {
        let p = route_points(RouteEnd::Box(a, Side::Right), up, routing).unwrap();
        let q = route_points(RouteEnd::Box(a, Side::Right), down, routing).unwrap();
        assert_eq!(p[0], q[0], "{routing:?}: the same start");
        assert_eq!(p[1].y, p[0].y, "{routing:?}: out along the side's normal first");
        assert_eq!(q[1].y, q[0].y, "{routing:?}: out along the side's normal first");
    }
}

/// A point placed by hand on the object (`RouteEnd::On`) is the user's: no gap is added.
#[test]
fn a_hand_placed_end_keeps_its_point() {
    let a = shape(0.0, 0.0);
    let on = Point::new(80.0, 60.0);
    let pts = route_points(RouteEnd::On(a, Side::Bottom, on), RouteEnd::Point(Point::new(80.0, 200.0)), Routing::Straight).unwrap();
    assert_eq!(pts.first().copied(), Some(on));
}
