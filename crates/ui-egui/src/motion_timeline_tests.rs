//! The Motion timeline's clip bar, driven through egui headless: the two ends trim (only the
//! length changes) and the middle moves (only the delay changes). These use the real hit test and
//! the real drag, so a fixed-width edge zone that swallowed a narrow clip would fail here.

use egui::{Event, Modifiers, PointerButton, Pos2, Rect, pos2};
use figcraft_doc::NodeId;
use figcraft_doc::anim::motion::{Clip, ClipKind, ClipMode};
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::layers_drag_tests::Ui;

fn id(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// A rectangle with one 500 ms clip at 0, on a 2 s timeline, selected with Motion open.
fn setup() -> (Ui, NodeId) {
    let mut s = Session::new();
    let layer = id(&s.execute("node.create", &json!({"type": "rectangle", "x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    {
        let doc = s.doc_mut().unwrap();
        if let Some(n) = doc.node_mut(layer) {
            n.motion.clips.push(Clip::new(ClipKind::Position, ClipMode::In).delay(0).dur(500));
            n.motion.normalize();
        }
        doc.mark_dirty([layer]);
    }
    s.execute("select.set", &json!({"ids": [layer]})).unwrap();
    let mut u = Ui::new(s);
    crate::motion::set_open(&mut u.app, true);
    for _ in 0..3 {
        u.frame(vec![]);
    }
    (u, layer)
}

fn clip_of(u: &Ui, layer: NodeId) -> Clip {
    u.app.session.doc().unwrap().node(layer).unwrap().motion.clips[0].clone()
}

/// The clip bar's rect on screen, from the same geometry the panel uses (a bottom panel 309 pt
/// tall on the 1440×900 test screen).
fn bar(u: &Ui, layer: NodeId, timeline_ms: u32) -> Rect {
    let full = Rect::from_min_max(pos2(0.0, 900.0 - crate::motion::timeline::HEIGHT), pos2(1440.0, 900.0));
    let body = Rect::from_min_max(pos2(0.0, full.min.y + 47.0), full.max);
    let track_x = body.min.x + 297.0;
    let track_right = body.max.x;
    let scale = ((track_right - track_x - 9.5 - 16.0).max(40.0)) / timeline_ms as f32;
    let x_of = |t: f32| track_x + 9.5 + t * scale;
    let c = clip_of(u, layer);
    let cy = body.min.y + 8.0 + 24.0 + 12.0; // row 1 (the clip), 24 pt rows
    Rect::from_min_max(pos2(x_of(c.delay_ms as f32), cy - 11.0), pos2(x_of(c.end_ms(timeline_ms) as f32), cy + 11.0))
}

fn drag(u: &mut Ui, from: Pos2, to: Pos2) {
    u.frame(vec![Event::PointerMoved(from)]);
    u.frame(vec![Event::PointerMoved(from), Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
    for i in 1..=6 {
        let p = from + (to - from) * (i as f32 / 6.0);
        u.frame(vec![Event::PointerMoved(p)]);
    }
    u.frame(vec![Event::PointerMoved(to), Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
    u.frame(vec![]);
}

#[test]
fn the_right_end_trims_only_the_length() {
    let (mut u, layer) = setup();
    let r = bar(&u, layer, 2000);
    let before = clip_of(&u, layer);
    drag(&mut u, pos2(r.right() - 2.0, r.center().y), pos2(r.right() + 100.0, r.center().y));
    let after = clip_of(&u, layer);
    assert_eq!(after.delay_ms, before.delay_ms, "trimming the end must not move the clip: {before:?} -> {after:?}");
    assert!(after.duration_ms >= before.duration_ms + 100, "the end must extend: {before:?} -> {after:?}");
}

#[test]
fn the_middle_moves_without_changing_the_length() {
    let (mut u, layer) = setup();
    let r = bar(&u, layer, 2000);
    let before = clip_of(&u, layer);
    drag(&mut u, r.center(), pos2(r.center().x + 100.0, r.center().y));
    let after = clip_of(&u, layer);
    assert_eq!(after.duration_ms, before.duration_ms, "moving must not change the length: {before:?} -> {after:?}");
    assert!(after.delay_ms >= before.delay_ms + 100, "the clip must move: {before:?} -> {after:?}");
}

#[test]
fn a_narrow_clip_still_trims_at_its_right_end() {
    let (mut u, layer) = setup();
    // A 60 s timeline shrinks the 500 ms clip to a few points: a fixed edge zone would make the
    // whole bar TrimStart, so dragging its right end would squeeze the start in instead.
    let page = u.app.session.page().unwrap();
    u.app.session.doc_mut().unwrap().timelines.entry(page).or_default().duration_ms = 60_000;
    u.frame(vec![]);
    let r = bar(&u, layer, 60_000);
    let before = clip_of(&u, layer);
    drag(&mut u, pos2(r.right() - 0.5, r.center().y), pos2(r.right() + 60.0, r.center().y));
    let after = clip_of(&u, layer);
    assert_eq!(after.delay_ms, before.delay_ms, "the start must stay put on a narrow clip: {before:?} -> {after:?}");
    assert!(after.duration_ms > before.duration_ms, "the end must still extend: {before:?} -> {after:?}");
}

/// Two clips (0..500 and 500..1000), so the layer's summary spans 0..1000.
fn setup_two() -> (Ui, NodeId) {
    let mut s = Session::new();
    let layer = id(&s.execute("node.create", &json!({"type": "rectangle", "x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    {
        let doc = s.doc_mut().unwrap();
        if let Some(n) = doc.node_mut(layer) {
            n.motion.clips.push(Clip::new(ClipKind::Position, ClipMode::In).delay(0).dur(500));
            n.motion.clips.push(Clip::new(ClipKind::Scale, ClipMode::In).delay(500).dur(500));
            n.motion.normalize();
        }
        doc.mark_dirty([layer]);
    }
    s.execute("select.set", &json!({"ids": [layer]})).unwrap();
    let mut u = Ui::new(s);
    crate::motion::set_open(&mut u.app, true);
    for _ in 0..3 {
        u.frame(vec![]);
    }
    (u, layer)
}

fn span_of(u: &Ui, layer: NodeId) -> (u32, u32) {
    u.app.session.doc().unwrap().node(layer).unwrap().motion.span_ms(2000).unwrap()
}

/// The layer summary bar (row 0).
fn summary_bar(u: &Ui, layer: NodeId, timeline_ms: u32) -> Rect {
    let full = Rect::from_min_max(pos2(0.0, 900.0 - crate::motion::timeline::HEIGHT), pos2(1440.0, 900.0));
    let body = Rect::from_min_max(pos2(0.0, full.min.y + 47.0), full.max);
    let track_x = body.min.x + 297.0;
    let track_right = body.max.x;
    let scale = ((track_right - track_x - 9.5 - 16.0).max(40.0)) / timeline_ms as f32;
    let x_of = |t: f32| track_x + 9.5 + t * scale;
    let (lo, hi) = u.app.session.doc().unwrap().node(layer).unwrap().motion.span_ms(timeline_ms).unwrap();
    let cy = body.min.y + 8.0 + 12.0; // row 0
    Rect::from_min_max(pos2(x_of(lo as f32), cy - 11.0), pos2(x_of(hi as f32), cy + 11.0))
}

#[test]
fn the_layer_summary_moves_without_squeezing() {
    let (mut u, layer) = setup_two();
    let before = span_of(&u, layer);
    let r = summary_bar(&u, layer, 2000);
    // Left: the animation already starts at 0, so the whole thing must stay put rather than the
    // later clip sliding under the first and squashing the span (the old per-item clamp at 0).
    drag(&mut u, r.center(), pos2(r.center().x - 100.0, r.center().y));
    assert_eq!(span_of(&u, layer), before, "the span must not be squeezed when it cannot move left");
    // Right: everything shifts by one delta and the width is unchanged.
    let r = summary_bar(&u, layer, 2000);
    drag(&mut u, r.center(), pos2(r.center().x + 100.0, r.center().y));
    let after = span_of(&u, layer);
    assert_eq!(after.1 - after.0, before.1 - before.0, "the span must not change width: {before:?} -> {after:?}");
    assert_eq!(after.0 - before.0, after.1 - before.1, "one delta for everything: {before:?} -> {after:?}");
    assert!(after.0 > before.0, "it must move right: {before:?} -> {after:?}");
}

#[test]
fn the_layer_summary_end_stretches() {
    let (mut u, layer) = setup_two();
    let before = span_of(&u, layer);
    let r = summary_bar(&u, layer, 2000);
    drag(&mut u, pos2(r.right() - 2.0, r.center().y), pos2(r.right() + 100.0, r.center().y));
    let after = span_of(&u, layer);
    assert_eq!(after.0, before.0, "the start must stay put: {before:?} -> {after:?}");
    assert!(after.1 > before.1 + 100, "the end must stretch: {before:?} -> {after:?}");
}
