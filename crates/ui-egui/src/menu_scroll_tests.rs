//! A panel menu taller than the window scrolls, so its last rows can be reached and picked: the
//! page's variable mode menu of a collection with more modes than fit.

use egui::{Event, Modifiers, MouseWheelUnit, PointerButton, Pos2, TouchPhase, vec2};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, text_center, texts};

/// A click at `p`, and the frames that follow; what the last frame drew.
fn click(u: &mut Ui, p: Pos2) -> Vec<egui::Shape> {
    for _ in 0..5 {
        u.frame(vec![Event::PointerMoved(p)]);
    }
    let mut shapes = Vec::new();
    for pressed in [Some(true), Some(false)].into_iter().chain(std::iter::repeat_n(None, 20)) {
        let events = pressed.map(|pressed| Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        shapes = flat(&u.frame_output(events.into_iter().collect()).shapes);
    }
    shapes
}

/// Where `label` is drawn, if it is.
fn at(shapes: &[egui::Shape], label: &str) -> Option<Pos2> {
    texts(shapes).into_iter().find(|(t, ..)| t == label).map(|(_, p, size)| p + size / 2.0)
}

#[test]
fn a_menu_taller_than_the_window_scrolls_to_its_last_row() {
    let modes: Vec<String> = (0..60).map(|k| format!("Mode {k}")).collect();
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme", "modes": modes})).unwrap();
    s.execute("select.set", &json!({"ids": []})).unwrap();
    let mut u = Ui::new(s);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let menu = click(&mut u, text_center(&shapes, "Auto (Mode 0)"));
    let first = at(&menu, "    Mode 0").expect("the menu is open");
    assert!(at(&menu, "    Mode 59").is_none_or(|p| p.y > 900.0), "the last row starts below the window");
    let wheel = Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        source: egui::MouseWheelSource::Unknown,
        delta: vec2(0.0, -5000.0),
        phase: TouchPhase::Move,
        modifiers: Modifiers::NONE,
    };
    u.frame(vec![Event::PointerMoved(first), wheel]);
    for _ in 0..30 {
        u.frame(vec![]);
    }
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let last = at(&shapes, "    Mode 59").expect("scrolled to the last row");
    assert!(last.y < 900.0, "inside the window: {last:?}");
    click(&mut u, last);
    let page = u.app.session.page().unwrap();
    let doc = u.app.session.doc().unwrap();
    let coll = &doc.variables.collections[0];
    let picked = coll.modes.iter().find(|m| m.name == "Mode 59").unwrap().id.clone();
    assert_eq!(doc.node(page).unwrap().var_modes.get(&coll.id), Some(&picked), "the last mode is picked");
}
