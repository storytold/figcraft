//! A canvas pan (middle button, a side button, or Space + primary) ends when its button is
//! released, even when the release happens over a panel: returning to the canvas with no button held
//! must not move the view.

use egui::{Event, Key, PointerButton, Pos2, RawInput, Rect, vec2};
use figcraft_engine::Session;
use figcraft_ui_egui::{FigApp, NoServices};
use serde_json::json;

struct Frames {
    ctx: egui::Context,
    app: FigApp,
    time: f64,
}

impl Frames {
    fn new() -> Self {
        let mut session = Session::new();
        session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 300, "height": 200})).unwrap();
        let mut f = Frames { ctx: egui::Context::default(), app: FigApp::new(session, Box::new(NoServices)), time: 0.0 };
        for _ in 0..3 {
            f.frame(vec![]);
        }
        f
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let input =
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))), time: Some(self.time), events, ..Default::default() };
        let mut output = self.ctx.run_ui(input, |ui| self.app.ui(ui));
        output.textures_delta.clear();
    }

    fn cx(&self) -> f64 {
        self.app.session.ds().unwrap().view.cx
    }
}

fn button(pos: Pos2, button: PointerButton, pressed: bool) -> Event {
    Event::PointerButton { pos, button, pressed, modifiers: Default::default() }
}

fn space(pressed: bool) -> Event {
    Event::Key { key: Key::Space, physical_key: None, pressed, repeat: false, modifiers: Default::default() }
}

fn check(space_pan: bool, outside: bool) {
    check_button(if space_pan { PointerButton::Primary } else { PointerButton::Middle }, space_pan, outside);
}

/// A pan with button `b` (with Space held when `space_pan`) moves the view, ends when `b` is
/// released (`outside` the canvas or over it), and a new one moves it again.
fn check_button(b: PointerButton, space_pan: bool, outside: bool) {
    let mut u = Frames::new();
    let rect = u.app.canvas.rect;
    let start = Pos2::new(rect.min.x + 200.0, rect.center().y);
    let dragged = Pos2::new(rect.min.x + 600.0, rect.center().y);
    let reentered = Pos2::new(rect.min.x + 300.0, rect.center().y);
    assert!(rect.contains(start) && rect.contains(dragged) && rect.contains(reentered));
    u.frame(vec![Event::PointerMoved(start)]);
    if space_pan {
        u.frame(vec![space(true)]);
    }
    let before = u.cx();
    u.frame(vec![button(start, b, true)]);
    u.frame(vec![Event::PointerMoved(dragged)]);
    let after_drag = u.cx();
    assert_ne!(before, after_drag, "the test must start an actual pan");
    let release_at = if outside { Pos2::new(rect.min.x - 100.0, rect.center().y) } else { dragged };
    u.frame(vec![Event::PointerMoved(release_at)]);
    u.frame(vec![button(release_at, b, false)]);
    let released = u.cx();
    assert_eq!(released, after_drag, "release must leave the view unchanged");
    assert!(!u.ctx.input(|i| i.pointer.any_down()));
    u.frame(vec![Event::PointerMoved(reentered)]);
    assert_eq!(u.cx(), released, "pointer motion after release must not pan");
    u.frame(vec![Event::PointerMoved(dragged)]);
    assert_eq!(u.cx(), released, "pointer motion after release must not pan");

    // A new intentional pan still moves the view.
    u.frame(vec![button(dragged, b, true)]);
    u.frame(vec![Event::PointerMoved(reentered)]);
    assert_ne!(u.cx(), released, "a new pan after reentry must move the view");
    u.frame(vec![button(reentered, b, false)]);
}

#[test]
fn middle_pan_released_over_panel_does_not_move_view_on_return() {
    check(false, true);
}

#[test]
fn space_pan_released_over_panel_does_not_move_view_on_return() {
    check(true, true);
}

#[test]
fn middle_pan_released_inside_canvas_keeps_view() {
    check(false, false);
}

/// #1758: a mouse's side buttons (4 and 5) pan the canvas as the middle button does, and the pan
/// ends on release, over the canvas or off it.
#[test]
fn side_buttons_pan_like_the_middle_button() {
    for b in [PointerButton::Extra1, PointerButton::Extra2] {
        check_button(b, false, false);
        check_button(b, false, true);
    }
}

/// A primary drag under way when a pan starts (a side or the middle button pressed mid-drag) ends
/// when the primary button is let go during the pan: the engine gets the release, so its gesture
/// isn't left open once the pan is over.
#[test]
fn a_primary_drag_released_during_a_pan_ends() {
    for pan in [PointerButton::Middle, PointerButton::Extra1] {
        let mut u = Frames::new();
        let rect = u.app.canvas.rect;
        let start = Pos2::new(rect.min.x + 200.0, rect.center().y);
        let moved = Pos2::new(rect.min.x + 400.0, rect.center().y);
        u.frame(vec![Event::PointerMoved(start)]);
        u.frame(vec![button(start, PointerButton::Primary, true)]);
        u.frame(vec![Event::PointerMoved(moved)]);
        assert!(u.app.session.tool.busy(), "{pan:?}: a primary drag is under way");
        u.frame(vec![button(moved, pan, true)]);
        u.frame(vec![button(moved, PointerButton::Primary, false)]);
        u.frame(vec![button(moved, pan, false)]);
        u.frame(vec![]);
        assert!(!u.app.session.tool.busy(), "{pan:?}: the primary release reached the engine");
    }
}
