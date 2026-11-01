//! A click (or Tab) into a one-line field selects all of its text, so typing replaces the value:
//! the hex fields, inline fields (a fill's hex, renames), search boxes; a drag from the press
//! selects only what it covers.

use egui::{Event, Id, Key, Modifiers, Pos2, RawInput, Rect, Ui, UiBuilder, pos2, vec2};

use crate::widgets;

const FIELD: Rect = Rect { min: pos2(20.0, 20.0), max: pos2(140.0, 44.0) };

fn frame(ctx: &egui::Context, events: Vec<Event>, f: &mut impl FnMut(&mut Ui)) {
    let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0))), events, ..Default::default() };
    ctx.run_ui(input, |ui| {
        let mut child = ui.new_child(UiBuilder::new().max_rect(FIELD));
        f(&mut child);
    })
    .textures_delta
    .clear();
}

fn button(p: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

/// Moves to `at`, presses, (drags to `to`,) releases, then a few quiet frames.
fn click(ctx: &egui::Context, at: Pos2, to: Option<Pos2>, f: &mut impl FnMut(&mut Ui)) {
    frame(ctx, vec![], f);
    frame(ctx, vec![Event::PointerMoved(at)], f);
    frame(ctx, vec![button(at, true)], f);
    let end = to.unwrap_or(at);
    if let Some(to) = to {
        for k in 1..=4 {
            frame(ctx, vec![Event::PointerMoved(at + (to - at) * (k as f32 / 4.0))], f);
        }
    }
    frame(ctx, vec![button(end, false)], f);
    for _ in 0..3 {
        frame(ctx, vec![], f);
    }
}

fn selection(ctx: &egui::Context, te: Id) -> Option<(usize, usize)> {
    let r = egui::TextEdit::load_state(ctx, te)?.cursor.char_range()?;
    let (a, b): (usize, usize) = (r.primary.index.into(), r.secondary.index.into());
    Some((a.min(b), a.max(b)))
}

/// Asserts clicking a hex field focuses it and selects all six characters.
#[test]
fn a_click_into_a_hex_field_selects_it_all() {
    let ctx = egui::Context::default();
    let id = Id::unique("hex");
    let mut f = |ui: &mut Ui| {
        widgets::text_field(ui, id, "D9D9D9", 120.0, "");
    };
    click(&ctx, pos2(60.0, 32.0), None, &mut f);
    assert!(ctx.memory(|m| m.has_focus(id.with("te"))));
    assert_eq!(selection(&ctx, id.with("te")), Some((0, 6)));
}

/// Asserts clicking an inline field selects it all and typing replaces the selection.
#[test]
fn a_click_into_an_inline_field_selects_it_all() {
    // A fill's hex in the Design panel, a variable's value, a rename.
    let ctx = egui::Context::default();
    let id = Id::unique("fill_hex");
    let mut f = |ui: &mut Ui| {
        widgets::text_field_inline(ui, id, "0D99FF", FIELD.shrink2(vec2(8.0, 4.0)), egui::FontId::proportional(11.0));
    };
    click(&ctx, pos2(70.0, 32.0), None, &mut f);
    assert_eq!(selection(&ctx, id.with("te")), Some((0, 6)));
    // Typing replaces it.
    frame(&ctx, vec![Event::Text("red".into())], &mut f);
    let mut out = None;
    let mut g = |ui: &mut Ui| out = widgets::text_field_inline(ui, id, "0D99FF", FIELD.shrink2(vec2(8.0, 4.0)), egui::FontId::proportional(11.0));
    frame(&ctx, vec![Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }], &mut g);
    assert_eq!(out.as_deref(), Some("red"));
}

/// Asserts clicking a search box selects its whole query.
#[test]
fn a_click_into_a_search_box_selects_its_query() {
    let ctx = egui::Context::default();
    let id = Id::unique("search");
    let mut q = String::from("button");
    let mut f = |ui: &mut Ui| {
        widgets::text_input(ui, id, &mut q, 120.0, "Search");
    };
    click(&ctx, pos2(60.0, 32.0), None, &mut f);
    assert_eq!(selection(&ctx, id), Some((0, 6)));
}

/// Asserts Tab focus selects a field's whole value.
#[test]
fn tab_into_a_field_selects_it_all() {
    let ctx = egui::Context::default();
    let id = Id::unique("tabbed");
    let mut q = String::from("Frame 1");
    let mut f = |ui: &mut Ui| {
        let r = widgets::text_input(ui, id, &mut q, 120.0, "");
        if !r.has_focus() && ui.input(|i| i.key_pressed(Key::Tab)) {
            r.request_focus();
        }
    };
    frame(&ctx, vec![], &mut f);
    frame(&ctx, vec![Event::Key { key: Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }], &mut f);
    for _ in 0..3 {
        frame(&ctx, vec![], &mut f);
    }
    assert_eq!(selection(&ctx, id), Some((0, 7)));
}

/// Asserts a drag selects a non-empty proper subrange rather than the whole value.
#[test]
fn a_drag_selects_only_what_it_covers() {
    let ctx = egui::Context::default();
    let id = Id::unique("dragged");
    let mut q = String::from("a long search query");
    let mut f = |ui: &mut Ui| {
        widgets::text_input(ui, id, &mut q, 120.0, "");
    };
    click(&ctx, pos2(30.0, 32.0), Some(pos2(70.0, 32.0)), &mut f);
    let (a, b) = selection(&ctx, id).unwrap_or_default();
    assert!(b > a, "something is selected: {a}..{b}");
    assert!(!(a == 0 && b == q.chars().count()), "not all of it: {a}..{b}");
}

/// Asserts re-clicking a field after focus was lost mid-edit selects all.
#[test]
fn a_field_left_mid_edit_selects_all_on_the_next_click() {
    let ctx = egui::Context::default();
    let id = Id::unique("fill_hex");
    let mut f = |ui: &mut Ui| {
        widgets::text_field_inline(ui, id, "0D99FF", FIELD.shrink2(vec2(8.0, 4.0)), egui::FontId::proportional(11.0));
    };
    click(&ctx, pos2(70.0, 32.0), None, &mut f);
    // The panel goes away mid-edit (another layer selected) and a click lands elsewhere.
    click(&ctx, pos2(300.0, 200.0), None, &mut |_: &mut Ui| {});
    click(&ctx, pos2(70.0, 32.0), None, &mut f);
    assert!(ctx.memory(|m| m.has_focus(id.with("te"))));
    assert_eq!(selection(&ctx, id.with("te")), Some((0, 6)));
}
