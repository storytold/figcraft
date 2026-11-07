//! The stroke's position / weight row in the Design panel, driven headless at the panel's default
//! width: the weight field keeps room for its value beside the variable button, whose click opens
//! the variable picker, and the row's buttons stay inside the panel (#1452: the weight field was
//! so narrow that the variable button sat on its icon and hid the value).

use egui::accesskit::Role;
use egui::{Id, Pos2, Rect};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

/// A stroked rectangle (`rect`) or ellipse, selected.
fn scene(kind: &str) -> Ui {
    let mut s = Session::new();
    let made = s.execute(&format!("shape.{kind}"), &json!({"x": 0, "y": 0, "width": 200, "height": 120})).unwrap();
    let id = made["id"].as_u64().unwrap();
    s.execute("stroke.add", &json!({"ids": [id]})).unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    let mut u = Ui::new(s);
    u.frame(vec![]);
    u
}

/// The area of part `part` ("lead", "field" or "bind") of the stroke weight field.
fn weight(u: &Ui, part: &str) -> Rect {
    u.ctx.read_response(Id::unique("stroke_w").with(part)).unwrap_or_else(|| panic!("the weight field's {part} shows")).rect
}

/// The bounds of the button named `label`, found in the AccessKit tree.
fn named_button(u: &mut Ui, label: &str) -> Rect {
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let out = u.frame_output(vec![]);
    let nodes = out.platform_output.accesskit_update.expect("an AccessKit update").nodes;
    let n = nodes.iter().map(|(_, n)| n).find(|n| n.role() == Role::Button && n.label() == Some(label)).expect(label);
    let b = n.bounds().expect("bounds");
    Rect::from_min_max(Pos2::new(b.x0 as f32, b.y0 as f32), Pos2::new(b.x1 as f32, b.y1 as f32))
}

/// The variable button clears the weight's icon and leaves the value room for a few digits, for
/// layers with per-side strokes (one more button in the row) and without.
#[test]
fn weight_field_keeps_its_value_beside_the_variable_button() {
    for kind in ["rectangle", "ellipse"] {
        let u = scene(kind);
        let (lead, field, bind) = (weight(&u, "lead"), weight(&u, "field"), weight(&u, "bind"));
        assert!(bind.min.x >= lead.max.x + 4.0, "{kind}: the variable button {bind:?} sits on the icon {lead:?}");
        assert!(bind.min.x - field.min.x >= 16.0, "{kind}: no room for the value left of the variable button in {field:?} ({bind:?})");
    }
}

/// The row's last button (Advanced) stays inside the panel's 8 pt right margin (#858's grid), in
/// line with the section headers' buttons, also with the per-side button beside the weight.
#[test]
fn stroke_row_buttons_stay_inside_the_panel() {
    for kind in ["rectangle", "ellipse"] {
        let mut u = scene(kind);
        let advanced = named_button(&mut u, "Advanced stroke");
        let panel_right = u.ctx.content_rect().max.x - crate::widgets::PAD_RIGHT;
        assert!(advanced.max.x <= panel_right + 0.5, "{kind}: Advanced at {advanced:?} runs past the panel's edge {panel_right}");
        if kind == "rectangle" {
            let sides = named_button(&mut u, "Individual strokes");
            assert!(sides.min.x >= weight(&u, "field").max.x, "{kind}: the per-side button {sides:?} overlaps the weight");
            assert!(sides.max.x <= advanced.min.x, "{kind}: the per-side button {sides:?} overlaps Advanced {advanced:?}");
        }
    }
}

/// A click on the variable button opens the variable picker for the weight.
#[test]
fn variable_button_opens_the_picker() {
    let mut u = scene("rectangle");
    let field = weight(&u, "field");
    let at = weight(&u, "bind").center();
    // The button shows while the pointer is over the field.
    u.frame(vec![egui::Event::PointerMoved(field.center())]);
    u.button(at, true);
    u.button(at, false);
    u.frame(vec![]);
    assert!(crate::variable_ui::picker_open(&u.ctx), "the variable picker opened");
}
