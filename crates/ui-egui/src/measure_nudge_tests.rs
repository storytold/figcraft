//! Nudging while measuring (#894), driven through egui headless: with Alt held over another layer
//! the arrows still nudge the selection (Shift: the big step), the distance follows every nudge,
//! and the selection's handles and size label hide while measuring (its outline stays).

use egui::{Event, Key, Modifiers, Pos2, Shape};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::live_tests::flat;
use crate::present_tests::id;

fn labels(u: &Ui) -> Vec<String> {
    u.app.session.tool.overlay.guides.iter().filter_map(|g| g.label.clone()).collect()
}

fn top(u: &Ui, i: NodeId) -> f64 {
    u.app.session.doc().unwrap().world_bounds(i).unwrap().y0
}

/// Press and release `key` with `m` held, then a settled frame (the pointer still over the canvas).
fn press(u: &mut Ui, key: Key, m: Modifiers, at: Pos2) {
    let ev = |pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: m };
    u.frame_with(vec![ev(true), ev(false)], m);
    u.frame_with(vec![Event::PointerMoved(at)], m);
}

/// Whether the frame drew corner handles or a text `label`.
fn drew(shapes: &[Shape], label: &str) -> (bool, bool) {
    let handle = shapes.iter().any(|s| matches!(s, Shape::Rect(r) if (r.rect.width() - 8.0).abs() < 0.1 && (r.rect.height() - 8.0).abs() < 0.1));
    let text = shapes.iter().any(|s| matches!(s, Shape::Text(t) if t.galley.text() == label));
    (handle, text)
}

#[test]
fn arrows_nudge_while_measuring_and_the_distance_follows() {
    let mut s = Session::new();
    let upper = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 40})).unwrap());
    let lower = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 140, "width": 100, "height": 40})).unwrap());
    s.execute("select.set", &json!({"ids": [lower]})).unwrap();
    let mut u = Ui::new(s);
    let over = u.app.to_screen(50.0, 20.0);
    let (alt, shift_alt) = (Modifiers { alt: true, ..Modifiers::NONE }, Modifiers { alt: true, shift: true, ..Modifiers::NONE });

    // Not measuring: the selection shows its handles and size.
    let shapes = flat(&u.frame_with(vec![Event::PointerMoved(over)], Modifiers::NONE).shapes);
    assert_eq!(drew(&shapes, "100 × 40"), (true, true), "handles and size label show");

    u.frame_with(vec![Event::PointerMoved(over)], alt);
    let shapes = flat(&u.frame_with(vec![Event::PointerMoved(over)], alt).shapes);
    assert!(u.app.session.tool.overlay.measure_box.is_some(), "Alt over the other layer measures to it");
    assert!(labels(&u).contains(&"100".to_string()), "the gap: {:?}", labels(&u));
    assert_eq!(drew(&shapes, "100 × 40"), (false, false), "measuring hides the handles and size label");

    press(&mut u, Key::ArrowUp, alt, over);
    assert_eq!(top(&u, lower), 139.0, "Alt+Up nudges by the small step");
    assert!(labels(&u).contains(&"99".to_string()), "the gap follows the nudge: {:?}", labels(&u));

    press(&mut u, Key::ArrowUp, shift_alt, over);
    assert_eq!(top(&u, lower), 129.0, "Shift+Alt+Up nudges by the big step");
    assert!(labels(&u).contains(&"89".to_string()), "the gap follows the big nudge: {:?}", labels(&u));
    assert_eq!(top(&u, upper), 0.0, "the measured layer stays put");
}
