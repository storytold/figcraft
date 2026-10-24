//! Panel dropdowns (the prototype device picker, Preferences, blend modes…) draw on the theme's
//! popup fill, so their rows' text must read on it in both themes (#972): drawn headless.

use egui::{Color32, Event, Order, PointerButton, Pos2, RawInput, Rect, Shape, pos2, vec2};

use crate::right::{menu_check, menu_item, menu_popup};
use crate::theme::{self, Tokens};

fn input(t: &mut f64, events: Vec<Event>) -> RawInput {
    *t += 1.0 / 60.0;
    RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), time: Some(*t), events, ..Default::default() }
}

fn press(p: Pos2, pressed: bool) -> Vec<Event> {
    vec![Event::PointerMoved(p), Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Default::default() }]
}

fn color(x: Color32) -> figcraft_color::Color {
    figcraft_color::Color::new(f32::from(x.r()) / 255.0, f32::from(x.g()) / 255.0, f32::from(x.b()) / 255.0, 1.0)
}

/// The text painted on the foreground (menu) layers: (text, colour).
fn menu_texts(out: &egui::FullOutput) -> Vec<(String, Color32)> {
    fn walk(s: &Shape, out: &mut Vec<(String, Color32)>) {
        match s {
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            Shape::Text(t) => out.push((t.galley.text().to_string(), t.fallback_color)),
            _ => {}
        }
    }
    let mut texts = Vec::new();
    for c in &out.shapes {
        walk(&c.shape, &mut texts);
    }
    texts
}

/// Open a dropdown with `tokens` and return its rows' text colours, with the pointer away from them.
fn rows(tokens: Tokens) -> Vec<(String, Color32)> {
    let ctx = egui::Context::default();
    theme::apply(&ctx, tokens);
    let mut t = 0.0;
    let mut frame = |events: Vec<Event>| {
        let mut out = ctx.run_ui(input(&mut t, events), |ui| {
            let (_, resp) = ui.allocate_exact_size(vec2(200.0, 24.0), egui::Sense::click());
            menu_popup(&resp).show(|ui| {
                menu_check(ui, true, "iPhone 17 Pro Max", "");
                menu_check(ui, false, "Pixel 9", "");
                menu_item(ui, "Copy", "Cmd+C");
            });
        });
        out.textures_delta.clear();
        out
    };
    let at = pos2(12.0, 12.0);
    frame(vec![]);
    frame(press(at, true));
    frame(press(at, false));
    // Past the popup's fade-in, so the colours are the ones it settles on.
    for _ in 0..30 {
        frame(vec![Event::PointerMoved(pos2(790.0, 590.0))]);
    }
    let out = frame(vec![]);
    assert!(ctx.memory(|m| m.areas().visible_layer_ids().iter().any(|l| l.order == Order::Foreground)), "the menu opened");
    menu_texts(&out)
}

#[test]
fn dropdown_rows_read_on_the_popup_in_both_themes() {
    for tokens in [Tokens::light(), Tokens::dark()] {
        // The popup's own fill (`window_fill`), what the rows are drawn on.
        let fill = color(tokens.panel);
        let texts = rows(tokens);
        let keys = crate::toolbar::pretty_shortcut("Cmd+C");
        // Labels at WCAG AA for text; the dimmer shortcut at least 3:1.
        for (label, min) in [("iPhone 17 Pro Max", 4.5), ("Pixel 9", 4.5), ("Copy", 4.5), (keys.as_str(), 3.0)] {
            let Some((_, col)) = texts.iter().find(|(s, _)| s.trim() == label || s.ends_with(label)) else {
                panic!("{label} painted (dark {}): {texts:?}", tokens.dark)
            };
            let ratio = color(*col).contrast(fill);
            assert!(ratio >= min, "{label} at {ratio:.2}:1 on the popup (dark {}): {texts:?}", tokens.dark);
        }
    }
}
