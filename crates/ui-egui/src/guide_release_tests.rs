//! Ruler guide drags released off the canvas, driven through egui headless: the gesture ends there,
//! and a later click on the canvas neither adds a guide nor moves one.

use egui::{Event, Pos2, vec2};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

fn rulers() -> Ui {
    let mut s = Session::new();
    s.execute("view.rulers", &json!({"on": true})).unwrap();
    Ui::new(s)
}

fn offsets(u: &Ui) -> Vec<f64> {
    let ds = u.app.session.ds().unwrap();
    ds.doc.node(ds.page).unwrap().guides().iter().map(|g| g.offset).collect()
}

fn drag(u: &mut Ui, from: Pos2, to: Pos2) {
    u.frame(vec![Event::PointerMoved(from)]);
    u.button(from, true);
    u.frame(vec![Event::PointerMoved(to)]);
    u.button(to, false);
}

fn click(u: &mut Ui, p: Pos2) {
    u.frame(vec![Event::PointerMoved(p)]);
    u.button(p, true);
    u.button(p, false);
}

#[test]
fn a_ruler_drag_released_off_the_canvas_adds_no_guide_on_a_later_click() {
    for left in [false, true] {
        let mut u = rulers();
        let r = u.app.canvas.rect;
        let start = if left { Pos2::new(r.min.x + 5.0, r.min.y + 60.0) } else { Pos2::new(r.min.x + 60.0, r.min.y + 5.0) };
        drag(&mut u, start, Pos2::new(r.min.x - 100.0, r.center().y));
        let released = offsets(&u);
        click(&mut u, r.center() + vec2(30.0, 60.0));
        assert_eq!(offsets(&u), released, "left ruler: {left}");
    }
}

#[test]
fn a_ruler_drag_released_off_the_canvas_ends_even_when_the_next_frame_presses() {
    let mut u = rulers();
    let r = u.app.canvas.rect;
    let start = Pos2::new(r.min.x + 60.0, r.min.y + 5.0);
    let off = Pos2::new(r.min.x - 100.0, r.center().y);
    u.frame(vec![Event::PointerMoved(start)]);
    u.button(start, true);
    u.frame(vec![Event::PointerMoved(off)]);
    u.button(off, false);
    let p = r.center() + vec2(30.0, 60.0);
    u.button(p, true);
    u.button(p, false);
    assert!(offsets(&u).is_empty());
}

#[test]
fn a_guide_moved_off_the_canvas_doesnt_follow_a_later_click() {
    let mut u = rulers();
    let r = u.app.canvas.rect;
    drag(&mut u, Pos2::new(r.min.x + 60.0, r.min.y + 5.0), r.center());
    assert_eq!(offsets(&u).len(), 1);
    drag(&mut u, r.center(), Pos2::new(r.min.x - 100.0, r.center().y));
    let released = offsets(&u);
    click(&mut u, r.center() + vec2(30.0, 60.0));
    assert_eq!(offsets(&u), released);
}

#[test]
fn a_ruler_drag_released_on_the_canvas_adds_one_guide() {
    let mut u = rulers();
    let r = u.app.canvas.rect;
    drag(&mut u, Pos2::new(r.min.x + 60.0, r.min.y + 5.0), r.center());
    assert_eq!(offsets(&u).len(), 1);
    click(&mut u, r.center() + vec2(30.0, 60.0));
    assert_eq!(offsets(&u).len(), 1);
    u.app.session.execute("edit.undo", &json!({})).unwrap();
    assert!(offsets(&u).is_empty());
}
