//! Right-to-left and mixed-direction names: elided display and the rename field's visual caret,
//! driven through egui headless.

use egui::{Event, Key, Modifiers, Pos2, RawInput, Rect, vec2};

use super::*;
use crate::theme::regular;

fn frame(ctx: &egui::Context, events: Vec<Event>, mut f: impl FnMut(&mut Ui)) {
    let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), events, ..Default::default() };
    ctx.run_ui(input, |ui| f(ui)).textures_delta.clear();
}

fn key(k: Key, shift: bool) -> Event {
    Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: if shift { Modifiers::SHIFT } else { Modifiers::NONE } }
}

fn caret(ctx: &egui::Context, id: Id) -> (usize, usize) {
    let r = egui::TextEdit::load_state(ctx, id).and_then(|s| s.cursor.char_range()).unwrap_or_default();
    (r.primary.index.0, r.secondary.index.0)
}

fn set_caret(ctx: &egui::Context, id: Id, i: usize) {
    let mut s = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    s.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(i))));
    s.store(ctx, id);
}

#[test]
fn names_elide_at_their_end() {
    let ctx = egui::Context::default();
    frame(&ctx, vec![], |ui| {
        let p = ui.painter();
        let g = name_galley(p, "Rectangle with a rather long name", regular(11.0), Color32::WHITE, 60.0);
        assert!(g.elided);
        assert!(g.size().x <= 61.0);
        let g = name_galley(p, "Frame", regular(11.0), Color32::WHITE, 200.0);
        assert_eq!(g.text(), "Frame");
        // Right to left: shown in visual order; cut names keep their start, at the right.
        let g = name_galley(p, "שלום", regular(11.0), Color32::WHITE, 500.0);
        assert_eq!(g.text(), "םולש");
        let g = name_galley(p, "שלום עולם ומלואו", regular(11.0), Color32::WHITE, 40.0);
        assert!(g.text().starts_with('…'), "{}", g.text());
        assert!(g.size().x <= 41.0);
        // Arabic letters join.
        let g = name_galley(p, "سلام", regular(11.0), Color32::WHITE, 500.0);
        assert_eq!(g.text(), "\u{FEE1}\u{FEFC}\u{FEB3}");
    });
}

#[test]
fn long_tab_names_fade_at_the_width_without_an_ellipsis() {
    let ctx = egui::Context::default();
    let name = "A filename that is much too long for one tab";
    let mut long_rect = egui::Rect::NOTHING;
    let mut short_rect = egui::Rect::NOTHING;
    let mut output = ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() }, |ui| {
        let painter = ui.painter();
        long_rect = paint_name_faded(painter, egui::pos2(20.0, 30.0), name, regular(12.0), Color32::BLACK, 60.0, Color32::WHITE);
        short_rect = paint_name_faded(painter, egui::pos2(20.0, 60.0), "Short", regular(12.0), Color32::BLACK, 60.0, Color32::WHITE);
    });
    output.textures_delta.clear();
    assert_eq!(long_rect.width(), 60.0);
    assert!(short_rect.width() < 60.0);
    assert!(
        output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == name)),
        "paint the full name instead of replacing its end with an ellipsis"
    );
    assert!(
        output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Mesh(mesh) if mesh.vertices.iter().any(|vertex| vertex.color.a() == 0) && mesh.vertices.iter().any(|vertex| vertex.color.a() == 255))),
        "overlay a transparent-to-background gradient on the clipped end"
    );
}

#[test]
fn long_rtl_tab_names_keep_their_logical_start_and_fade_at_the_left() {
    let ctx = egui::Context::default();
    let name = "שלום עולם ומלואו";
    let visual = display::visual_line(name).text;
    let mut output = ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() }, |ui| {
        paint_name_faded(ui.painter(), egui::pos2(20.0, 30.0), name, regular(12.0), Color32::BLACK, 40.0, Color32::WHITE);
    });
    output.textures_delta.clear();

    let text = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.text() == visual => Some(text),
        _ => None,
    });
    let Some(text) = text else { panic!("paint the full RTL name in visual order") };
    assert!(text.pos.x < 20.0, "right-align the RTL galley to preserve its logical start");
    assert!(
        output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Mesh(mesh)
                if mesh.vertices[0].pos.x == 20.0
                    && mesh.vertices[0].color.a() == 0
                    && mesh.vertices[1].pos.x > mesh.vertices[0].pos.x
                    && mesh.vertices[1].color.a() == 255)
        }),
        "fade the left edge of an RTL tab name"
    );
}

#[test]
fn rename_caret_moves_visually() {
    let ctx = egui::Context::default();
    let id = Id::new("rename_test");
    let mut text = "abc אבג".to_string();
    let run = |events: Vec<Event>, text: &mut String| {
        frame(&ctx, events, |ui| {
            ui.memory_mut(|m| m.request_focus(id));
            single_line(ui, id, text, regular(11.0), 200.0);
        });
    };
    run(vec![], &mut text);
    run(vec![], &mut text);
    set_caret(&ctx, id, 3);
    // Right from the space: into the Hebrew word at its visual left, which is its logical end.
    run(vec![key(Key::ArrowRight, false)], &mut text);
    assert_eq!(caret(&ctx, id).0, 7);
    run(vec![key(Key::ArrowRight, false)], &mut text);
    assert_eq!(caret(&ctx, id).0, 6);
    // Shift extends from where the caret was.
    run(vec![key(Key::ArrowRight, true)], &mut text);
    assert_eq!(caret(&ctx, id), (5, 6));
    // Without Shift a selection collapses to its side.
    run(vec![key(Key::ArrowLeft, false)], &mut text);
    assert_eq!(caret(&ctx, id), (6, 6));
    run(vec![key(Key::ArrowLeft, false), key(Key::ArrowLeft, false)], &mut text);
    assert_eq!(caret(&ctx, id).0, 3);
    // Typing still goes in at the caret.
    run(vec![Event::Text("ד".into())], &mut text);
    assert_eq!(text, "abcד אבג");
    // Plain left-to-right text keeps egui's own caret.
    let mut latin = "Frame".to_string();
    let lid = Id::new("latin");
    let mut run = |events: Vec<Event>| {
        frame(&ctx, events, |ui| {
            ui.memory_mut(|m| m.request_focus(lid));
            single_line(ui, lid, &mut latin, regular(11.0), 200.0);
        });
    };
    run(vec![]);
    run(vec![]);
    set_caret(&ctx, lid, 1);
    run(vec![key(Key::ArrowRight, false)]);
    assert_eq!(caret(&ctx, lid).0, 2);
}

#[test]
fn clicks_place_the_caret_by_what_is_shown() {
    let ctx = egui::Context::default();
    let id = Id::new("click_test");
    let mut text = "שלום".to_string();
    let mut rect = Rect::NOTHING;
    let run = |events: Vec<Event>, text: &mut String, rect: &mut Rect| {
        frame(&ctx, events, |ui| {
            ui.memory_mut(|m| m.request_focus(id));
            *rect = single_line(ui, id, text, regular(11.0), 200.0).rect;
        });
    };
    run(vec![], &mut text, &mut rect);
    run(vec![], &mut text, &mut rect);
    // The text's right end is its start.
    let galley = ctx.fonts_mut(|f| f.layout_no_wrap("םולש".into(), regular(11.0), Color32::WHITE));
    let right = Pos2::new(rect.min.x + galley.size().x - 1.0, rect.center().y);
    let press = |p: Pos2, pressed: bool| Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    run(vec![Event::PointerMoved(right), press(right, true)], &mut text, &mut rect);
    run(vec![press(right, false)], &mut text, &mut rect);
    assert_eq!(caret(&ctx, id).0, 0);
    let left = Pos2::new(rect.min.x + 1.0, rect.center().y);
    run(vec![Event::PointerMoved(left), press(left, true)], &mut text, &mut rect);
    run(vec![press(left, false)], &mut text, &mut rect);
    assert_eq!(caret(&ctx, id).0, 4);
}

#[test]
fn words_for_double_click() {
    assert_eq!(word_at("abc אבג", 5), (4, 7));
    assert_eq!(word_at("abc אבג", 7), (4, 7));
    assert_eq!(word_at("abc אבג", 1), (0, 3));
    assert_eq!(word_at("a  b", 2), (2, 3));
    assert_eq!(word_at("", 3), (0, 0));
}

/// egui shapes runs of one font itself (reversing right-to-left ones): shown names are laid out a
/// cluster at a time, in the order computed here, one glyph per shown char.
#[test]
fn shown_glyphs_keep_the_visual_order() {
    let ctx = egui::Context::default();
    frame(&ctx, vec![], |ui| {
        for name in
            ["\u{05D0}\u{05D1}", "abc \u{05D0}\u{05D1}\u{05D2} (1)", "\u{0633}\u{0644}\u{0627}\u{0645} \u{0639}\u{0644}\u{064A}\u{0643}\u{0645}"]
        {
            let v = display::visual_line(name);
            let g = visual_galley(ui.painter(), &v, &regular(11.0), Color32::WHITE);
            let glyphs: Vec<(char, f32)> = g.rows.iter().flat_map(|r| r.row.glyphs.iter().map(|gl| (gl.chr, gl.pos.x))).collect();
            assert_eq!(glyphs.iter().map(|g| g.0).collect::<String>(), v.text, "{name}");
            assert!(glyphs.windows(2).all(|w| w[0].1 <= w[1].1), "{name}: left to right");
        }
    });
}
