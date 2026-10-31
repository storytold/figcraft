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
