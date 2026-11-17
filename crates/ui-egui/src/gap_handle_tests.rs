//! The on-canvas gap handles of an auto layout frame whose gap is automatic (the lines of a
//! wrapping layout spread across it, or the items spread along it): the handle shows the distance
//! the layout gives the gap, and dragging it makes the gap a fixed one from there, as typing a gap
//! in the panel does. Driven through egui headless.

use egui::{Event, vec2};
use figcraft_doc::{ContentAlign, MainAlign, NodeId};
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::layers_drag_tests::Ui;

fn id(v: Value) -> NodeId {
    figcraft_engine::params::id_of(&v["id"]).unwrap()
}

/// A 300 × 300 frame of three 120 × 60 rectangles with `props` on top of a horizontal layout
/// without padding (gap 10): selected, on screen.
fn scene(props: Value) -> (Ui, NodeId) {
    let mut s = Session::new();
    let mut all = json!({"layout": "horizontal", "padding": 0, "gap": 10, "crossGap": 10});
    all.as_object_mut().unwrap().extend(props.as_object().unwrap().clone());
    let f = id(s.execute("frame.create", &json!({"x": 100, "y": 100, "width": 300, "height": 300, "props": all})).unwrap());
    for _ in 0..3 {
        s.execute("shape.rectangle", &json!({"parent": f, "x": 0, "y": 0, "width": 120, "height": 60})).unwrap();
    }
    s.execute("select.set", &json!({"ids": [f]})).unwrap();
    (Ui::new(s), f)
}

/// Press at page point `from`, move by `by` page units in steps, release.
fn drag(u: &mut Ui, from: (f64, f64), by: (f32, f32)) {
    let p = u.app.to_screen(from.0, from.1);
    let zoom = u.app.session.ds().unwrap().view.zoom as f32;
    u.frame(vec![Event::PointerMoved(p)]);
    u.frame(vec![Event::PointerMoved(p)]);
    u.button(p, true);
    for k in 1..=4 {
        u.frame(vec![Event::PointerMoved(p + vec2(by.0, by.1) * zoom * (k as f32 / 4.0))]);
    }
    u.button(p + vec2(by.0, by.1) * zoom, false);
    u.frame(vec![]);
}

#[test]
fn dragging_the_gap_between_spread_rows_fixes_it_from_where_it_was() {
    // Two rows of 60 in 300: spread, they are 180 apart (the stored row gap of 10 doesn't count).
    let (mut u, f) = scene(json!({"wrap": true, "contentAlign": "spaceBetween"}));
    // Between the rows, at the frame's middle: page (250, 250). 20 down: 180 + 20.
    drag(&mut u, (250.0, 250.0), (0.0, 20.0));
    let l = u.app.session.doc().unwrap().node(f).unwrap().layout.clone().unwrap();
    assert_eq!((l.content_align, l.cross_gap), (ContentAlign::Auto, 200.0));
    // One undo step for the whole drag.
    u.app.run("edit.undo", json!({}));
    let l = u.app.session.doc().unwrap().node(f).unwrap().layout.clone().unwrap();
    assert_eq!((l.content_align, l.cross_gap), (ContentAlign::SpaceBetween, 10.0));
}

#[test]
fn dragging_an_auto_gap_between_items_fixes_it_too_and_fixed_gaps_drag_as_before() {
    // Three 60-wide items spread along 300: (300 - 3 × 60) / 2 = 60 apart.
    let (mut u, f) = scene(json!({"mainAlign": "spaceBetween"}));
    for k in u.app.session.doc().unwrap().node(f).unwrap().children.clone() {
        u.app.run("node.set", json!({"id": k, "props": {"width": 60}}));
    }
    u.frame(vec![]);
    // Between the first two items: x from 160 to 220 on the page, in the items' height.
    drag(&mut u, (190.0, 130.0), (-10.0, 0.0));
    let l = u.app.session.doc().unwrap().node(f).unwrap().layout.clone().unwrap();
    assert_eq!((l.main_align, l.gap), (MainAlign::Min, 50.0));
    // A fixed gap: the drag changes the value and nothing else.
    drag(&mut u, (185.0, 130.0), (6.0, 0.0));
    let l = u.app.session.doc().unwrap().node(f).unwrap().layout.clone().unwrap();
    assert_eq!((l.main_align, l.gap), (MainAlign::Min, 56.0));
}

/// Agent `a` drags a gap handle with raw input, and the batch of agent `b` lets the button go.
/// `b` may not end `a`'s gesture, but the drag is not forgotten for that: the next frame of the
/// window ends it. Forgotten, `a`'s gesture stayed open with nothing left to end it, and held every
/// other agent back.
#[test]
fn a_gap_drag_an_agent_began_is_ended_after_another_agents_release() {
    let (mut u, _) = scene(json!({"wrap": true, "contentAlign": "spaceBetween"}));
    // One batch of raw input of `who`, and the frame that plays it.
    let input = |u: &mut Ui, who: &str, events: Value| {
        let ctx = u.ctx.clone();
        let p = json!({"events": events, "agent": who, "agentKey": format!("key {who}")});
        crate::control::handle(&mut u.app, &ctx, "ui.input", &p).unwrap();
        let events = crate::windows::take_input(&mut u.app, egui::ViewportId::ROOT);
        u.frame(events);
    };
    // Whose gesture is open on the document, if one is.
    let gesture = |u: &Ui| u.app.session.ds().unwrap().interaction.as_ref().map(|it| it.agent.clone());
    // Between the rows, at the frame's middle, as in the first test.
    let p = u.app.to_screen(250.0, 250.0);
    let at = |dy: f32| json!([p.x, p.y + dy]);
    input(&mut u, "a", json!([{"move": at(0.0)}]));
    input(&mut u, "a", json!([{"move": at(0.0)}]));
    input(&mut u, "a", json!([{"down": at(0.0)}]));
    input(&mut u, "a", json!([{"move": at(10.0)}]));
    assert_eq!(gesture(&u), Some(Some("a".to_string())), "calibration: a's drag of the gap is under way");

    input(&mut u, "b", json!([{"up": at(10.0)}]));
    assert_eq!(gesture(&u), Some(Some("a".to_string())), "calibration: b's release did not end a's gesture");
    u.frame(vec![]);
    assert_eq!(gesture(&u), None, "a's gesture was left open after b let the button go");
}
