//! Delays on mouse enter, leave, down and up triggers, and manual overlay positions, in the
//! player: gestures go through egui headless on the harness's clock (a frame is 1/60 s).

use egui::{Event, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use figcraft_doc::NodeId;
use figcraft_doc::proto::OverlayPosition;
use figcraft_engine::Session;
use serde_json::{Value, json};

use super::overlay_rect;
use crate::present_tests::{Ui as Harness, id};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn frame(s: &mut Session, name: &str, x: f64, w: f64, h: f64) -> NodeId {
    id(&ex(s, "frame.create", json!({"x": x, "y": 0, "width": w, "height": h, "name": name})))
}

fn rect(s: &mut Session, parent: NodeId, [x, y, w, h]: [f64; 4]) -> NodeId {
    id(&ex(s, "shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h, "parent": parent})))
}

fn presenter(u: &Harness) -> Value {
    u.app.inspect()["presenter"].clone()
}

fn screen(u: &Harness) -> String {
    presenter(u)["name"].as_str().unwrap_or_default().to_string()
}

fn hotspot(u: &Harness, layer: NodeId) -> Rect {
    let hot = presenter(u)["hotspots"].as_array().cloned().unwrap_or_default();
    let h = hot.iter().find(|h| h["id"] == json!(layer)).unwrap_or_else(|| panic!("no hotspot on {layer}: {hot:?}"));
    let r: Vec<f32> = h["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
    Rect::from_min_max(pos2(r[0], r[1]), pos2(r[2], r[3]))
}

fn button(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

/// A (presented) with a 100 × 60 "Hot" layer that goes to B on `trigger` after `delay` ms.
fn delayed(trigger: &str, delay: u32) -> (Harness, Pos2) {
    let mut s = Session::new();
    let a = frame(&mut s, "A", 0.0, 300.0, 400.0);
    let b = frame(&mut s, "B", 400.0, 300.0, 400.0);
    let hot = rect(&mut s, a, [50.0, 50.0, 100.0, 60.0]);
    let it = json!({"id": hot, "trigger": {"type": trigger}, "action": {"type": "navigate", "dest": b}, "transition": {"type": "instant"}, "delayMs": delay});
    ex(&mut s, "proto.interaction.add", it);
    let mut u = Harness::new(s);
    u.run("proto.present", json!({"start": a}));
    u.frames(2, Vec::new);
    let at = hotspot(&u, hot).center();
    (u, at)
}

/// The gesture that sets `trigger` off on the layer at `at`.
fn gesture(u: &mut Harness, trigger: &str, at: Pos2) {
    let away = at - vec2(80.0, 45.0);
    u.frame(vec![Event::PointerMoved(at)]);
    match trigger {
        "mouseEnter" => {}
        "mouseLeave" => u.frame(vec![Event::PointerMoved(away)]),
        "mouseDown" => u.frame(vec![button(at, true)]),
        _ => {
            u.frame(vec![button(at, true)]);
            u.frame(vec![button(at, false)]);
        }
    }
}

#[test]
fn mouse_triggers_wait_out_their_delay() {
    for trigger in ["mouseEnter", "mouseLeave", "mouseDown", "mouseUp"] {
        // 500 ms is 30 frames: not yet after 20 more, done after 40.
        let (mut u, at) = delayed(trigger, 500);
        gesture(&mut u, trigger, at);
        u.frames(20, Vec::new);
        assert_eq!(screen(&u), "A", "{trigger}: not before the delay");
        u.frames(20, Vec::new);
        assert_eq!(screen(&u), "B", "{trigger}: once the delay is over");
        // Without a delay it acts at once.
        let (mut u, at) = delayed(trigger, 0);
        gesture(&mut u, trigger, at);
        u.frames(2, Vec::new);
        assert_eq!(screen(&u), "B", "{trigger}: no delay");
    }
}

#[test]
fn a_delayed_trigger_fires_once_and_not_on_another_screen() {
    // Entering twice while it waits doesn't queue it twice (the documentation is silent on a
    // pointer that leaves early: it still fires).
    let (mut u, at) = delayed("mouseEnter", 500);
    for _ in 0..3 {
        u.frame(vec![Event::PointerMoved(at)]);
        u.frame(vec![Event::PointerMoved(at - vec2(80.0, 45.0))]);
    }
    assert_eq!(presenter(&u)["waiting"], 1);
    u.frames(40, Vec::new);
    assert_eq!((screen(&u), presenter(&u)["waiting"].clone()), ("B".to_string(), json!(0)));
    // Restarting (or leaving the screen any other way) drops what was waiting.
    let (mut u, at) = delayed("mouseDown", 500);
    gesture(&mut u, "mouseDown", at);
    u.frame(vec![button(at, false)]);
    u.frame(vec![Event::Key { key: egui::Key::R, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
    u.frames(40, Vec::new);
    assert_eq!((screen(&u), presenter(&u)["waiting"].clone()), ("A".to_string(), json!(0)), "restarted while waiting");
}

#[test]
fn a_manual_overlay_sits_at_its_offset_from_the_layer_that_opens_it() {
    let (base, size) = (Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 800.0)), vec2(200.0, 100.0));
    let anchor = Rect::from_min_size(pos2(30.0, 40.0), vec2(50.0, 20.0));
    let at = |offset| overlay_rect(OverlayPosition::Manual, base, size, Some(anchor), offset).min;
    // (offset, top-left, derivation)
    for (offset, want, why) in [
        (None, pos2(30.0, 60.0), "no offset: right below the layer"),
        (Some(vec2(0.0, 0.0)), pos2(30.0, 40.0), "0, 0: on the layer's top-left"),
        (Some(vec2(12.0, -8.0)), pos2(42.0, 32.0), "30 + 12, 40 - 8"),
    ] {
        assert_eq!(at(offset), want, "{why}");
    }
    assert_eq!(
        overlay_rect(OverlayPosition::Manual, base, size, None, Some(vec2(5.0, 5.0))).min,
        pos2(100.0, 350.0),
        "opened by nothing on screen: centred"
    );
    assert_eq!(
        overlay_rect(OverlayPosition::TopLeft, base, size, Some(anchor), Some(vec2(5.0, 5.0))).min,
        pos2(0.0, 0.0),
        "other positions ignore it"
    );
}

#[test]
fn the_player_places_a_manual_overlay_in_page_units() {
    // "Hot" at (50, 50) in A opens "Menu" at offset (20, 30); a layer at (10, 10) in Menu shows
    // where the overlay landed: Hot's top-left + (20 + 10, 30 + 10) page units.
    let mut s = Session::new();
    let a = frame(&mut s, "A", 0.0, 300.0, 400.0);
    let menu = frame(&mut s, "Menu", 400.0, 120.0, 80.0);
    let hot = rect(&mut s, a, [50.0, 50.0, 100.0, 60.0]);
    let item = rect(&mut s, menu, [10.0, 10.0, 40.0, 20.0]);
    let open = json!({"type": "openOverlay", "dest": menu, "position": "manual", "offset": [20, 30]});
    ex(&mut s, "proto.interaction.add", json!({"id": hot, "action": open, "transition": {"type": "instant"}}));
    ex(&mut s, "proto.interaction.add", json!({"id": item, "action": {"type": "closeOverlay"}, "transition": {"type": "instant"}}));
    let mut u = Harness::new(s);
    u.run("proto.present", json!({"start": a}));
    u.frames(2, Vec::new);
    let h = hotspot(&u, hot);
    let scale = h.width() / 100.0;
    u.frame(vec![Event::PointerMoved(h.center())]);
    u.frame(vec![button(h.center(), true)]);
    u.frame(vec![button(h.center(), false)]);
    u.frames(2, Vec::new);
    let got = hotspot(&u, item).min;
    let want = h.min + vec2(30.0, 40.0) * scale;
    assert!((got - want).length() < 0.01, "{got:?} vs {want:?} at scale {scale}");
}
