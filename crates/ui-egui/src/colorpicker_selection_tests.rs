//! The colour picker of a layer's paint or effect closes when that layer is no longer the
//! selection, so it never keeps editing a layer that was deselected (#396).

use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::colorpicker::PickerTarget;
use crate::layers_drag_tests::Ui;

fn rect(s: &mut Session, x: f64) -> NodeId {
    let v = s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 20, "height": 20})).unwrap();
    NodeId(v["id"].as_u64().unwrap())
}

fn two() -> (Ui, NodeId, NodeId) {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0);
    let b = rect(&mut s, 40.0);
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    (Ui::new(s), a, b)
}

#[test]
fn picker_closes_when_its_layer_is_deselected() {
    for effect in [false, true] {
        let (mut u, a, _) = two();
        let target = if effect {
            u.app.run("effect.add", json!({"ids": [a], "type": "dropShadow"}));
            PickerTarget::Effect { ids: vec![a], index: 0, secondary: false }
        } else {
            PickerTarget::Paint { fills: true, index: 0, ids: vec![a] }
        };
        u.app.ui.picker = Some(target.clone());
        u.frame(vec![]);
        u.frame(vec![]);
        assert_eq!(u.app.ui.picker, Some(target.clone()), "the picker opens while its layer is selected");
        u.app.run("select.none", json!({}));
        u.frame(vec![]);
        u.frame(vec![]);
        assert!(u.app.ui.picker.is_none(), "deselecting closes {target:?}");
    }
}

#[test]
fn picker_closes_when_another_layer_is_selected() {
    let (mut u, a, b) = two();
    u.app.ui.picker = Some(PickerTarget::Paint { fills: true, index: 0, ids: vec![a] });
    u.frame(vec![]);
    u.app.run("select.set", json!({"ids": [b]}));
    u.frame(vec![]);
    u.frame(vec![]);
    assert!(u.app.ui.picker.is_none(), "selecting another layer closes the picker: {:?}", u.app.ui.picker);
}

#[test]
fn clicking_the_swatch_again_closes_the_picker() {
    let (mut u, a, _) = two();
    u.app.run("fill.add", json!({"ids": [a], "color": "#00FF00"}));
    let target = PickerTarget::Paint { fills: true, index: 0, ids: vec![a] };
    u.app.toggle_picker(target.clone(), Some(egui::Rect::from_min_size(egui::pos2(1250.0, 200.0), egui::vec2(16.0, 16.0))));
    u.frame(vec![]);
    u.frame(vec![]);
    assert_eq!(u.app.ui.picker, Some(target.clone()), "the swatch opens the picker");
    u.app.toggle_picker(target.clone(), Some(egui::Rect::from_min_size(egui::pos2(1250.0, 200.0), egui::vec2(16.0, 16.0))));
    u.frame(vec![]);
    assert!(u.app.ui.picker.is_none(), "the same swatch toggles it shut");
    // Another paint of the same layer opens the picker for that one.
    let second = PickerTarget::Paint { fills: true, index: 1, ids: vec![a] };
    u.app.toggle_picker(second.clone(), Some(egui::Rect::from_min_size(egui::pos2(1250.0, 200.0), egui::vec2(16.0, 16.0))));
    u.frame(vec![]);
    assert_eq!(u.app.ui.picker, Some(second), "another paint opens the picker for it");
}

// Regression test for #237: the "On this page" swatches wrapped onto rows that started at the picker's
// left edge (only the first row was indented) and ran past its right edge.
#[test]
fn wrapped_page_colour_swatches_stay_inset_inside_the_picker() {
    use egui::Shape;
    let mut s = Session::new();
    let mut first = None;
    for i in 0..40 {
        let v = s
            .execute(
                "shape.rectangle",
                &json!({"x": i * 30, "y": 0, "width": 20, "height": 20, "props": {"fill": format!("#{:02X}{:02X}80", i * 6, 255 - i * 6)}}),
            )
            .unwrap();
        first.get_or_insert(NodeId(v["id"].as_u64().unwrap()));
    }
    let a = first.unwrap();
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    let mut u = Ui::new(s);
    u.app.ui.picker = Some(PickerTarget::Paint { fills: true, index: 0, ids: vec![a] });
    for _ in 0..3 {
        u.frame(vec![]);
    }
    let out = u.frame_output(vec![]);
    // The picker is the only floating window open: its area is the picker's own id, which the style
    // picker also looks up to tell a press in here from one outside.
    let picker = u.ctx.memory(|m| m.area_rect(crate::colorpicker::window_id())).expect("the picker is open");
    // A swatch is a 14 pt square with a 1 pt outline.
    let swatches: Vec<egui::Rect> = out
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            Shape::Rect(r) if r.stroke.width == 1.0 && (r.rect.width() - 14.0).abs() < 0.01 && (r.rect.height() - 14.0).abs() < 0.01 => Some(r.rect),
            _ => None,
        })
        .filter(|r| picker.contains_rect(*r))
        .collect();
    let mut rows: Vec<f32> = swatches.iter().map(|r| r.min.y).collect();
    rows.sort_by(f32::total_cmp);
    rows.dedup_by(|a, b| (*a - *b).abs() < 1.0);
    assert!(rows.len() >= 2, "expected the page colours to wrap onto several rows, got {} swatches in {} row(s)", swatches.len(), rows.len());
    for r in &swatches {
        assert!(
            r.min.x >= picker.min.x + 16.0 - 0.5 && r.max.x <= picker.max.x - 16.0 + 0.5,
            "swatch {r:?} is not inset 16 pt from both sides of the picker {picker:?}"
        );
    }
}

#[test]
fn opening_the_first_picker_preserves_the_active_text_undo_interaction() {
    let mut s = Session::new();
    let text = NodeId(s.execute("text.create", &json!({"text": "Hi"})).unwrap()["id"].as_u64().unwrap());
    let mut app = crate::FigApp::new(s, Box::new(crate::NoServices));
    figcraft_engine::textedit::begin(&mut app.session, text, Some(2), false).unwrap();
    figcraft_engine::textedit::insert(&mut app.session, "a").unwrap();
    let depth = app.session.undo_depth();
    app.open_picker(PickerTarget::TextRange { id: text, start: 0, end: 2 }, None);
    assert!(app.session.ds().unwrap().interaction.is_some());
    assert_eq!(app.session.undo_depth(), depth);
    figcraft_engine::textedit::insert(&mut app.session, "b").unwrap();
    figcraft_engine::textedit::end(&mut app.session).unwrap();
    app.session.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().node(text).unwrap().kind.text().unwrap().characters, "Hi");
}

#[test]
fn the_picker_uses_the_fitted_sidebar_edge() {
    use egui::{Pos2, RawInput, Rect, vec2};
    let (mut u, a, _) = two();
    u.app.ui.right_width = 480.0;
    u.app.open_picker(PickerTarget::Paint { fills: true, index: 0, ids: vec![a] }, None);
    for _ in 0..6 {
        u.frame_raw(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 900.0))), ..Default::default() });
    }
    let sidebar = u.app.ui.right_panel_rect.unwrap();
    assert!(sidebar.width() < u.app.ui.right_width, "the requested width does not fit");
    let pop = u.app.ui.picker_popover.as_ref().unwrap();
    assert!((pop.rect.max.x - (sidebar.min.x - 8.0)).abs() < 1.0, "{:?} beside {sidebar:?}", pop.rect);
}

#[test]
fn moved_page_effect_and_text_range_swatches_still_toggle_the_picker_closed() {
    use egui::{Id, Pos2};
    for kind in ["page", "effect", "text"] {
        let (mut u, a, _) = two();
        let target = match kind {
            "page" => {
                u.app.session.execute("select.none", &json!({})).unwrap();
                PickerTarget::PageBackground
            }
            "effect" => {
                u.app.session.execute("effect.add", &json!({"id":a,"type":"dropShadow"})).unwrap();
                u.ctx.data_mut(|d| d.insert_temp(Id::unique(("fx_open", 0usize)), true));
                PickerTarget::Effect { ids: vec![a], index: 0, secondary: false }
            }
            _ => {
                let text = NodeId(u.app.session.execute("text.create", &json!({"text":"Hello"})).unwrap()["id"].as_u64().unwrap());
                figcraft_engine::textedit::begin(&mut u.app.session, text, None, false).unwrap();
                PickerTarget::TextRange { id: text, start: 0, end: 5 }
            }
        };
        u.app.open_picker(target, None);
        for _ in 0..5 {
            u.frame(vec![]);
        }
        let before = u.app.ui.picker_button.expect(kind);
        u.app.ui.right_width = 360.0;
        for _ in 0..5 {
            u.frame(vec![]);
        }
        let after = u.app.ui.picker_button.expect(kind);
        assert!((after.min.x - before.min.x).abs() > 50.0, "{kind}: swatch moves");
        u.button(after.center(), true);
        u.button(after.center(), false);
        u.frame(vec![egui::Event::PointerMoved(Pos2::new(1430.0, 890.0))]);
        assert!(u.app.ui.picker.is_none(), "{kind}: the moved swatch closes it");
    }
}

#[test]
fn a_press_in_the_gradient_variable_picker_keeps_the_colour_picker_open() {
    use egui::{Pos2, RawInput, Rect, pos2, vec2};
    let (mut u, a, _) = two();
    u.app.session.execute("fill.set", &json!({"id":a,"paint":{"type":"linearGradient","stops":["#ff0000","#0000ff"]}})).unwrap();
    u.app.open_picker(PickerTarget::Paint { fills: true, index: 0, ids: vec![a] }, None);
    for _ in 0..5 {
        u.frame(vec![]);
    }
    // Open the same child picker used by a gradient stop, away from the parent's card.
    let _ = u.ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))), ..Default::default() }, |ui| {
        crate::variable_ui::open(ui, &[a], "fills/0/stops/0/color", figcraft_doc::VariableType::Color, pos2(500.0, 180.0));
    });
    for _ in 0..5 {
        u.frame(vec![]);
    }
    let child = crate::variable_ui::picker_rect(&u.ctx).unwrap();
    let at = child.min + vec2(30.0, 20.0);
    assert!(!u.app.ui.picker_popover.as_ref().unwrap().rect.contains(at));
    u.button(at, true);
    assert!(u.app.ui.picker.is_some(), "the child variable picker is part of the interaction");
    u.button(at, false);
}
