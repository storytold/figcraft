//! Hovering a blend mode in the layer's or a paint's menu shows it on the canvas as a live
//! preview: no edit, no undo step, and the layer is back as it was when the pointer leaves.

use egui::{Event, Pos2, RawInput, Rect, pos2, vec2};
use figcraft_color::BlendMode;
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

fn id(v: serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn scene() -> (Ui, NodeId) {
    let mut s = Session::new();
    let a = id(s.execute("node.create", &json!({"type": "rectangle", "x": 0, "y": 0, "width": 40, "height": 20})).unwrap());
    s.execute("fill.set", &json!({"ids": [a], "index": 0, "paint": {"type": "solid", "color": "#ff0000"}})).unwrap();
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    (Ui::new(s), a)
}

#[test]
fn every_row_of_the_blend_menu_previews_its_own_mode_under_the_pointer() {
    let ctx = egui::Context::default();
    let mut seen: Vec<BlendMode> = Vec::new();
    let mut time = 0.0;
    // Down the menu, a row at a time: each row under the pointer reports its mode.
    for y in (0..1000).step_by(2) {
        time += 1.0 / 60.0;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 1000.0))),
            time: Some(time),
            events: vec![Event::PointerMoved(pos2(40.0, y as f32))],
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            let _ = crate::right::blend_menu(ui, true, |b| seen.push(b));
        });
        out.textures_delta.clear();
    }
    seen.dedup();
    assert_eq!(seen, BlendMode::ALL.to_vec(), "rows preview their modes top to bottom");
}

#[test]
fn a_hovered_layer_blend_shows_on_the_canvas_and_leaves_no_trace() {
    let (mut u, a) = scene();
    let ctx = u.ctx.clone();
    assert_eq!(u.app.session.doc().unwrap().node(a).unwrap().blend, BlendMode::Normal);
    let steps = u.app.session.undo_labels(100).len();
    crate::right::preview_blend(&mut u.app, &ctx, &[a], BlendMode::Multiply);
    assert_eq!(u.app.session.doc().unwrap().node(a).unwrap().blend, BlendMode::Multiply, "the canvas shows the hovered mode");
    assert!(figcraft_engine::textpreview::active(&u.app.session), "as a preview");
    // Nothing asks for it on the next frame: the layer is back as it was.
    u.frame(vec![]);
    u.frame(vec![]);
    assert_eq!(u.app.session.doc().unwrap().node(a).unwrap().blend, BlendMode::Normal, "the preview ended");
    assert!(!figcraft_engine::textpreview::active(&u.app.session));
    assert_eq!(u.app.session.undo_labels(100).len(), steps, "a preview is no undo step");
}

#[test]
fn picking_after_a_preview_is_one_step_back_to_the_original() {
    let (mut u, a) = scene();
    let steps = u.app.session.undo_labels(100).len();
    crate::right::preview_blend(&mut u.app, &u.ctx.clone(), &[a], BlendMode::Multiply);
    // The click lands while the hovered mode shows: the pick applies to the document, not the preview.
    u.app.set_props(&[a], json!({"blend": BlendMode::Screen.label()}));
    assert_eq!(u.app.session.doc().unwrap().node(a).unwrap().blend, BlendMode::Screen);
    assert!(!figcraft_engine::textpreview::active(&u.app.session), "the pick ended the preview");
    assert_eq!(u.app.session.undo_labels(100).len(), steps + 1, "one undo step");
    u.app.session.undo().unwrap();
    assert_eq!(u.app.session.doc().unwrap().node(a).unwrap().blend, BlendMode::Normal, "undo goes back to the original, not the hovered mode");
}

#[test]
fn a_hovered_paint_blend_shows_on_its_layer_and_leaves_no_trace() {
    let (mut u, a) = scene();
    let target = crate::colorpicker::PickerTarget::Paint { fills: true, index: 0, ids: vec![a] };
    let paint = u.app.session.doc().unwrap().node(a).unwrap().fills[0].clone();
    let steps = u.app.session.undo_labels(100).len();
    crate::colorpicker::preview_blend(&mut u.app, &u.ctx.clone(), &target, &paint, BlendMode::Screen);
    assert_eq!(u.app.session.doc().unwrap().node(a).unwrap().fills[0].blend, BlendMode::Screen, "the fill shows the hovered mode");
    u.frame(vec![]);
    u.frame(vec![]);
    assert_eq!(u.app.session.doc().unwrap().node(a).unwrap().fills[0].blend, BlendMode::Normal, "the preview ended");
    assert_eq!(u.app.session.undo_labels(100).len(), steps, "a preview is no undo step");
}
