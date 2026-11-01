//! The side panels' widths, driven through egui headless: dragging a panel's edge sizes it within
//! its limits, a double-click puts back the default, the widths are kept in the preferences (when
//! the drag ends) and come back with them, the view's centre stays where it was, Minimize UI
//! hides the panels without losing their widths, and a window at its smallest size still has a
//! canvas.

use std::sync::{Arc, Mutex};

use egui::{Event, Pos2, RawInput, Rect, pos2, vec2};
use figcraft_engine::{Prefs, Session};
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::{CANVAS_MIN, FigApp, LEFT_PANEL, NoServices, PANEL_DEFAULT, RIGHT_PANEL, Services, fit_panels};

/// The panel widths in the preferences each time they were kept.
type Kept = Arc<Mutex<Vec<(f32, f32)>>>;

/// Services that note the preferences each time they are kept.
struct Keeps(Kept);
impl Services for Keeps {
    fn save_prefs(&mut self, prefs: &Prefs) {
        self.0.lock().unwrap().push(prefs.panel_widths);
    }
}

fn ui() -> (Ui, Kept) {
    let mut s = Session::new();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 300, "height": 200})).unwrap();
    let mut u = Ui::new(s);
    let kept = Kept::default();
    u.app.services = Box::new(Keeps(kept.clone()));
    u.frame(vec![]);
    (u, kept)
}

/// The x of the left panel's edge (the canvas's left) and of the right panel's (its right).
fn edges(u: &Ui) -> (f32, f32) {
    (u.app.canvas.rect.min.x, u.app.canvas.rect.max.x)
}

fn widths(u: &Ui) -> (f32, f32) {
    (u.app.ui.left_width, u.app.ui.right_width)
}

/// Press on `from`, move to `to` in steps and let go.
fn drag(u: &mut Ui, from: Pos2, to: Pos2) {
    u.drag_to(from, to);
    u.button(to, false);
    u.frame(vec![]);
}

fn double_click(u: &mut Ui, p: Pos2) {
    // Well after any earlier click (a third quick click, within twice the platform's 0.5 s
    // double-click window, isn't a double-click).
    for _ in 0..70 {
        u.frame(vec![]);
    }
    for _ in 0..2 {
        u.button(p, true);
        u.button(p, false);
    }
    u.frame(vec![]);
}

#[test]
fn panels_fit_the_window() {
    let (l0, l1, r0, r1) = (*LEFT_PANEL.start(), *LEFT_PANEL.end(), *RIGHT_PANEL.start(), *RIGHT_PANEL.end());
    // (room beside the icon rail, left asked (none: closed), right asked) → (left, right) drawn
    let rows = [
        ((1384.0, Some(240.0), Some(240.0)), (240.0, 240.0), "as asked in a wide window"),
        ((1384.0, Some(100.0), Some(100.0)), (l0, r0), "never narrower than their least"),
        ((1384.0, Some(900.0), Some(900.0)), (l1, r1), "never wider than their most"),
        ((1384.0, None, Some(300.0)), (0.0, 300.0), "the left panel closed"),
        ((744.0, Some(240.0), Some(240.0)), (240.0, 240.0), "the smallest window with the default widths"),
        ((744.0, Some(480.0), Some(480.0)), (l0, 744.0 - l0 - CANVAS_MIN), "the smallest window: the left panel gives way first, then the right"),
        ((744.0, Some(480.0), Some(240.0)), (744.0 - 240.0 - CANVAS_MIN, 240.0), "the left panel gives the canvas its room"),
        ((744.0, None, Some(480.0)), (0.0, 480.0), "with the left panel closed the right one has room"),
        ((300.0, Some(240.0), Some(240.0)), (l0, r0), "a window smaller than any layout: the least widths"),
        ((1384.0, Some(f32::NAN), Some(f32::INFINITY)), (PANEL_DEFAULT, PANEL_DEFAULT), "junk widths: the default"),
        ((744.0, Some(480.0), None), (480.0, 0.0), "the right panel closed (a board's): the left one keeps its width"),
    ];
    for ((avail, left, right), want, why) in rows {
        assert_eq!(fit_panels(avail, left, right), want, "{why}");
    }
    // Whatever is asked, a window of the smallest size or more keeps its canvas.
    for avail in [744.0, 900.0, 1384.0] {
        for left in [None, Some(l0), Some(300.0), Some(l1)] {
            for right in [r0, 333.0, r1] {
                let (l, r) = fit_panels(avail, left, Some(right));
                assert!(avail - l - r >= CANVAS_MIN, "{avail} {left:?} {right}: canvas {}", avail - l - r);
            }
        }
    }
}

#[test]
fn dragging_an_edge_sizes_its_panel_within_its_limits_and_keeps_the_views_centre() {
    let (mut u, kept) = ui();
    assert_eq!(widths(&u), (PANEL_DEFAULT, PANEL_DEFAULT));
    let (left, right) = edges(&u);
    let centre = {
        let v = &u.app.session.ds().unwrap().view;
        (v.cx, v.cy, v.zoom)
    };
    // The left panel's edge, 60 to the right: 60 wider, and the canvas starts 60 further on.
    drag(&mut u, pos2(left, 400.0), pos2(left + 60.0, 400.0));
    assert_eq!(widths(&u), (300.0, 240.0));
    assert_eq!(edges(&u), (left + 60.0, right));
    // The right panel's edge, 40 to the left: 40 wider.
    drag(&mut u, pos2(right, 400.0), pos2(right - 40.0, 400.0));
    assert_eq!(widths(&u), (300.0, 280.0));
    assert_eq!(edges(&u), (left + 60.0, right - 40.0));
    // The view keeps its centre and zoom: the page doesn't jump, the canvas is just narrower.
    let v = u.app.session.ds().unwrap().view.clone();
    assert_eq!((v.cx, v.cy, v.zoom), centre);
    assert_eq!(v.vw, f64::from(right - left - 100.0));
    // Kept in the preferences, once per drag (not on every move).
    assert_eq!(u.app.session.prefs.panel_widths, (300.0, 280.0));
    assert_eq!(*kept.lock().unwrap(), [(300.0, 240.0), (300.0, 280.0)]);
    // Past the limits the edge stops, and comes back with the pointer.
    let (left, right) = edges(&u);
    drag(&mut u, pos2(left, 300.0), pos2(left + 700.0, 300.0));
    assert_eq!(widths(&u).0, *LEFT_PANEL.end());
    let (left, _) = edges(&u);
    drag(&mut u, pos2(left, 300.0), pos2(20.0, 300.0));
    assert_eq!(widths(&u).0, *LEFT_PANEL.start());
    drag(&mut u, pos2(right, 300.0), pos2(right + 500.0, 300.0));
    assert_eq!(widths(&u).1, *RIGHT_PANEL.start());
    let (_, right) = edges(&u);
    u.drag_to(pos2(right, 300.0), pos2(right - 900.0, 300.0));
    assert_eq!(widths(&u).1, *RIGHT_PANEL.end(), "at its most while the pointer is past it");
    u.frame(vec![Event::PointerMoved(pos2(right - 100.0, 300.0))]);
    assert_eq!(widths(&u).1, *RIGHT_PANEL.start() + 100.0, "back with the pointer");
    u.button(pos2(right - 100.0, 300.0), false);
}

#[test]
fn a_double_click_on_an_edge_puts_back_the_default_width() {
    let (mut u, kept) = ui();
    let (left, right) = edges(&u);
    drag(&mut u, pos2(left, 400.0), pos2(left + 90.0, 400.0));
    drag(&mut u, pos2(right, 400.0), pos2(right - 90.0, 400.0));
    assert_eq!(widths(&u), (330.0, 330.0));
    let (left, right) = edges(&u);
    double_click(&mut u, pos2(left, 500.0));
    assert_eq!(widths(&u), (PANEL_DEFAULT, 330.0), "the left panel alone");
    double_click(&mut u, pos2(right, 500.0));
    assert_eq!(widths(&u), (PANEL_DEFAULT, PANEL_DEFAULT));
    assert_eq!(kept.lock().unwrap().last(), Some(&(PANEL_DEFAULT, PANEL_DEFAULT)), "kept");
    // A click on the canvas beside the edge is the canvas's.
    let (left, _) = edges(&u);
    drag(&mut u, pos2(left + 40.0, 500.0), pos2(left + 90.0, 520.0));
    assert_eq!(widths(&u), (PANEL_DEFAULT, PANEL_DEFAULT));
}

#[test]
fn the_command_sets_and_resets_the_widths() {
    let (mut u, kept) = ui();
    u.app.run("view.panelWidths", json!({"left": 320, "right": 400}));
    u.frame(vec![]);
    assert_eq!(widths(&u), (320.0, 400.0));
    assert_eq!(u.app.inspect()["panels"], json!({"left": 320.0, "right": 400.0, "leftCollapsed": false}));
    u.app.run("view.panelWidths", json!({"left": 5000}));
    assert_eq!(widths(&u), (*LEFT_PANEL.end(), 400.0), "within its limits; the other panel as it was");
    for junk in [json!({"left": "wide"}), json!({"left": null, "right": null}), json!({})] {
        u.app.run("view.panelWidths", junk.clone());
        assert_eq!(widths(&u), (PANEL_DEFAULT, PANEL_DEFAULT), "{junk}: both back to the default");
        u.app.run("view.panelWidths", json!({"left": 300, "right": 300}));
    }
    assert_eq!(kept.lock().unwrap().last(), Some(&(300.0, 300.0)));
}

#[test]
fn the_widths_come_back_with_the_preferences() {
    let mut s = Session::new();
    s.prefs.panel_widths = (360.0, 300.0);
    let mut u = Ui::new(s);
    u.frame(vec![]);
    assert_eq!(widths(&u), (360.0, 300.0));
    let (left, right) = edges(&u);
    assert_eq!(right - left, 1440.0 - 56.0 - 360.0 - 300.0, "the canvas between them (the icon rail is 56 wide)");
    // Widths from a damaged file are brought within the limits when drawn.
    let mut s = Session::new();
    s.prefs.panel_widths = (-5.0, f32::NAN);
    let mut u = Ui::new(s);
    u.frame(vec![]);
    let (left, right) = edges(&u);
    assert_eq!((left, 1440.0 - right), (56.0 + *LEFT_PANEL.start(), PANEL_DEFAULT));
}

#[test]
fn minimize_ui_hides_the_panels_and_keeps_their_widths() {
    let (mut u, _) = ui();
    u.app.run("view.panelWidths", json!({"left": 320, "right": 400}));
    u.frame(vec![]);
    let shown = edges(&u);
    u.app.run("view.toggleUi", json!({}));
    u.frame(vec![]);
    assert_eq!(edges(&u), (0.0, 1440.0), "the canvas has the window");
    // No edge to drag where the panels were.
    drag(&mut u, pos2(shown.0, 400.0), pos2(shown.0 + 80.0, 400.0));
    assert_eq!(widths(&u), (320.0, 400.0));
    u.app.run("view.toggleUi", json!({}));
    u.frame(vec![]);
    assert_eq!((edges(&u), widths(&u)), (shown, (320.0, 400.0)), "back as they were");
    // The left sidebar alone: its edge goes with it, the right panel stays.
    u.app.run("view.leftSidebar", json!({}));
    u.frame(vec![]);
    assert_eq!(edges(&u), (56.0, shown.1));
}

#[test]
fn the_smallest_window_keeps_its_canvas_whatever_the_widths() {
    for asked in [(240.0, 240.0), (480.0, 480.0), (480.0, 240.0), (200.0, 480.0)] {
        let mut s = Session::new();
        s.prefs.panel_widths = asked;
        let ctx = egui::Context::default();
        let mut app = FigApp::new(s, Box::new(NoServices));
        // The desktop window's minimum inner size.
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 500.0));
        for k in 0..5 {
            let input = RawInput { screen_rect: Some(screen), time: Some(f64::from(k) / 60.0), ..Default::default() };
            ctx.run_ui(input, |ui| app.ui(ui)).textures_delta.clear();
        }
        let canvas = app.canvas.rect;
        assert!(canvas.width() >= CANVAS_MIN, "{asked:?}: the canvas is {} wide", canvas.width());
        assert!(screen.contains_rect(canvas), "{asked:?}: the canvas {canvas:?} is in the window");
        assert!(canvas.height() > 300.0, "{asked:?}: and has its height");
        assert_eq!((app.ui.left_width, app.ui.right_width), asked, "the widths asked for come back in a wider window");
    }
}

/// A phone-sized window: the canvas has the whole width, and each side panel opens alone over it,
/// leaving the strip with the panel buttons. A wide window ignores the choice.
#[test]
fn a_narrow_window_shows_one_side_panel_at_a_time() {
    use crate::MobilePanel;
    let ctx = egui::Context::default();
    let mut app = FigApp::new(Session::new(), Box::new(NoServices));
    let mut k = 0.0;
    let mut canvas_width = |app: &mut FigApp, size: egui::Vec2| {
        for _ in 0..4 {
            k += 1.0 / 60.0;
            let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), time: Some(k), ..Default::default() };
            ctx.run_ui(input, |ui| app.ui(ui)).textures_delta.clear();
        }
        app.canvas.rect.width()
    };
    let phone = vec2(390.0, 844.0);
    assert!(app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 300, "height": 200})).is_ok());
    assert!(canvas_width(&mut app, phone) >= 380.0, "closed panels leave the whole width");
    for side in [MobilePanel::Left, MobilePanel::Right] {
        app.ui.mobile_panel = Some(side);
        let w = canvas_width(&mut app, phone);
        assert!((1.0..=60.0).contains(&w), "{side:?}: the open panel leaves {w}");
    }
    // A command that opens the left panel opens it on a phone too, in place of the right one.
    app.run("view.assets", json!({}));
    let w = canvas_width(&mut app, phone);
    assert!((1.0..=60.0).contains(&w) && app.ui.mobile_panel == Some(MobilePanel::Left), "view.assets leaves {w}");
    app.ui.mobile_panel = Some(MobilePanel::Right);
    let wide = canvas_width(&mut app, vec2(1440.0, 900.0));
    assert!(wide > 600.0 && wide < 1100.0, "a wide window shows both panels beside the canvas ({wide})");
}
