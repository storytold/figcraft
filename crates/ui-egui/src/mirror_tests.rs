//! A right-to-left UI language mirrors the window, driven through egui headless: the icon rail
//! and the layers panel go on the right, the Design panel on the left, their edges still size them
//! (now dragged the other way) and the widths are the same preferences; egui's own chrome (scroll
//! bars, submenus) mirrors with it, and an English window is as it was.

use egui::pos2;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::{PANEL_DEFAULT, layers};

const WINDOW_W: f32 = 1440.0;

fn ui(mirrored: bool) -> Ui {
    let mut s = Session::new();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 300, "height": 200})).unwrap();
    let mut u = Ui::new(s);
    u.app.ui.mirrored = mirrored;
    u.frame(vec![]);
    u.frame(vec![]);
    u
}

/// The canvas's left and right x.
fn canvas(u: &Ui) -> (f32, f32) {
    (u.app.canvas.rect.min.x, u.app.canvas.rect.max.x)
}

/// Press on `from`, move to `to` in steps and let go.
fn drag(u: &mut Ui, from: egui::Pos2, to: egui::Pos2) {
    u.drag_to(from, to);
    u.button(to, false);
    u.frame(vec![]);
}

#[test]
fn the_design_panel_is_on_the_left_and_the_layers_on_the_right() {
    let u = ui(true);
    let (left, right) = canvas(&u);
    assert_eq!(left, PANEL_DEFAULT, "the Design panel from the window's left edge");
    assert_eq!(right, WINDOW_W - layers::RAIL_WIDTH - PANEL_DEFAULT, "the layers panel and the rail on the right");
    // egui keeps a point between a side panel and the canvas, on the canvas side.
    let design = u.app.ui.right_panel_rect.expect("the Design panel drew");
    assert_eq!((design.min.x, design.max.x), (0.0, PANEL_DEFAULT - 1.0));
    assert!(u.ctx.global_style().right_to_left, "egui's chrome mirrors too");
}

#[test]
fn an_english_window_is_as_it_was() {
    let u = ui(false);
    let (left, right) = canvas(&u);
    assert_eq!((left, right), (layers::RAIL_WIDTH + PANEL_DEFAULT, WINDOW_W - PANEL_DEFAULT));
    let design = u.app.ui.right_panel_rect.expect("the Design panel drew");
    assert_eq!((design.min.x, design.max.x), (WINDOW_W - PANEL_DEFAULT + 1.0, WINDOW_W));
    assert!(!u.ctx.global_style().right_to_left);
}

#[test]
fn mirrored_edges_size_their_panels_dragged_the_other_way() {
    let mut u = ui(true);
    let (left, right) = canvas(&u);
    // The layers panel's edge (the canvas's right), 60 to the left: the layers panel 60 wider.
    drag(&mut u, pos2(right, 400.0), pos2(right - 60.0, 400.0));
    assert_eq!((u.app.ui.left_width, u.app.ui.right_width), (PANEL_DEFAULT + 60.0, PANEL_DEFAULT));
    assert_eq!(canvas(&u), (left, right - 60.0));
    // The Design panel's edge (the canvas's left), 40 to the right: the Design panel 40 wider.
    drag(&mut u, pos2(left, 400.0), pos2(left + 40.0, 400.0));
    assert_eq!((u.app.ui.left_width, u.app.ui.right_width), (PANEL_DEFAULT + 60.0, PANEL_DEFAULT + 40.0));
    assert_eq!(canvas(&u), (left + 40.0, right - 60.0));
    // The same preferences as an English window's.
    assert_eq!(u.app.session.prefs.panel_widths, (PANEL_DEFAULT + 60.0, PANEL_DEFAULT + 40.0));
}

#[test]
fn closing_the_layers_panel_gives_the_canvas_the_right_side() {
    let mut u = ui(true);
    u.app.ui.close_left_panel();
    u.frame(vec![]);
    assert_eq!(canvas(&u), (PANEL_DEFAULT, WINDOW_W - layers::RAIL_WIDTH), "only the rail is left on the right");
}

#[test]
fn switching_back_unmirrors_the_window_and_egui() {
    let mut u = ui(true);
    u.app.ui.mirrored = false;
    u.frame(vec![]);
    u.frame(vec![]);
    assert_eq!(canvas(&u), (layers::RAIL_WIDTH + PANEL_DEFAULT, WINDOW_W - PANEL_DEFAULT));
    assert!(!u.ctx.global_style().right_to_left);
}

/// One frame of `add` in a 240-wide panel, with egui's `right_to_left` as given.
fn panel(rtl: bool, add: impl Fn(&mut egui::Ui)) {
    let ctx = egui::Context::default();
    ctx.all_styles_mut(|s| s.right_to_left = rtl);
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(240.0, 400.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| add(ui));
    out.textures_delta.clear();
}

#[test]
fn flipping_mirrors_across_the_rect_and_only_when_asked() {
    use crate::widgets::{flip_rect, flip_x};
    let within = egui::Rect::from_min_max(pos2(100.0, 0.0), pos2(340.0, 40.0));
    assert_eq!(flip_x(true, within, 116.0), 324.0, "16 in from the left becomes 16 in from the right");
    assert_eq!(flip_x(false, within, 116.0), 116.0);
    let r = egui::Rect::from_min_max(pos2(108.0, 8.0), pos2(132.0, 32.0));
    assert_eq!(flip_rect(true, within, r), egui::Rect::from_min_max(pos2(308.0, 8.0), pos2(332.0, 32.0)));
    assert_eq!(flip_rect(false, within, r), r);
}

#[test]
fn a_panel_row_starts_at_its_right_end_when_mirrored() {
    use crate::widgets::{GUTTER, PAD};
    for rtl in [false, true] {
        panel(rtl, |ui| {
            let mut placed = Vec::new();
            crate::right::row_rect(ui, 32.0, |ui| {
                placed.push(ui.allocate_exact_size(egui::vec2(50.0, 24.0), egui::Sense::hover()).0);
                placed.push(ui.allocate_exact_size(egui::vec2(50.0, 24.0), egui::Sense::hover()).0);
            });
            let w = ui.max_rect().width();
            let (first, second) = (placed[0], placed[1]);
            if rtl {
                assert_eq!(first.max.x, w - PAD, "the first control 16 in from the right");
                assert_eq!(second.max.x, first.min.x - GUTTER, "the next one to its left");
            } else {
                assert_eq!(first.min.x, PAD, "the first control 16 in from the left");
                assert_eq!(second.min.x, first.max.x + GUTTER, "the next one to its right");
            }
        });
    }
}

#[test]
fn a_rows_end_buttons_are_on_its_left_when_mirrored() {
    for rtl in [false, true] {
        panel(rtl, |ui| {
            let mut buttons = Vec::new();
            crate::right::row_rect(ui, 32.0, |ui| {
                crate::widgets::tight_end(ui, |ui| {
                    buttons.push(ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover()).0);
                    buttons.push(ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover()).0);
                });
            });
            let w = ui.max_rect().width();
            let last = buttons[1];
            if rtl {
                assert_eq!(last.min.x, crate::widgets::PAD_RIGHT, "the last button 8 in from the left");
                assert!(buttons[0].min.x > last.min.x);
            } else {
                assert_eq!(last.max.x, w - crate::widgets::PAD_RIGHT, "the last button 8 in from the right");
            }
        });
    }
}

#[test]
fn a_name_is_painted_from_its_start_edge() {
    for rtl in [false, true] {
        panel(rtl, |ui| {
            crate::theme::install_fonts(ui.ctx(), false, 0);
            let font = crate::theme::regular(11.0);
            let start = pos2(200.0, 20.0);
            let r = crate::bidi::paint_name_from(ui.painter(), start, rtl, "Home — Dark", font, egui::Color32::WHITE, 150.0);
            assert!(r.width() > 0.0);
            if rtl {
                assert_eq!(r.max.x, 200.0, "its right end on the start");
            } else {
                assert_eq!(r.min.x, 200.0, "its left end on the start");
            }
        });
    }
}
