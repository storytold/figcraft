//! Every 24 pt icon button paints its glyph at 16 pt, the size `icons::button` draws (#2238, after
//! the Design panel's in #2227). The panels' own buttons: the Layers panel's header, the Assets
//! panel's back button and the Dev Mode status button. The tests of the other panels sit beside
//! them (`present/tests.rs`, and the `icon_size_tests` modules of `versions`, `proto_panel`,
//! `online` and the style picker).

use egui::{Rect, pos2, vec2};
use figcraft_engine::Session;

use crate::header_icon_size_tests::{painted_rects, paints_icon_at_button_size};
use crate::icons::Icon;
use crate::{FigApp, NoServices};

fn app() -> FigApp {
    FigApp::new(Session::new(), Box::new(NoServices))
}

#[test]
fn layers_and_assets_icon_buttons_paint_16pt() {
    let mut a = app();
    let rail = painted_rects(None, |ui| crate::layers::left_panel(&mut a, ui));
    for icon in [Icon::Panels, Icon::Search, Icon::Plus] {
        assert!(paints_icon_at_button_size(&rail, icon), "Layers panel: {icon:?} is not 16 pt");
    }
    a.ui.libraries = true;
    let back = painted_rects(None, |ui| crate::assets_panel::show(&mut a, ui));
    assert!(paints_icon_at_button_size(&back, Icon::ChevronLeft), "Assets panel: the back chevron is not 16 pt");
}

#[test]
fn the_dev_mode_status_button_paints_a_16pt_glyph() {
    let mut a = app();
    let doc = a.session.doc().unwrap().clone();
    let button = Rect::from_min_size(pos2(20.0, 20.0), vec2(24.0, 24.0));
    let painted = painted_rects(None, |ui| crate::section_panel::status_icon(&mut a, ui, &doc, &[], button));
    assert!(paints_icon_at_button_size(&painted, Icon::Code), "the status button's code glyph is not 16 pt");
}
