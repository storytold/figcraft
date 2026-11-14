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

/// A frame with a child, selected, and the screen point of an empty spot in it.
fn frame_with_child(effects: bool) -> (Ui, figcraft_doc::NodeId, egui::Pos2) {
    let mut s = Session::new();
    let id = |v: serde_json::Value| figcraft_engine::params::id_of(&v["id"]).unwrap();
    let f = id(s.execute("frame.create", &json!({"x": 100, "y": 100, "width": 300, "height": 200})).unwrap());
    s.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 80, "height": 60, "parent": f, "props": {"fill": "#e03030"}})).unwrap();
    if effects {
        s.execute("node.set", &json!({"ids": [f], "effects": [{"type": "dropShadow", "color": "#00000066", "offset": [0, 8], "radius": 24}]}))
            .unwrap();
    }
    s.execute("select.set", &json!({"ids": [f]})).unwrap();
    let mut u = Ui::new(s);
    settle(&mut u);
    let p = u.app.to_screen(350.0, 250.0);
    (u, f, p)
}

fn drag_frames(u: &mut Ui, p: egui::Pos2, n: u32) {
    for k in 1..=n {
        u.frame(vec![Event::PointerMoved(p + vec2(k as f32 * 7.0, k as f32 * 3.0))]);
    }
}

#[test]
fn dragging_a_plain_frame_draws_its_sprite_with_the_outline() {
    let (mut u, f, p) = frame_with_child(false);
    u.button(p, true);
    drag_frames(&mut u, p, 4);
    assert!(u.app.canvas.sprite_rect(&u.app).is_some(), "the sprite exists");
    assert_eq!(u.app.canvas.submitted_skip(), Some(&[f][..]), "the base render leaves the dragged layer out");
    settle(&mut u);
    // The sprite and the outline come from the same shift in the same frame.
    for k in 5..9 {
        u.frame(vec![Event::PointerMoved(p + vec2(k as f32 * 7.0, k as f32 * 3.0))]);
        let ds = u.app.session.ds().unwrap();
        let (base, (dx, dy)) = crate::selection_cache::moved(ds).expect("a plain move");
        let b = figcraft_engine::tools::selection_box(&base, &[f]).unwrap();
        let outline = u.app.to_screen(b[0].x + dx, b[0].y + dy);
        let sprite = u.app.canvas.sprite_rect(&u.app).unwrap();
        assert!((sprite.min - outline).length() < 0.6, "sprite {:?} outline {outline:?}", sprite.min);
        // It is the frame (300 x 200) at the view's zoom, and no move asks the worker for anything.
        let zoom = u.app.session.ds().unwrap().view.zoom;
        assert!((f64::from(sprite.width()) - 300.0 * zoom).abs() < 1.5 && (f64::from(sprite.height()) - 200.0 * zoom).abs() < 1.5, "{sprite:?}");
        assert!(!u.app.worker.as_ref().is_some_and(|w| w.in_flight()), "no render for a move");
    }
    // The base stays put: later moves ask for no new render.
    settle(&mut u);
    let before = u.app.canvas.submitted_skip().map(<[_]>::to_vec);
    u.frame(vec![Event::PointerMoved(p + vec2(90.0, 40.0))]);
    assert_eq!(u.app.canvas.submitted_skip().map(<[_]>::to_vec), before);
    assert!(!u.app.worker.as_ref().is_some_and(|w| w.in_flight()), "no render for a move");
    u.button(p + vec2(90.0, 40.0), false);
    u.frame(vec![]);
    assert_eq!(u.app.canvas.submitted_skip(), Some(&[][..]), "the whole document is rendered again");
    assert!(u.app.canvas.sprite_rect(&u.app).is_some(), "the sprite stays until that render is shown");
    settle(&mut u);
    u.frame(vec![]);
    assert!(u.app.canvas.sprite_rect(&u.app).is_none(), "the sprite goes once that render is shown");
}

#[test]
fn a_frame_with_effects_on_itself_keeps_the_old_path() {
    let (mut u, _f, p) = frame_with_child(true);
    u.button(p, true);
    drag_frames(&mut u, p, 4);
    assert!(u.app.canvas.sprite_rect(&u.app).is_none());
    assert_eq!(u.app.canvas.submitted_skip(), Some(&[][..]));
    u.button(p + vec2(30.0, 10.0), false);
}

#[test]
fn past_the_largest_render_scale_the_drag_keeps_the_old_path() {
    // At 8000% a small frame still fits a texture, but `render_area` renders at most at 64x:
    // the sprite would draw smaller than the layers, so there is none.
    let mut s = Session::new();
    let id = |v: serde_json::Value| figcraft_engine::params::id_of(&v["id"]).unwrap();
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap());
    s.execute("shape.rectangle", &json!({"x": 1, "y": 1, "width": 2, "height": 2, "parent": f, "props": {"fill": "#e03030"}})).unwrap();
    s.execute("select.set", &json!({"ids": [f]})).unwrap();
    let mut u = Ui::new(s);
    settle(&mut u);
    let v = &mut u.app.session.ds_mut().unwrap().view;
    (v.zoom, v.cx, v.cy) = (80.0, 2.0, 2.0);
    settle(&mut u);
    let p = u.app.to_screen(2.0, 2.0);
    u.button(p, true);
    drag_frames(&mut u, p, 4);
    assert_eq!(u.app.session.ds().unwrap().view.zoom, 80.0);
    assert!(u.app.canvas.sprite_rect(&u.app).is_none());
    assert_eq!(u.app.canvas.submitted_skip(), Some(&[][..]));
    u.button(p + vec2(30.0, 10.0), false);
}

#[test]
fn zooming_during_a_drag_ends_the_fast_path() {
    let (mut u, _f, p) = frame_with_child(false);
    u.button(p, true);
    drag_frames(&mut u, p, 4);
    assert!(u.app.canvas.sprite_rect(&u.app).is_some());
    let z = u.app.session.ds().unwrap().view.zoom;
    u.app.session.ds_mut().unwrap().view.zoom = z * 1.5;
    u.frame(vec![Event::PointerMoved(p + vec2(40.0, 12.0))]);
    assert_eq!(u.app.canvas.submitted_skip(), Some(&[][..]), "the zoom ended it");
    // Not started again by the next move of the same gesture.
    u.frame(vec![Event::PointerMoved(p + vec2(48.0, 15.0))]);
    assert_eq!(u.app.canvas.submitted_skip(), Some(&[][..]));
    u.button(p + vec2(48.0, 15.0), false);
}

/// Drag `u`'s frame four frames, with the sprite up.
fn start_drag(u: &mut Ui, p: egui::Pos2) {
    u.button(p, true);
    drag_frames(u, p, 4);
    assert!(u.app.canvas.sprite_rect(&u.app).is_some(), "the sprite exists");
}

#[test]
fn a_cancelled_drag_keeps_its_sprite_until_the_restored_document_shows_and_the_next_drag_has_one() {
    let (mut u, _f, p) = frame_with_child(false);
    start_drag(&mut u, p);
    u.app.session.cancel_interaction().unwrap();
    u.frame(vec![]);
    assert_eq!(u.app.canvas.submitted_skip(), Some(&[][..]), "the restored document is rendered whole");
    assert!(u.app.canvas.sprite_rect(&u.app).is_some(), "the sprite stays until that render is shown");
    u.button(p, false);
    settle(&mut u);
    u.frame(vec![]);
    assert!(u.app.canvas.sprite_rect(&u.app).is_none());
    // The restored document is the one the cancelled drag started from: a new drag of it still gets a sprite.
    start_drag(&mut u, p);
    u.button(p + vec2(30.0, 10.0), false);
}

#[test]
fn a_drag_that_declined_the_sprite_does_not_decline_the_next_drag_of_the_same_document() {
    let (mut u, _f, p) = frame_with_child(false);
    start_drag(&mut u, p);
    let z = u.app.session.ds().unwrap().view.zoom;
    u.app.session.ds_mut().unwrap().view.zoom = z * 1.5;
    u.frame(vec![Event::PointerMoved(p + vec2(40.0, 12.0))]);
    u.app.session.cancel_interaction().unwrap();
    u.button(p, false);
    settle(&mut u);
    u.app.session.ds_mut().unwrap().view.zoom = z;
    settle(&mut u);
    let p = u.app.to_screen(350.0, 250.0);
    start_drag(&mut u, p);
    u.button(p + vec2(30.0, 10.0), false);
}

#[test]
fn deleting_the_dragged_layer_mid_drag_drops_the_sprite_and_renders_normally() {
    let (mut u, f, p) = frame_with_child(false);
    start_drag(&mut u, p);
    u.app.session.cancel_interaction().unwrap();
    u.app.session.execute("edit.delete", &json!({"ids": [f]})).unwrap();
    u.frame(vec![Event::PointerMoved(p + vec2(50.0, 20.0))]);
    assert!(u.app.canvas.sprite_rect(&u.app).is_none(), "nothing to show");
    assert_eq!(u.app.canvas.submitted_skip(), Some(&[][..]));
    u.button(p + vec2(50.0, 20.0), false);
}

#[test]
fn outline_views_keep_the_old_path() {
    for flag in 0..3 {
        let (mut u, _f, p) = frame_with_child(false);
        let v = &mut u.app.session.ds_mut().unwrap().view;
        match flag {
            0 => v.outlines = true,
            1 => v.outlines_selection = true,
            _ => v.outline_hidden = true,
        }
        settle(&mut u);
        u.button(p, true);
        drag_frames(&mut u, p, 4);
        assert!(u.app.canvas.sprite_rect(&u.app).is_none(), "flag {flag}");
        assert_eq!(u.app.canvas.submitted_skip(), Some(&[][..]), "flag {flag}");
        u.button(p + vec2(30.0, 10.0), false);
    }
}

#[test]
fn a_page_switch_does_not_keep_the_drags_base_render_as_the_pages_image() {
    let (mut u, f, p) = frame_with_child(false);
    u.button(p, true);
    drag_frames(&mut u, p, 4);
    settle(&mut u);
    assert_eq!(u.app.canvas.submitted_skip(), Some(&[f][..]), "the render shown leaves the dragged layer out");
    let ds = u.app.session.ds().unwrap();
    let (here, other) = ((ds.uid, ds.page), (ds.uid, figcraft_doc::NodeId(ds.page.0 + 1)));
    assert_eq!(u.app.canvas.tex_of(), Some(here));
    // Switching away while it shows must not keep it: coming back would show the page without the dragged layer.
    u.app.canvas.switch_to(other);
    assert!(!u.app.canvas.keeps(here), "an image without the dragged layers is not kept");
    u.app.canvas.switch_to(here);
    assert!(!u.app.canvas.has_texture(), "so coming back shows no image until the render lands");
    u.button(p + vec2(30.0, 10.0), false);
}
