//! Hovering the canvas, driven through egui headless (#1571): the frames egui draws after a pointer
//! move carry no new input, and a hover where nothing changed isn't hit-tested again; a move, an
//! edit, a new selection or another tool is.

use egui::{Event, Pos2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

/// A page with two rectangles side by side; the window over it.
fn two_rectangles() -> (Ui, NodeId, NodeId) {
    let mut s = Session::new();
    let mut make = |x| {
        figcraft_engine::params::id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": 100, "width": 200, "height": 200})).unwrap()["id"]).unwrap()
    };
    let (a, b) = (make(100), make(400));
    s.execute("select.none", &json!({})).unwrap();
    (Ui::new(s), a, b)
}

fn hovered(u: &Ui) -> Option<NodeId> {
    u.app.session.ds().unwrap().hover
}

/// Forget what is hovered, as if the hit test had never run.
fn forget(u: &mut Ui) {
    u.app.session.ds_mut().unwrap().hover = None;
}

#[test]
fn a_hover_where_nothing_changed_is_not_hit_tested_again() {
    let (mut u, a, b) = two_rectangles();
    u.frame(vec![]);
    let over_a: Pos2 = u.app.to_screen(200.0, 200.0);
    u.frame(vec![Event::PointerMoved(over_a)]);
    assert_eq!(hovered(&u), Some(a));
    // The frames after the move carry no new input: the hover isn't looked for again.
    forget(&mut u);
    u.frame(vec![]);
    u.frame(vec![]);
    assert_eq!(hovered(&u), None, "not hit-tested again");
    // A move is.
    u.frame(vec![Event::PointerMoved(u.app.to_screen(500.0, 200.0))]);
    assert_eq!(hovered(&u), Some(b));
    // So is the same place after an edit (the layer under the pointer moved away)…
    u.app.session.execute("node.set", &json!({"ids": [b.0], "props": {"x": 900}})).unwrap();
    u.frame(vec![]);
    assert_eq!(hovered(&u), None, "the edit is seen");
    u.app.session.execute("node.set", &json!({"ids": [b.0], "props": {"x": 400}})).unwrap();
    u.frame(vec![]);
    assert_eq!(hovered(&u), Some(b));
    // …a new selection and another tool.
    forget(&mut u);
    u.app.session.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    u.frame(vec![]);
    assert_eq!(hovered(&u), Some(b), "the selection is seen");
    forget(&mut u);
    u.app.session.execute("tool.set", &json!({"tool": "scale"})).unwrap();
    u.frame(vec![]);
    assert_eq!(hovered(&u), Some(b), "the tool is seen");
    // Leaving the canvas and coming back to the same place hovers again.
    let off = u.app.canvas.rect.left_top() - egui::vec2(10.0, 10.0);
    u.frame(vec![Event::PointerMoved(off)]);
    forget(&mut u);
    u.frame(vec![Event::PointerMoved(u.app.to_screen(500.0, 200.0))]);
    assert_eq!(hovered(&u), Some(b));
}
