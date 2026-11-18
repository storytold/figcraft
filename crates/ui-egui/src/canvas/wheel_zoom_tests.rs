//! Preferences ▸ Scroll wheel zoom: a mouse wheel zooms the canvas about the pointer and
//! Cmd/Ctrl+wheel scrolls it; Shift+wheel still scrolls sideways, and trackpads and pinches are
//! unchanged. Off (the default), a wheel scrolls and Cmd/Ctrl+wheel zooms.

use egui::{Event, Modifiers, MouseWheelSource, MouseWheelUnit, Pos2, RawInput, Rect, TouchPhase, Vec2, vec2};
use figcraft_engine::Session;
use serde_json::json;

use super::input::{invert_zoom_factor, is_mouse_wheel, swap_wheel};
use crate::{FigApp, NoServices};

/// The view's zoom, cx and cy.
type View = (f64, f64, f64);

struct Canvas {
    ctx: egui::Context,
    app: FigApp,
    time: f64,
}

/// A notch of a mouse wheel (lines, as Windows and Linux report one) or a trackpad scroll (points).
fn wheel(unit: MouseWheelUnit, dy: f32, modifiers: Modifiers) -> Event {
    Event::MouseWheel { unit, source: MouseWheelSource::Unknown, delta: vec2(0.0, dy), phase: TouchPhase::Move, modifiers }
}

impl Canvas {
    fn new(scroll_wheel_zoom: bool) -> Self {
        Self::with_invert(scroll_wheel_zoom, false)
    }

    fn with_invert(scroll_wheel_zoom: bool, invert_zoom: bool) -> Self {
        let mut c = Self { ctx: egui::Context::default(), app: FigApp::new(Session::new(), Box::new(NoServices)), time: 0.0 };
        c.app.run("pref.scrollWheelZoom", json!({"on": scroll_wheel_zoom}));
        c.app.run("pref.invertZoom", json!({"on": invert_zoom}));
        for _ in 0..3 {
            c.frame(vec![], Modifiers::NONE);
        }
        c
    }

    /// The middle of the canvas (of the window until the canvas is laid out).
    fn centre(&self) -> Pos2 {
        let r = self.app.canvas.rect;
        if r.is_positive() { r.center() } else { Pos2::new(720.0, 450.0) }
    }

    fn frame(&mut self, mut events: Vec<Event>, modifiers: Modifiers) {
        self.time += 1.0 / 60.0;
        events.splice(0..0, [Event::ModifiersChanged(modifiers), Event::PointerMoved(self.centre())]);
        let input =
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))), time: Some(self.time), events, ..Default::default() };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(input, |ui| app.ui(ui));
        out.textures_delta.clear();
    }

    /// `event` over the middle of the canvas, then frames until egui's smoothing settles; the
    /// view's (zoom, cx, cy) before and after, and the page point under the pointer before and after.
    fn gesture(&mut self, event: Event, modifiers: Modifiers) -> (View, View, (Vec2, Vec2)) {
        let view = |c: &Self| c.app.session.ds().map(|d| (d.view.zoom, d.view.cx, d.view.cy)).unwrap();
        let under = |c: &Self| {
            let p = c.app.to_page(c.centre());
            vec2(p.x as f32, p.y as f32)
        };
        let (before, at) = (view(self), under(self));
        self.frame(vec![event], modifiers);
        for _ in 0..40 {
            self.frame(vec![], modifiers);
        }
        (before, view(self), (at, under(self)))
    }
}

#[test]
fn off_a_wheel_scrolls_and_cmd_or_ctrl_wheel_zooms() {
    let mut c = Canvas::new(false);
    let (a, b, _) = c.gesture(wheel(MouseWheelUnit::Line, -3.0, Modifiers::NONE), Modifiers::NONE);
    assert_eq!(a.0, b.0, "no zoom");
    assert!(b.2 > a.2 + 1.0, "scrolled down: {a:?} → {b:?}");
    let (a, b, _) = c.gesture(wheel(MouseWheelUnit::Line, 3.0, Modifiers::COMMAND), Modifiers::COMMAND);
    assert!(b.0 > a.0 * 1.05, "zoomed in: {a:?} → {b:?}");
}

#[test]
fn on_a_wheel_zooms_about_the_pointer_and_cmd_or_ctrl_wheel_scrolls() {
    let mut c = Canvas::new(true);
    let (a, b, (at, after)) = c.gesture(wheel(MouseWheelUnit::Line, 3.0, Modifiers::NONE), Modifiers::NONE);
    assert!(b.0 > a.0 * 1.05, "zoomed in: {a:?} → {b:?}");
    assert!((at - after).length() < 0.5, "about the pointer: {at:?} → {after:?}");
    let (a, b, _) = c.gesture(wheel(MouseWheelUnit::Line, -3.0, Modifiers::NONE), Modifiers::NONE);
    assert!(b.0 < a.0 / 1.05, "zoomed out: {a:?} → {b:?}");
    let (a, b, _) = c.gesture(wheel(MouseWheelUnit::Line, -3.0, Modifiers::COMMAND), Modifiers::COMMAND);
    assert!((b.0 - a.0).abs() < 1e-9 && b.2 > a.2 + 1.0 && (b.1 - a.1).abs() < 1e-9, "scrolled down: {a:?} → {b:?}");
    let (a, b, _) = c.gesture(wheel(MouseWheelUnit::Line, -3.0, Modifiers::SHIFT), Modifiers::SHIFT);
    assert!((b.0 - a.0).abs() < 1e-9 && b.1 > a.1 + 1.0 && (b.2 - a.2).abs() < 1e-9, "scrolled sideways: {a:?} → {b:?}");
}

#[test]
fn on_trackpads_and_pinches_are_unchanged() {
    let mut c = Canvas::new(true);
    let (a, b, _) = c.gesture(wheel(MouseWheelUnit::Point, -6.0, Modifiers::NONE), Modifiers::NONE);
    assert!((b.0 - a.0).abs() < 1e-9 && b.2 > a.2, "two fingers pan: {a:?} → {b:?}");
    let (a, b, _) = c.gesture(wheel(MouseWheelUnit::Point, 6.0, Modifiers::COMMAND), Modifiers::COMMAND);
    assert!(b.0 > a.0, "Cmd/Ctrl+two fingers zoom: {a:?} → {b:?}");
    // A pinch right after a mouse wheel is still a pinch.
    c.gesture(wheel(MouseWheelUnit::Line, 1.0, Modifiers::NONE), Modifiers::NONE);
    let (a, b, _) = c.gesture(Event::Zoom(1.5), Modifiers::NONE);
    assert!((b.0 / a.0 - 1.5).abs() < 1e-3, "pinch: {a:?} → {b:?}");
}

#[test]
fn which_scrolls_count_as_a_mouse_wheel() {
    assert!(is_mouse_wheel(MouseWheelSource::Wheel, MouseWheelUnit::Point));
    assert!(is_mouse_wheel(MouseWheelSource::Unknown, MouseWheelUnit::Line));
    assert!(is_mouse_wheel(MouseWheelSource::Unknown, MouseWheelUnit::Page));
    assert!(!is_mouse_wheel(MouseWheelSource::Unknown, MouseWheelUnit::Point));
    assert!(!is_mouse_wheel(MouseWheelSource::Trackpad, MouseWheelUnit::Line));
    assert!(!is_mouse_wheel(MouseWheelSource::Momentum, MouseWheelUnit::Point));
}

#[test]
fn the_swap_keeps_egui_rates() {
    let speed = 1.0 / 200.0;
    // Cmd/Ctrl+wheel: egui's zoom back into the scroll it came from.
    let (s, z) = swap_wheel(Vec2::ZERO, (speed * 40.0f32).exp(), false, speed);
    assert!((s.y - 40.0).abs() < 1e-3 && s.x == 0.0 && z == 1.0);
    // A plain wheel: the zoom egui gives Cmd/Ctrl+wheel.
    let (s, z) = swap_wheel(vec2(0.0, 40.0), 1.0, false, speed);
    assert!(s == Vec2::ZERO && (z - (speed * 40.0f32).exp()).abs() < 1e-6);
    // Shift+wheel and nothing at all are left alone.
    assert_eq!(swap_wheel(vec2(40.0, 0.0), 1.0, true, speed), (vec2(40.0, 0.0), 1.0));
    assert_eq!(swap_wheel(Vec2::ZERO, 1.0, false, speed), (Vec2::ZERO, 1.0));
}

#[test]
fn invert_zoom_reverses_cmd_or_ctrl_wheel_by_the_same_factor() {
    for mods in [Modifiers::COMMAND, Modifiers::CTRL] {
        let (mut off, mut on) = (Canvas::with_invert(false, false), Canvas::with_invert(false, true));
        let (a, b, _) = off.gesture(wheel(MouseWheelUnit::Line, 3.0, mods), mods);
        let (c, d, _) = on.gesture(wheel(MouseWheelUnit::Line, 3.0, mods), mods);
        assert!(b.0 > a.0 * 1.05, "{mods:?} off zooms in: {a:?} -> {b:?}");
        assert!(d.0 < c.0 / 1.05, "{mods:?} on zooms out: {c:?} -> {d:?}");
        assert!(((b.0 / a.0) * (d.0 / c.0) - 1.0).abs() < 1e-3, "{mods:?} by the same factor");
    }
}

#[test]
fn invert_zoom_reverses_a_plain_mouse_wheel_with_scroll_wheel_zoom_on() {
    let (mut off, mut on) = (Canvas::with_invert(true, false), Canvas::with_invert(true, true));
    let (a, b, _) = off.gesture(wheel(MouseWheelUnit::Line, 3.0, Modifiers::NONE), Modifiers::NONE);
    let (c, d, _) = on.gesture(wheel(MouseWheelUnit::Line, 3.0, Modifiers::NONE), Modifiers::NONE);
    assert!(b.0 > a.0 * 1.05, "off zooms in: {a:?} -> {b:?}");
    assert!(d.0 < c.0 / 1.05, "on zooms out: {c:?} -> {d:?}");
    assert!(((b.0 / a.0) * (d.0 / c.0) - 1.0).abs() < 1e-3, "by the same factor");
}

/// Panning is never inverted: the scroll vector is the same with the preference on and off.
#[test]
fn invert_zoom_leaves_panning_unchanged() {
    // (Scroll wheel zoom, modifiers): a plain wheel without it, Cmd/Ctrl+wheel with it, Shift+wheel.
    for (swz, mods) in [(false, Modifiers::NONE), (true, Modifiers::COMMAND), (true, Modifiers::CTRL), (true, Modifiers::SHIFT)] {
        let (mut off, mut on) = (Canvas::with_invert(swz, false), Canvas::with_invert(swz, true));
        let (a, b, _) = off.gesture(wheel(MouseWheelUnit::Line, -3.0, mods), mods);
        let (c, d, _) = on.gesture(wheel(MouseWheelUnit::Line, -3.0, mods), mods);
        assert!((b.0 - a.0).abs() < 1e-9 && (d.0 - c.0).abs() < 1e-9, "{mods:?} no zoom");
        assert!(b.1 != a.1 || b.2 != a.2, "{mods:?} off pans: {a:?} -> {b:?}");
        assert!(
            ((b.1 - a.1) - (d.1 - c.1)).abs() < 1e-9 && ((b.2 - a.2) - (d.2 - c.2)).abs() < 1e-9,
            "{mods:?} same pan: {a:?} -> {b:?} vs {c:?} -> {d:?}"
        );
    }
}

#[test]
fn a_pinch_zooms_the_same_with_invert_zoom_on_and_off() {
    for (swz, invert, factor) in
        [(false, false, 1.5), (false, true, 1.5), (true, false, 1.5), (true, true, 1.5), (true, true, 0.5), (false, true, 0.5)]
    {
        let mut c = Canvas::with_invert(swz, invert);
        let (a, b, _) = c.gesture(Event::Zoom(factor), Modifiers::NONE);
        assert!((b.0 / a.0 - f64::from(factor)).abs() < 1e-3, "scroll wheel zoom {swz}, invert {invert}, x{factor}: {a:?} -> {b:?}");
    }
}

#[test]
fn the_inverted_factor_is_the_reciprocal_and_guards_bad_values() {
    assert!((invert_zoom_factor(2.0) - 0.5).abs() < 1e-6);
    assert_eq!(invert_zoom_factor(1.0), 1.0);
    for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let r = invert_zoom_factor(bad);
        assert!(r == bad || (r.is_nan() && bad.is_nan()), "{bad} left alone");
    }
}
