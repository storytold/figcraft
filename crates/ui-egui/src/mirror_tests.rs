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
    ui_from(s, mirrored)
}

fn ui_from(s: Session, mirrored: bool) -> Ui {
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

#[test]
fn a_minimized_window_has_its_expand_button_where_the_rail_was() {
    use egui::accesskit::Role;
    for mirrored in [true, false] {
        let mut u = ui(mirrored);
        u.app.run("select.set", json!({"ids": []}));
        u.app.run("view.minimize", json!({"on": true}));
        u.frame(vec![]);
        let expand = crate::panel_width_tests::control(&mut u, Role::Button, "Expand the interface").expect("the Expand button");
        // 32 wide, 8 from the window's edge.
        let x = if mirrored { WINDOW_W - 24.0 } else { 24.0 };
        assert_eq!(expand.x, x, "mirrored: {mirrored}");
    }
}

/// Mirrored, a layer's icon sits at its row's right end (8 + 4 + 22 points in from it), and a
/// double-click there zooms to the layer instead of renaming it, as on the left in English.
#[test]
fn a_double_click_on_a_mirrored_layers_icon_zooms_to_it() {
    use egui::{Event, Pos2};
    let mut s = Session::new();
    let far = crate::layers_wide_names_tests::named(&mut s, "far", None);
    s.execute("node.set", &json!({"ids": [far], "props": {"x": 3000}})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let mut u = ui_from(s, true);
    let over = |u: &mut Ui, p: Pos2| {
        u.app.session.ds_mut().unwrap().hover = None;
        u.frame(vec![Event::PointerMoved(p)]);
        u.app.session.ds().unwrap().hover == Some(far)
    };
    let panel_end = WINDOW_W - layers::RAIL_WIDTH;
    let ys: Vec<f32> = (60..900).step_by(2).map(|y| y as f32).filter(|&y| over(&mut u, pos2(panel_end - 40.0, y))).collect();
    let y = (ys.first().expect("the row") + ys.last().expect("the row")) / 2.0;
    let right = (0..120).map(|d| panel_end - d as f32).find(|&x| over(&mut u, pos2(x, y))).expect("the row's right edge");
    let icon = pos2(right - 34.0, y);
    let view = |u: &Ui| {
        let v = &u.app.session.ds().unwrap().view;
        (v.cx, v.cy, v.zoom)
    };
    let before = view(&u);
    for _ in 0..2 {
        u.button(icon, true);
        u.button(icon, false);
    }
    u.frame(vec![]);
    assert_eq!(u.app.ui.rename, None, "a double-click on the icon renames nothing");
    assert_eq!(u.app.session.selection(), vec![far], "the layer is selected");
    assert_ne!(view(&u), before, "the view moved to the layer");
}

/// A list as wide as a long name (#1904) mirrors across its whole width, so mirrored it starts
/// scrolled to its right end, where the rows start: the names start at the panel's start edge,
/// not past it. English, Arabic, then English again, as when the UI language is switched: each
/// direction keeps its own scroll position.
#[test]
fn a_wide_layers_list_starts_where_its_rows_start() {
    use crate::layers_wide_names_tests::{named, painted};
    let mut s = Session::new();
    named(&mut s, "Size=small, Type=primary, State=default, Icon=leading, Width=full", None);
    named(&mut s, "Short", None);
    let mut u = Ui::new(s);
    for mirrored in [false, true, false] {
        u.app.ui.mirrored = mirrored;
        u.frame(vec![]);
        u.frame(vec![]);
        let panel = if mirrored {
            (WINDOW_W - layers::RAIL_WIDTH - PANEL_DEFAULT)..=(WINDOW_W - layers::RAIL_WIDTH)
        } else {
            layers::RAIL_WIDTH..=(layers::RAIL_WIDTH + PANEL_DEFAULT)
        };
        for name in ["Short", "Size=small"] {
            let (_, p, size) = painted(&mut u, name);
            let start = if mirrored { p.x + size.x } else { p.x };
            assert!(panel.contains(&start), "mirrored: {mirrored}, {name} starts at {start}, outside the panel {panel:?}");
        }
    }
}

/// The x of every filled shape painted in the component colour inside `panel`'s header strip.
fn component_icon_xs(u: &mut Ui, panel: egui::Rect) -> Vec<f32> {
    fn walk(shape: &egui::Shape, out: &mut Vec<egui::Pos2>) {
        match shape {
            egui::Shape::Path(p) if p.fill == crate::theme::COMPONENT => out.push(p.visual_bounding_rect().center()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let frame = u.frame_output(vec![]);
    let mut at = Vec::new();
    frame.shapes.iter().for_each(|c| walk(&c.shape, &mut at));
    at.into_iter().filter(|p| panel.x_range().contains(p.x) && p.y < 190.0).map(|p| p.x).collect()
}

#[test]
fn a_mirrored_header_paints_the_component_icon_right_of_its_name() {
    for mirrored in [false, true] {
        let mut s = Session::new();
        let f = s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 80, "height": 40, "name": "Knapp"})).unwrap();
        let c = s.execute("component.create", &json!({"ids": [f["id"]]})).unwrap();
        s.execute("select.set", &json!({"ids": [c["id"]]})).unwrap();
        let mut u = ui_from(s, mirrored);
        let panel = u.app.ui.right_panel_rect.expect("the Design panel drew");
        let name = crate::export_preview_tests::labelled(&mut u)
            .into_iter()
            .find(|(t, at)| t == "Knapp" && panel.x_range().contains(at.x) && at.y < 190.0)
            .map(|(_, at)| at.x)
            .expect("the header names the component");
        let icon = component_icon_xs(&mut u, panel);
        assert!(!icon.is_empty(), "mirrored: {mirrored}, no component icon in the header");
        for x in icon {
            assert_eq!(x > name, mirrored, "mirrored: {mirrored}, icon at {x}, name at {name}");
        }
    }
}
