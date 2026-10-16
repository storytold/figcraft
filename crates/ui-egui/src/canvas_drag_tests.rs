//! The canvas while a drag edits the document every frame, driven through egui headless: each
//! finished render shows (a newer one already on its way doesn't hide it), and a button held still
//! stops making new documents to render.

use std::sync::Arc;
use std::time::Duration;

use egui::{Event, vec2};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

/// Frames until the render worker has nothing in flight (a few seconds at most).
fn settle(u: &mut Ui) {
    for _ in 0..300 {
        u.frame(vec![]);
        if u.app.worker.as_ref().is_some_and(|w| !w.in_flight()) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_drag_shows_on_the_canvas_while_it_moves() {
    let mut s = Session::new();
    let id = |v: serde_json::Value| figcraft_engine::params::id_of(&v["id"]).unwrap();
    let f = id(s.execute("frame.create", &json!({"x": 100, "y": 100, "width": 300, "height": 200})).unwrap());
    s.execute("node.set", &json!({"id": f, "rotation": 25})).unwrap();
    let r =
        id(s.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 80, "height": 60, "parent": f, "props": {"fill": "#e03030"}})).unwrap());
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    let mut u = Ui::new(s);
    settle(&mut u);
    let c = u.app.session.doc().unwrap().world_bounds(r).unwrap().center();
    let p = u.app.to_screen(c.x, c.y);
    u.button(p, true);
    settle(&mut u);
    // A move every frame, each one a new document and a new render request.
    u.app.canvas.render_work = None;
    let before = u.app.session.ds().unwrap().doc.clone();
    // At least 30 moves, then on until a finished render shows (slow machines need longer).
    for k in 1..=300 {
        u.frame(vec![Event::PointerMoved(p + vec2(k as f32, k as f32 / 3.0))]);
        std::thread::sleep(Duration::from_millis(15));
        if k >= 30 && u.app.canvas.render_work.is_some() {
            break;
        }
    }
    assert!(!Arc::ptr_eq(&before, &u.app.session.ds().unwrap().doc), "the drag moved the layer");
    assert!(u.app.canvas.render_work.is_some(), "renders finished during the drag are shown");
    // Held still: the same move each frame keeps the preview, and the canvas settles.
    let held = u.app.session.ds().unwrap().doc.clone();
    // Frames until the last render lands (slow machines take a while): a drag held still makes
    // no new documents, so the worker runs dry.
    settle(&mut u);
    assert!(Arc::ptr_eq(&held, &u.app.session.ds().unwrap().doc));
    assert!(u.app.worker.as_ref().is_some_and(|w| !w.in_flight()), "nothing left to render");
    u.button(p + vec2(30.0, 10.0), false);
}

#[test]
fn releasing_a_canvas_drag_outside_finishes_the_interaction() {
    let mut s = Session::new();
    let r =
        figcraft_engine::params::id_of(&s.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 80, "height": 60})).unwrap()["id"]).unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    let mut u = Ui::new(s);
    let start = u.app.to_screen(80.0, 70.0);
    let outside = egui::pos2(u.app.canvas.rect.max.x + 20.0, start.y);
    u.button(start, true);
    u.frame(vec![Event::PointerMoved(outside)]);
    u.button(outside, false);

    assert!(u.app.session.ds().unwrap().interaction.is_none(), "the document interaction is committed");
}

fn key(u: &mut Ui, key: egui::Key) {
    let modifiers = egui::Modifiers::NONE;
    u.frame(vec![
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers },
        Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers },
    ]);
}

#[test]
fn a_drag_released_off_the_window_ends_where_the_pointer_was_last_seen_and_later_edits_stay() {
    // #278: a drag left open made a later text edit roll back the changes made since.
    let mut s = Session::new();
    let id = |v: serde_json::Value| figcraft_engine::params::id_of(&v["id"]).unwrap();
    let kept = id(s.execute("shape.rectangle", &json!({"x": 40, "y": 400, "width": 80, "height": 40})).unwrap());
    let gone = id(s.execute("shape.rectangle", &json!({"x": 160, "y": 400, "width": 80, "height": 40})).unwrap());
    let text = id(s.execute("text.create", &json!({"text": "A", "x": 40, "y": 500})).unwrap());
    s.execute("tool.rectangle", &json!({})).unwrap();
    let mut u = Ui::new(s);
    // A rectangle drawn from the middle of the canvas, released off the window, the pointer gone.
    let start = u.app.canvas.rect.center();
    u.button(start, true);
    for i in 1..=6 {
        u.frame(vec![Event::PointerMoved(start + vec2(10.0, 6.0) * i as f32)]);
    }
    let release =
        Event::PointerButton { pos: egui::pos2(-50.0, -50.0), button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() };
    u.frame(vec![release, Event::PointerGone]);
    let ds = u.app.session.ds().unwrap();
    assert!(ds.interaction.is_none(), "the drawing is committed");
    let (drawn, zoom) = (ds.selection[0], ds.view.zoom);
    let b = ds.doc.world_bounds(drawn).unwrap();
    // Not the 100 x 100 shape of a click, as an end in the middle of the canvas would make.
    assert!((b.width() * zoom - 60.0).abs() < 1.5 && (b.height() * zoom - 36.0).abs() < 1.5, "{b:?} at zoom {zoom}");
    // #278's steps: a move in the panel, a delete, then a text edit.
    u.app.run("node.set", json!({"ids": [kept], "props": {"x": 650}}));
    u.app.run("edit.delete", json!({"ids": [gone]}));
    u.app.run("select.set", json!({"ids": [text]}));
    key(&mut u, egui::Key::Enter);
    u.frame(vec![Event::Text("B".into())]);
    key(&mut u, egui::Key::Escape);
    let doc = u.app.session.doc().unwrap();
    assert_eq!(doc.world_bounds(kept).unwrap().x0, 650.0, "the move stays");
    assert!(!doc.contains(gone), "the delete stays");
    assert_eq!(doc.node(text).unwrap().kind.text().unwrap().characters, "B");
}
