//! Canvas auto-pan (#2015), driven through egui headless at 60 frames per second: a drag that
//! holds the pointer within about 15 px of the canvas's edge, or past it, pans the view toward
//! it at `40 x (15 - d)` screen px per second (d: px inside the edge, at most 600), steadily while
//! the pointer stays still. It covers moving layers, the selection box and drawing a shape (the
//! prototype connection is in `proto_canvas`); resizing, rotating and every other drag leave the
//! view where it is.

use egui::{Event, Pos2, pos2, vec2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

/// One frame's length, in seconds.
const DT: f64 = 1.0 / 60.0;

fn id(v: serde_json::Value) -> NodeId {
    figcraft_engine::params::id_of(&v["id"]).unwrap()
}

/// An empty file at 100%, centred on the page's origin, with the canvas laid out.
fn setup() -> Ui {
    let mut u = Ui::new(Session::new());
    {
        let v = &mut u.app.session.ds_mut().unwrap().view;
        v.zoom = 1.0;
        v.cx = 0.0;
        v.cy = 0.0;
    }
    u.frame(vec![]);
    u.frame(vec![]);
    u
}

fn run(u: &mut Ui, cmd: &str, params: serde_json::Value) -> serde_json::Value {
    u.app.session.execute(cmd, &params).unwrap()
}

/// The view's centre, in page units.
fn centre(u: &Ui) -> (f64, f64) {
    let v = &u.app.session.ds().unwrap().view;
    (v.cx, v.cy)
}

fn bounds_x0(u: &Ui, n: NodeId) -> f64 {
    u.app.session.doc().unwrap().world_bounds(n).unwrap().x0
}

/// Move the pointer to `to` (button held) and hold it there for `frames` frames.
fn hold(u: &mut Ui, to: Pos2, frames: usize) {
    u.frame(vec![Event::PointerMoved(to)]);
    for _ in 1..frames {
        u.frame(vec![]);
    }
}

fn release(u: &mut Ui, at: Pos2) {
    u.button(at, false);
}

/// A 100 x 50 rectangle in the middle of the view, nothing selected; its screen centre.
fn centred_rect(u: &mut Ui) -> (NodeId, Pos2) {
    let r = id(run(u, "shape.rectangle", json!({"x": -50, "y": -25, "width": 100, "height": 50})));
    run(u, "select.none", json!({}));
    u.frame(vec![]);
    (r, u.app.canvas.rect.center())
}

/// Press on `p`, drag a little (past the slop) and settle.
fn pick_up(u: &mut Ui, p: Pos2) -> Pos2 {
    u.button(p, true);
    let moved = p + vec2(12.0, 0.0);
    u.frame(vec![Event::PointerMoved(moved)]);
    u.frame(vec![]);
    moved
}

#[test]
fn a_layer_held_2_px_inside_the_right_edge_pans_the_view_at_520_px_per_second_and_stays_under_the_pointer() {
    let mut u = setup();
    let (r, p) = centred_rect(&mut u);
    let from = pick_up(&mut u, p);
    // Out of the zone first (40 px in): no pan, however long.
    let rect = u.app.canvas.rect;
    let out = pos2(rect.max.x - 40.0, from.y);
    hold(&mut u, out, 20);
    assert_eq!(centre(&u), (0.0, 0.0), "40 px inside: no pan");
    let at = pos2(rect.max.x - 2.0, from.y);
    let n = 30;
    let (cx0, x0) = (centre(&u).0, bounds_x0(&u, r));
    let grab = x0 - u.app.to_page(out).x;
    hold(&mut u, at, n);
    let (cx1, x1) = (centre(&u).0, bounds_x0(&u, r));
    let pan = cx1 - cx0;
    let want = 520.0 * n as f64 * DT;
    assert!((pan - want).abs() < 0.5, "panned {pan} px for {n} frames, wanted {want}");
    assert_eq!(centre(&u).1, 0.0);
    // The layer went with the pointer: the pointer's move into the zone plus the pan.
    let went = x1 - x0;
    assert!((went - (38.0 + pan)).abs() < 1.5, "the layer moved {went}, the pointer 38 and the view {pan}");
    let now = x1 - u.app.to_page(at).x;
    assert!((now - grab).abs() < 1.5, "the layer stays under the pointer: {grab} then {now}");
    // Constant speed: the same again for the same time.
    hold(&mut u, at, n);
    assert!((centre(&u).0 - cx1 - want).abs() < 0.5, "no acceleration");
    // Back out of the zone it stops; so it does on release.
    hold(&mut u, out, 10);
    let stopped = centre(&u);
    hold(&mut u, out, 10);
    assert_eq!(centre(&u), stopped, "out of the zone");
    hold(&mut u, at, 3);
    release(&mut u, at);
    let done = centre(&u);
    for _ in 0..10 {
        u.frame(vec![]);
    }
    assert_eq!(centre(&u), done, "released");
    assert!(!u.app.session.tool.busy());
}

#[test]
fn the_speed_depends_on_the_distance_and_a_corner_pans_both_ways() {
    let mut u = setup();
    let (_, p) = centred_rect(&mut u);
    let from = pick_up(&mut u, p);
    let rect = u.app.canvas.rect;
    // 12 px in: 120 px/s; 8 px: 280; past the edge: 600 (the pointer stays on the canvas).
    for (d, speed) in [(12.0, 120.0), (8.0, 280.0), (0.0, 600.0)] {
        hold(&mut u, pos2(rect.max.x - 40.0, from.y), 2);
        let before = centre(&u).0;
        hold(&mut u, pos2(rect.max.x - d, from.y), 60);
        let pan = centre(&u).0 - before;
        assert!((pan - speed).abs() < 3.0, "{d} px in: panned {pan} px in a second, wanted {speed}");
    }
    // The bottom-left corner: down by its distance from the bottom, left by its from the left.
    hold(&mut u, rect.center(), 2);
    let (cx, cy) = centre(&u);
    hold(&mut u, pos2(rect.min.x + 8.0, rect.max.y - 2.0), 30);
    let (dx, dy) = (centre(&u).0 - cx, centre(&u).1 - cy);
    assert!((dx + 280.0 / 2.0).abs() < 2.0, "left at 280 px/s for half a second: {dx}");
    assert!((dy - 520.0 / 2.0).abs() < 2.0, "down at 520 px/s for half a second: {dy}");
}

#[test]
fn a_selection_box_held_at_the_bottom_edge_pans_and_grows_over_what_scrolls_in() {
    let mut u = setup();
    let below = f64::from(u.app.canvas.rect.height()) / 2.0 + 120.0;
    let far = id(run(&mut u, "shape.rectangle", json!({"x": -40, "y": below, "width": 80, "height": 40})));
    run(&mut u, "select.none", json!({}));
    u.frame(vec![]);
    let rect = u.app.canvas.rect;
    let press = rect.center() + vec2(-200.0, -100.0);
    u.button(press, true);
    let at = pos2(press.x + 250.0, rect.max.y - 2.0);
    u.frame(vec![Event::PointerMoved(at)]);
    assert!(u.app.session.selection().is_empty(), "not in the box yet");
    let box0 = u.app.session.tool.overlay.marquee.unwrap();
    let cy0 = centre(&u).1;
    let n = 45;
    for _ in 1..n {
        u.frame(vec![]);
    }
    let pan = centre(&u).1 - cy0;
    let want = 520.0 * (n - 1) as f64 * DT;
    assert!((pan - want).abs() < 1.0, "panned {pan}, wanted {want}");
    let box1 = u.app.session.tool.overlay.marquee.unwrap();
    assert!((box1[3] - box0[3] - pan).abs() < 1.0, "the box's bottom followed the pointer: {box0:?} then {box1:?}");
    assert_eq!(u.app.session.selection(), vec![far], "the box reached the layer that scrolled in");
    release(&mut u, at);
}

#[test]
fn a_press_that_has_not_become_a_drag_does_not_pan() {
    let mut u = setup();
    let rect = u.app.canvas.rect;
    // A click on the empty canvas right by the edge is not a drag.
    let at = pos2(rect.max.x - 2.0, rect.center().y);
    u.button(at, true);
    for _ in 0..30 {
        u.frame(vec![]);
    }
    assert_eq!(centre(&u), (0.0, 0.0));
    release(&mut u, at);
}

#[test]
fn a_shape_drawn_at_the_edge_pans_and_grows() {
    let mut u = setup();
    run(&mut u, "tool.set", json!({"tool": "rectangle"}));
    let rect = u.app.canvas.rect;
    let press = rect.center();
    u.button(press, true);
    u.frame(vec![Event::PointerMoved(press + vec2(30.0, 30.0))]);
    let at = pos2(press.x + 30.0, rect.max.y - 2.0);
    u.frame(vec![Event::PointerMoved(at)]);
    let cy0 = centre(&u).1;
    for _ in 0..30 {
        u.frame(vec![]);
    }
    let pan = centre(&u).1 - cy0;
    assert!((pan - 520.0 * 30.0 * DT).abs() < 1.0, "panned {pan}");
    release(&mut u, at);
    let r = u.app.session.selection()[0];
    let h = u.app.session.doc().unwrap().world_bounds(r).unwrap().height();
    let before = f64::from(at.y - press.y);
    assert!(h > before + pan - 2.0, "the rectangle grew with the pan: {h} for a drag of {before} and a pan of {pan}");
}

#[test]
fn a_resize_held_at_the_edge_does_not_pan() {
    let mut u = setup();
    let r = id(run(&mut u, "shape.rectangle", json!({"x": 100, "y": -25, "width": 100, "height": 50})));
    run(&mut u, "select.set", json!({"ids": [r]}));
    u.frame(vec![]);
    let rect = u.app.canvas.rect;
    let corner = u.app.to_screen(200.0, 25.0);
    u.button(corner, true);
    let at = pos2(rect.max.x - 2.0, corner.y);
    u.frame(vec![Event::PointerMoved(at)]);
    for _ in 0..30 {
        u.frame(vec![]);
    }
    let b = u.app.session.doc().unwrap().world_bounds(r).unwrap();
    assert!(b.width() > 100.0, "it resized, so the press was on the handle: {}", b.width());
    assert_eq!(centre(&u), (0.0, 0.0), "a resize does not pan");
    release(&mut u, at);
}

#[test]
fn a_hand_drag_does_not_pan_on_its_own_at_the_edge() {
    let mut u = setup();
    run(&mut u, "tool.set", json!({"tool": "hand"}));
    let rect = u.app.canvas.rect;
    let at = pos2(rect.max.x - 2.0, rect.center().y);
    u.button(rect.center(), true);
    hold(&mut u, at, 20);
    // The hand moves the view with the pointer, by the drag, and nothing more.
    let c = centre(&u);
    for _ in 0..20 {
        u.frame(vec![]);
    }
    assert_eq!(centre(&u), c);
    release(&mut u, at);
}
