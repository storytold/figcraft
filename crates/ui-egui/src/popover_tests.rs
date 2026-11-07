//! The shared popover drawn on its own: 240 points wide, a 40-high header with the title 16 in
//! from the left and the X 8 in from the right, the slot under a hairline, the opening rule
//! against an anchor (right edge on it, top aligned, lifted when that would run off the screen),
//! the header dragging it, and closing on the X, Escape and a press outside — but not on a press
//! in a held-out rect, so the button that opened it can toggle it shut.

use egui::{Event, Key, PointerButton, Pos2, RawInput, Rect, pos2, vec2};

use crate::popover::{HEADER_H, Hold, Open, Popover, WIDTH};

const SCREEN: egui::Vec2 = vec2(1440.0, 900.0);

/// A context with the UI's fonts: the header's title is drawn in the semibold family.
fn context() -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx, false, 0);
    ctx
}

fn input(t: &mut f64, events: Vec<Event>) -> RawInput {
    *t += 1.0 / 60.0;
    RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)), time: Some(*t), events, ..Default::default() }
}

fn press(p: Pos2, pressed: bool) -> Vec<Event> {
    vec![Event::PointerMoved(p), Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Default::default() }]
}

fn escape() -> Vec<Event> {
    vec![Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]
}

/// One frame of `p` opened at `open`, its slot holding a row: the shapes it painted and whether it
/// asked to close.
fn draw(ctx: &egui::Context, t: &mut f64, p: &mut Popover, open: Open, hold: Hold<'_>, events: Vec<Event>) -> (Vec<egui::Shape>, bool) {
    let mut closed = false;
    let mut out = ctx.run_ui(input(t, events), |ui| {
        closed = p.show(ui.ctx(), open, hold, |ui| {
            ui.label("slot");
        });
    });
    out.textures_delta.clear();
    (crate::live_tests::flat(&out.shapes), closed)
}

/// Two frames: the first lays the panel out to learn its height, the second places and paints it.
fn settle(ctx: &egui::Context, t: &mut f64, p: &mut Popover, open: Open) -> Vec<egui::Shape> {
    draw(ctx, t, p, open, Hold::default(), vec![]);
    draw(ctx, t, p, open, Hold::default(), vec![]).0
}

fn near(a: Pos2, b: Pos2) -> bool {
    (a - b).length() < 0.05
}

#[test]
fn the_header_is_40_high_with_the_title_16_in_and_the_x_8_in() {
    let ctx = context();
    let mut t = 0.0;
    let mut p = Popover::new("popover_header", "Custom");
    let shapes = settle(&ctx, &mut t, &mut p, Open::At(pos2(160.0, 100.0)));
    assert_eq!(p.rect.width(), WIDTH, "240 points wide");
    assert!(near(p.rect.min, pos2(160.0, 100.0)), "where it was put: {:?}", p.rect);
    assert_eq!(p.header.height(), HEADER_H, "the header is 40 high");
    // The X: 24 square, its right edge 8 in from the panel's, centred on the header's line.
    assert!((p.close.max.x - (p.rect.max.x - 8.0)).abs() < 0.01, "the X is 8 in from the right: {:?}", p.close);
    assert!((p.close.center().y - p.header.center().y).abs() < 0.01, "the X is on the header's line");
    // The title: one line, 16 in from the left, on the header's middle.
    let (_, at, size) = crate::live_tests::texts(&shapes).into_iter().find(|(s, _, _)| s == "Custom").expect("the title shows");
    assert!((at.x - (p.rect.min.x + 16.0)).abs() < 0.5, "the title starts 16 in: {at:?}");
    assert!((at.y + size.y / 2.0 - p.header.center().y).abs() < 1.0, "the title is on the header's line: {at:?}");
    // The slot: 16 in from the panel's sides, under the header.
    assert!((p.slot.min.x - (p.rect.min.x + 16.0)).abs() < 0.01, "the slot starts 16 in: {:?}", p.slot);
    assert!((p.slot.min.y - (p.header.max.y + 16.0)).abs() < 0.01, "the slot starts under the header: {:?}", p.slot);
}

#[test]
fn it_opens_with_its_right_edge_on_the_anchor_and_its_top_aligned() {
    let ctx = context();
    let mut t = 0.0;
    let mut p = Popover::new("popover_anchor", "Popup");
    settle(&ctx, &mut t, &mut p, Open::Anchor(pos2(900.0, 200.0)));
    assert!((p.rect.max.x - 900.0).abs() < 0.01, "its right edge on the anchor: {:?}", p.rect);
    assert!((p.rect.min.y - 200.0).abs() < 0.01, "its top on the anchor: {:?}", p.rect);
}

#[test]
fn it_lifts_so_its_bottom_keeps_16_from_the_screens_bottom() {
    let ctx = context();
    let mut t = 0.0;
    let mut p = Popover::new("popover_lift", "Popup");
    settle(&ctx, &mut t, &mut p, Open::Anchor(pos2(900.0, 860.0)));
    // Aligning the top would run it off: it rises so its bottom sits 16 above the screen's.
    assert!((p.rect.max.y - (SCREEN.y - 16.0)).abs() < 0.01, "its bottom is 16 above the screen's: {:?}", p.rect);
    assert!(p.rect.min.y < 860.0, "it was lifted: {:?}", p.rect);
}

#[test]
fn the_header_drags_it_and_the_slot_does_not() {
    let ctx = context();
    let mut t = 0.0;
    let mut p = Popover::new("popover_drag", "Popup");
    settle(&ctx, &mut t, &mut p, Open::At(pos2(160.0, 100.0)));
    let at = Open::At(pos2(160.0, 100.0));
    let (from, to) = (pos2(200.0, 120.0), pos2(250.0, 180.0));
    let before = p.rect.min;
    draw(&ctx, &mut t, &mut p, at, Hold::default(), press(from, true));
    draw(&ctx, &mut t, &mut p, at, Hold::default(), vec![Event::PointerMoved(to)]);
    draw(&ctx, &mut t, &mut p, at, Hold::default(), vec![]);
    draw(&ctx, &mut t, &mut p, at, Hold::default(), press(to, false));
    draw(&ctx, &mut t, &mut p, at, Hold::default(), vec![]);
    assert!(near(p.rect.min, before + (to - from)), "the header dragged it by the pointer's move: {:?} from {before:?}", p.rect.min);
    // The slot does not drag it.
    let (from, to) = (p.slot.center(), p.slot.center() + vec2(-70.0, -40.0));
    let held = p.rect.min;
    draw(&ctx, &mut t, &mut p, at, Hold::default(), press(from, true));
    draw(&ctx, &mut t, &mut p, at, Hold::default(), vec![Event::PointerMoved(to)]);
    draw(&ctx, &mut t, &mut p, at, Hold::default(), press(to, false));
    draw(&ctx, &mut t, &mut p, at, Hold::default(), vec![]);
    assert!(near(p.rect.min, held), "a press in the slot leaves it: {:?} from {held:?}", p.rect.min);
}

#[test]
fn the_x_and_escape_close_it() {
    let ctx = context();
    let mut t = 0.0;
    let mut p = Popover::new("popover_close", "Popup");
    settle(&ctx, &mut t, &mut p, Open::At(pos2(160.0, 100.0)));
    let at = Open::At(pos2(160.0, 100.0));
    let close = p.close.center();
    assert!(!draw(&ctx, &mut t, &mut p, at, Hold::default(), press(close, true)).1, "a press is not a click yet");
    assert!(draw(&ctx, &mut t, &mut p, at, Hold::default(), press(close, false)).1, "the X asks for the close");
    assert!(draw(&ctx, &mut t, &mut p, at, Hold::default(), escape()).1, "Escape asks for the close");
    // Escape that belongs to something above it does not.
    let mut p = Popover::new("popover_escape_held", "Popup");
    settle(&ctx, &mut t, &mut p, Open::At(pos2(160.0, 100.0)));
    let hold = Hold { rects: &[], escape: true };
    assert!(!draw(&ctx, &mut t, &mut p, at, hold, escape()).1, "a held Escape leaves it open");
}

#[test]
fn a_press_outside_closes_it_but_not_on_the_button_that_opened_it() {
    let ctx = context();
    let mut t = 0.0;
    let mut p = Popover::new("popover_outside", "Popup");
    settle(&ctx, &mut t, &mut p, Open::At(pos2(300.0, 200.0)));
    let at = Open::At(pos2(300.0, 200.0));
    // The button that opened it: a press there leaves it open, so its release can toggle it shut.
    let opener = Rect::from_min_size(pos2(60.0, 60.0), vec2(24.0, 24.0));
    let hold = Hold { rects: &[opener], escape: false };
    let on_opener = opener.center();
    assert!(!draw(&ctx, &mut t, &mut p, at, hold, press(on_opener, true)).1, "a press on the opener leaves it open");
    assert!(!draw(&ctx, &mut t, &mut p, at, hold, press(on_opener, false)).1, "and so does its release");
    // A press anywhere else closes it, before the release.
    let elsewhere = pos2(900.0, 400.0);
    assert!(draw(&ctx, &mut t, &mut p, at, hold, press(elsewhere, true)).1, "a press outside closes it");
}

#[test]
fn a_caller_can_put_it_where_it_likes() {
    let ctx = context();
    let mut t = 0.0;
    // Placed outright, the opening rule no longer matters.
    let mut p = Popover::new("popover_at", "Popup").at(pos2(300.0, 250.0));
    settle(&ctx, &mut t, &mut p, Open::At(pos2(900.0, 100.0)));
    assert!(near(p.rect.min, pos2(300.0, 250.0)), "where the caller put it: {:?}", p.rect);
    // Nor after a drag: it stays where it was dropped.
    let at = Open::At(pos2(900.0, 100.0));
    let (from, to) = (pos2(340.0, 270.0), pos2(400.0, 320.0));
    draw(&ctx, &mut t, &mut p, at, Hold::default(), press(from, true));
    draw(&ctx, &mut t, &mut p, at, Hold::default(), vec![Event::PointerMoved(to)]);
    draw(&ctx, &mut t, &mut p, at, Hold::default(), press(to, false));
    draw(&ctx, &mut t, &mut p, at, Hold::default(), vec![]);
    assert!(near(p.rect.min, pos2(360.0, 300.0)), "dropped where the drag left it: {:?}", p.rect);
}
