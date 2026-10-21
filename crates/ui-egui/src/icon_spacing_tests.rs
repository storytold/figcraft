//! The right panel's icon rhythm: a row that opens with a 24-point icon sits it 8 in from the
//! panel's edge, an icon keeps 8 to the next control, and two icons side by side keep 4.

use egui::{Rect, pos2};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

/// Every accesskit node labelled `label` (the icon buttons carry their tips), as egui rects.
fn all_bounds(out: &egui::FullOutput, label: &str) -> Vec<Rect> {
    out.platform_output
        .accesskit_update
        .as_ref()
        .expect("accesskit update")
        .nodes
        .iter()
        .filter(|(_, n)| n.label() == Some(label))
        .filter_map(|(_, n)| n.bounds())
        .map(|b| Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32)))
        .collect()
}

/// The one pair of an eye and a remove on the same row (rows share their x, so the y tells them
/// apart), and the gap between the two icons.
fn pair_gap(out: &egui::FullOutput, eye_label: &str) -> f32 {
    let eyes = all_bounds(out, eye_label);
    let removes = all_bounds(out, "Remove");
    for e in &eyes {
        for r in &removes {
            if (e.min.y - r.min.y).abs() < 1.0 {
                return r.min.x - e.max.x;
            }
        }
    }
    panic!("no eye/remove pair on one row: {eyes:?} {removes:?}");
}

#[test]
fn a_leading_icon_sits_8_in_and_an_icon_pair_keeps_4() {
    let mut s = Session::new();
    let frame =
        figcraft_doc::NodeId(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap());
    s.execute("effect.add", &json!({"ids": [frame], "type": "dropShadow"})).unwrap();
    s.execute("select.set", &json!({"ids": [frame]})).unwrap();
    let mut u = Ui::new(s);
    for _ in 0..3 {
        u.frame(vec![]);
    }
    // Accesskit is enabled once the layout has settled: its next frame reports the whole tree.
    u.ctx.enable_accesskit();
    let out = u.frame_output(vec![]);
    let settings = all_bounds(&out, "Effect settings").remove(0);
    // The leading icon sits 8 in from the panel's edge — tighter than the fields' 16 (the panel's
    // border and the accesskit rounding land within a point of it).
    let inset = settings.min.x - (u.ctx.content_rect().max.x - u.app.ui.right_width);
    assert!((8.0..16.0).contains(&inset), "the leading icon sits 8 in, not the fields' 16: {inset}");
    assert!((pair_gap(&out, "Toggle") - 4.0).abs() < 0.5, "an icon pair keeps 4");
}

#[test]
fn a_paint_rows_visibility_and_remove_keep_4() {
    let mut s = Session::new();
    let r = figcraft_doc::NodeId(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap());
    s.execute("fill.add", &json!({"ids": [r], "color": "#FF0000"})).unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    let mut u = Ui::new(s);
    for _ in 0..3 {
        u.frame(vec![]);
    }
    u.ctx.enable_accesskit();
    let out = u.frame_output(vec![]);
    assert!((pair_gap(&out, "Toggle visibility") - 4.0).abs() < 0.5, "an icon pair keeps 4");
}

/// A row that opens with an icon (the effect and layout-grid rows, inset 8) keeps the margin the
/// Fill and Stroke rows' buttons end at: the icon moves the row's left edge only.
#[test]
fn a_row_that_opens_with_an_icon_ends_its_buttons_where_the_fill_rows_do() {
    let mut s = Session::new();
    let frame =
        figcraft_doc::NodeId(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap());
    s.execute("fill.add", &json!({"ids": [frame], "color": "#FF0000"})).unwrap();
    s.execute("effect.add", &json!({"ids": [frame], "type": "dropShadow"})).unwrap();
    s.execute("select.set", &json!({"ids": [frame]})).unwrap();
    let mut u = Ui::new(s);
    for _ in 0..3 {
        u.frame(vec![]);
    }
    u.ctx.enable_accesskit();
    let out = u.frame_output(vec![]);
    let removes = all_bounds(&out, "Remove");
    assert!(removes.len() >= 2, "a fill row's remove and an effect row's: {removes:?}");
    let right = removes[0].max.x;
    for r in &removes {
        assert!((r.max.x - right).abs() < 0.5, "every remove ends at one margin: {removes:?}");
    }
}
