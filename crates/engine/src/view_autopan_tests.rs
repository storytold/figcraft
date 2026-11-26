//! Canvas auto-pan (#2015): the speed a drag held near the viewport's edge pans the view at, as
//! measured in the incumbent at 100% (screen px per second, `40 x (15 - d)` clamped to 0..600,
//! `d` the pointer's distance inside the edge), and which gestures pan.

use crate::ViewState;

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn the_speed_grows_40_px_per_second_for_each_px_inside_the_zone_up_to_600() {
    for (d, speed) in [(40.0, 0.0), (25.0, 0.0), (15.0, 0.0), (12.0, 120.0), (8.0, 280.0), (2.0, 520.0), (0.0, 600.0), (-30.0, 600.0)] {
        assert!(near(ViewState::autopan_speed(d), speed), "{d} px inside: {} px/s, wanted {speed}", ViewState::autopan_speed(d));
    }
    assert_eq!(ViewState::autopan_speed(f64::NAN), 0.0);
    assert!(near(ViewState::autopan_speed(f64::NEG_INFINITY), 600.0));
}

#[test]
fn the_velocity_points_toward_the_near_edge_on_each_axis() {
    let (w, h) = (1000.0, 600.0);
    let v = |x: f64, y: f64| ViewState::autopan_velocity(x, y, w, h);
    assert_eq!(v(500.0, 300.0), (0.0, 0.0), "the middle");
    assert_eq!(v(1000.0 - 2.0, 300.0), (520.0, 0.0), "right edge: right");
    assert_eq!(v(2.0, 300.0), (-520.0, 0.0), "left edge: left");
    assert_eq!(v(500.0, 600.0 - 8.0), (0.0, 280.0), "bottom edge: down");
    assert_eq!(v(500.0, 12.0), (0.0, -120.0), "top edge: up");
    assert_eq!(v(1000.0 - 40.0, 300.0), (0.0, 0.0), "40 px in: none");
    assert_eq!(v(1000.0 - 15.0, 300.0), (0.0, 0.0), "15 px in: none");
    // A corner pans on both axes, each by its own distance.
    assert_eq!(v(1000.0 - 12.0, 600.0 - 2.0), (120.0, 520.0));
    assert_eq!(v(8.0, 2.0), (-280.0, -520.0));
    // Past the edge, or outside the viewport altogether: the cap.
    assert_eq!(v(1030.0, 300.0), (600.0, 0.0));
    assert_eq!(v(-500.0, 900.0), (-600.0, 600.0));
}

#[test]
fn panning_moves_the_view_by_the_velocity_in_screen_px_at_any_zoom() {
    for zoom in [0.25, 1.0, 4.0] {
        let mut v = ViewState { vw: 1000.0, vh: 600.0, zoom, cx: 50.0, cy: 70.0, ..Default::default() };
        assert!(v.autopan(998.0, 300.0, 0.5), "in the zone");
        assert!(near((v.cx - 50.0) * zoom, 260.0), "{zoom}: 520 px/s for half a second is 260 screen px");
        assert_eq!(v.cy, 70.0);
        assert!(!v.autopan(500.0, 300.0, 0.5), "out of the zone");
        assert!(!v.autopan(998.0, 300.0, 0.0), "no time");
        assert!(!v.autopan(998.0, 300.0, f64::NAN), "no time");
    }
}
