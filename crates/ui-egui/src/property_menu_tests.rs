//! The Design panel's property row menus: a secondary click on a fill, stroke or effect row
//! copies that one paint or effect, and on another layer's row pastes it there.

use egui::{Event, Modifiers, PointerButton, Pos2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, text_center};

fn id(v: &serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rect(s: &mut Session) -> NodeId {
    id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap())
}

/// A click with `button` at `p`, and the frames that follow; what the last frame drew.
fn click(u: &mut Ui, p: Pos2, button: PointerButton) -> Vec<egui::Shape> {
    for _ in 0..5 {
        u.frame(vec![Event::PointerMoved(p)]);
    }
    let mut shapes = Vec::new();
    for pressed in [Some(true), Some(false)].into_iter().chain(std::iter::repeat_n(None, 20)) {
        let events = pressed.map(|pressed| Event::PointerButton { pos: p, button, pressed, modifiers: Modifiers::NONE });
        shapes = flat(&u.frame_output(events.into_iter().collect()).shapes);
    }
    shapes
}

/// The text a row menu offers, clicked.
fn pick(u: &mut Ui, menu: &[egui::Shape], label: &str) {
    click(u, text_center(menu, label), PointerButton::Primary);
}

#[test]
fn a_fill_row_copies_and_pastes_a_fill() {
    let mut s = Session::new();
    let a = rect(&mut s);
    let b = rect(&mut s);
    s.execute("fill.set", &json!({"ids": [a], "color": "#FF0000"})).unwrap();
    s.execute("fill.set", &json!({"ids": [b], "color": "#0000FF"})).unwrap();
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    let mut u = Ui::new(s);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let menu = click(&mut u, text_center(&shapes, "FF0000"), PointerButton::Secondary);
    pick(&mut u, &menu, crate::i18n::tr("Copy fill"));
    let copied = u.app.session.clipboard.paint.clone().expect("the fill is on the clipboard");
    assert_eq!(copied.1, u.app.session.doc().unwrap().node(a).unwrap().fills[0]);
    // Onto b's fill row: it replaces that fill.
    u.app.run("select.set", json!({"ids": [b]}));
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let menu = click(&mut u, text_center(&shapes, "0000FF"), PointerButton::Secondary);
    pick(&mut u, &menu, crate::i18n::tr("Paste fill"));
    assert_eq!(u.app.session.doc().unwrap().node(b).unwrap().fills[0].solid_color().unwrap().to_hex(), "FF0000");
}

#[test]
fn a_stroke_row_copies_and_pastes_a_stroke() {
    let mut s = Session::new();
    let a = rect(&mut s);
    let b = rect(&mut s);
    s.execute("stroke.add", &json!({"ids": [a], "paint": {"type": "solid", "color": "#FF0000"}})).unwrap();
    s.execute("node.set", &json!({"ids": [a], "props": {"strokeWeight": 3.0}})).unwrap();
    s.execute("stroke.add", &json!({"ids": [b]})).unwrap();
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    let mut u = Ui::new(s);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let menu = click(&mut u, text_center(&shapes, "FF0000"), PointerButton::Secondary);
    pick(&mut u, &menu, crate::i18n::tr("Copy stroke"));
    u.app.run("select.set", json!({"ids": [b]}));
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let menu = click(&mut u, text_center(&shapes, "000000"), PointerButton::Secondary);
    pick(&mut u, &menu, crate::i18n::tr("Paste stroke"));
    let strokes = u.app.session.doc().unwrap().node(b).unwrap().strokes.clone();
    assert_eq!(strokes.len(), 1, "it replaced b's stroke");
    assert_eq!(strokes[0].solid_color().unwrap().to_hex(), "FF0000");
    assert_eq!(u.app.session.doc().unwrap().node(b).unwrap().stroke.weight, 3.0, "the weight came with it");
}

#[test]
fn an_effect_row_copies_and_pastes_an_effect() {
    let mut s = Session::new();
    let a = rect(&mut s);
    let b = rect(&mut s);
    s.execute("effect.add", &json!({"ids": [a], "type": "layerBlur"})).unwrap();
    s.execute("effect.add", &json!({"ids": [b], "type": "dropShadow"})).unwrap();
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    let mut u = Ui::new(s);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let menu = click(&mut u, text_center(&shapes, "Layer blur"), PointerButton::Secondary);
    pick(&mut u, &menu, crate::i18n::tr("Copy effect"));
    u.app.run("select.set", json!({"ids": [b]}));
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let menu = click(&mut u, text_center(&shapes, "Drop shadow"), PointerButton::Secondary);
    pick(&mut u, &menu, crate::i18n::tr("Paste effect"));
    let effects = u.app.session.doc().unwrap().node(b).unwrap().effects.clone();
    assert_eq!(effects, u.app.session.doc().unwrap().node(a).unwrap().effects);
}
