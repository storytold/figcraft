//! Component rows in the Layers panel (#1903), drawn headless in both themes: a component's name
//! reads at WCAG AA (4.5:1) on the panel and on its selected row, and the rows inside a selected
//! component take a tone of its row, not the plain selection's green.

use egui::{Color32, Pos2, Shape};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::theme::Tokens;

fn id(v: serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn color(x: Color32) -> figcraft_color::Color {
    figcraft_color::Color::new(f32::from(x.r()) / 255.0, f32::from(x.g()) / 255.0, f32::from(x.b()) / 255.0, 1.0)
}

/// The Layers panel with the component "Button" (holding the rectangle "Label") selected and
/// expanded, in the `dark` or light theme: the frame's shapes and the theme's tokens.
fn drawn(dark: bool) -> (Vec<Shape>, Tokens) {
    let mut s = Session::new();
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 40, "props": {"name": "Button"}})).unwrap());
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 20, "height": 20, "parent": f, "props": {"name": "Label"}})).unwrap();
    let c = id(s.execute("component.create", &json!({"ids": [f]})).unwrap());
    s.execute(if dark { "view.themeDark" } else { "view.themeLight" }, &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [c]})).unwrap();
    let mut u = Ui::new(s);
    u.app.ui.expanded.insert(c);
    u.frame(vec![]);
    let shapes = crate::live_tests::flat(&u.frame_output(vec![]).shapes);
    (shapes, Tokens::get(&u.ctx))
}

/// The colour the name `s` is painted in.
fn name_colour(shapes: &[Shape], s: &str) -> Color32 {
    shapes
        .iter()
        .find_map(|sh| match sh {
            Shape::Text(t) if t.galley.text() == s => Some(t.fallback_color),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no name {s:?} painted"))
}

/// Where the name `s` is painted (its top left).
fn name_pos(shapes: &[Shape], s: &str) -> Pos2 {
    shapes
        .iter()
        .find_map(|sh| match sh {
            Shape::Text(t) if t.galley.text() == s => Some(t.pos),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no name {s:?} painted"))
}

/// Whether a rectangle filled in `c` is painted under `p`.
fn rect_under(shapes: &[Shape], c: Color32, p: Pos2) -> bool {
    shapes.iter().any(|s| matches!(s, Shape::Rect(r) if r.fill == c && r.rect.contains(p)))
}

#[test]
fn component_names_read_at_aa_on_the_panel_and_on_their_selected_row() {
    for dark in [false, true] {
        let (shapes, t) = drawn(dark);
        assert_eq!(t.dark, dark);
        let col = name_colour(&shapes, "Button");
        assert_eq!(col, t.component, "the theme's component tint (dark {dark})");
        for (what, bg) in [("the panel", t.panel), ("the selected component row", t.component_row), ("a hovered row", t.hover)] {
            let ratio = color(col).contrast(color(bg));
            assert!(ratio >= 4.5, "{ratio:.2}:1 on {what} (dark {dark})");
        }
        // The layer inside it is a plain layer: the theme's text colour.
        assert_eq!(name_colour(&shapes, "Label"), t.text);
    }
}

#[test]
fn rows_inside_a_selected_component_take_a_tone_of_its_row() {
    for dark in [false, true] {
        let (shapes, t) = drawn(dark);
        let button = name_pos(&shapes, "Button");
        let label = name_pos(&shapes, "Label");
        // Checked under each name, by row: the pages list paints the current page in the plain
        // selection's `selected_row` too.
        assert!(rect_under(&shapes, t.component_row, button), "the component's own row (dark {dark})");
        assert!(rect_under(&shapes, t.component_row_child, label), "the row inside it (dark {dark})");
        assert!(!rect_under(&shapes, t.selected_row, button), "not the plain selection's row (dark {dark})");
        assert!(!rect_under(&shapes, t.selected_row_child, label), "not the plain selection's child tone (dark {dark})");
    }
}
