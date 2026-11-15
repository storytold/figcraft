use egui::{Pos2, vec2};
use figcraft_color::{ConcentricShape, Paint, ShaderPaint, ShaderPattern};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use super::{PickerTarget, window_id};
use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, has_text, text_center};

fn shader_picker() -> (Ui, NodeId) {
    let mut session = Session::new();
    let id = NodeId(session.execute("shape.rectangle", &json!({"width": 240, "height": 60})).unwrap()["id"].as_u64().unwrap());
    session
        .execute(
            "fill.set",
            &json!({"ids": [id], "index": 0, "paint": Paint::shader(ShaderPaint {
                pattern: ShaderPattern::Concentric,
                shape: ConcentricShape::Triangle,
                bands: 13,
                ..ShaderPaint::default()
            })}),
        )
        .unwrap();
    session.execute("select.set", &json!({"ids": [id]})).unwrap();
    let mut ui = Ui::new(session);
    ui.app.open_picker(PickerTarget::Paint { fills: true, index: 0, ids: vec![id] }, None);
    for _ in 0..5 {
        ui.frame(vec![]);
    }
    (ui, id)
}

#[test]
fn shader_is_reached_through_the_existing_paint_type_tabs() {
    let mut session = Session::new();
    let id = NodeId(session.execute("shape.rectangle", &json!({"width": 80, "height": 40})).unwrap()["id"].as_u64().unwrap());
    session.execute("select.set", &json!({"ids": [id]})).unwrap();
    let mut ui = Ui::new(session);
    ui.app.open_picker(PickerTarget::Paint { fills: true, index: 0, ids: vec![id] }, None);
    ui.screen = vec2(360.0, 300.0);
    for _ in 0..5 {
        ui.frame(vec![]);
    }

    let picker = ui.ctx.memory(|m| m.area_rect(window_id())).unwrap();
    let cell = ((picker.width() - 12.0) / 8.0).min(30.0);
    let shader_tab = picker.min + vec2(12.0 + 7.0 * cell + cell / 2.0, 60.0);
    assert!(egui::Rect::from_min_size(Pos2::ZERO, ui.screen).contains(shader_tab), "the Shader tab remains on-screen");
    ui.button(shader_tab, true);
    ui.button(shader_tab, false);
    assert!(
        matches!(&ui.app.session.doc().unwrap().node(id).unwrap().fills[0].kind, figcraft_color::PaintKind::Shader(_)),
        "the Shader tab in the existing picker creates a shader fill"
    );
    assert!(matches!(ui.app.ui.picker, Some(PickerTarget::Paint { .. })), "the picker remains open on the new paint");
}

#[test]
fn shader_pattern_and_concentric_settings_are_in_the_existing_paint_picker() {
    let (mut ui, id) = shader_picker();
    let out = ui.frame_output(vec![]);
    let text = flat(&out.shapes);
    assert!(has_text(&text, "Concentric"), "the pattern selector is in the picker");
    assert!(has_text(&text, "Triangle"), "the concentric shape selector is in the picker");
    assert!(has_text(&text, "Bands"), "the bands field is in the picker");
    assert!(
        matches!(&ui.app.session.doc().unwrap().node(id).unwrap().fills[0].kind, figcraft_color::PaintKind::Shader(p) if p.pattern == ShaderPattern::Concentric && p.bands == 13),
        "opening the picker doesn't alter the saved paint"
    );

    let picker = ui.ctx.memory(|m| m.area_rect(window_id())).unwrap();
    let pattern_button = picker.min + vec2(120.0, 124.0);
    ui.button(pattern_button, true);
    ui.button(pattern_button, false);
    let text = flat(&ui.frame_output(vec![]).shapes);
    assert!(has_text(&text, "Noise") && has_text(&text, "Waves") && has_text(&text, "Cells"), "the shader menu is in the picker");

    ui.button(pattern_button, true);
    ui.button(pattern_button, false);
    let shape_button = picker.min + vec2(120.0, 156.0);
    ui.button(shape_button, true);
    ui.button(shape_button, false);
    let text = flat(&ui.frame_output(vec![]).shapes);
    assert!(has_text(&text, "Circle") && has_text(&text, "Square") && has_text(&text, "Triangle"), "shape choices are in the picker");
}

#[test]
fn shader_picker_stays_inside_a_small_window_and_its_tab_buttons_do_not_overflow() {
    let (mut ui, _) = shader_picker();
    ui.screen = vec2(360.0, 300.0);
    for _ in 0..5 {
        ui.frame(vec![]);
    }
    let picker = ui.ctx.memory(|m| m.area_rect(window_id())).unwrap();
    let screen = egui::Rect::from_min_size(Pos2::ZERO, ui.screen);
    assert!(screen.contains_rect(picker), "picker {picker:?} stays in {screen:?}");

    let out = ui.frame_output(vec![]);
    let tab_rects: Vec<_> = flat(&out.shapes)
        .into_iter()
        .filter_map(|shape| match shape {
            egui::Shape::Rect(rect)
                if (rect.stroke.width - 1.0).abs() < 0.01
                    && (rect.rect.height() - 20.0).abs() < 0.01
                    && rect.rect.min.x >= picker.min.x
                    && rect.rect.max.x <= picker.max.x =>
            {
                Some(rect.rect)
            }
            _ => None,
        })
        .collect();
    assert!(!tab_rects.is_empty(), "the paint-type buttons are drawn inside the picker: {picker:?}");
    assert!(tab_rects.iter().all(|rect| screen.contains_rect(*rect)), "paint-type buttons stay inside the app window: {tab_rects:?} in {screen:?}");

    let shape_button = picker.min + vec2(picker.width() / 2.0, 156.0);
    ui.button(shape_button, true);
    ui.button(shape_button, false);
    let shapes = flat(&ui.frame_output(vec![]).shapes);
    for label in ["Circle", "Square", "Triangle", "Diamond", "Hexagon"] {
        let center = text_center(&shapes, label);
        assert!(screen.contains(center), "shape option {label} at {center:?} stays in {screen:?}");
    }
}
