use egui::{Event, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use figcraft_doc::NodeId;
use figcraft_doc::proto::{OverlayAnchor, OverlayPosition};
use figcraft_engine::Session;
use serde_json::{Value, json};

use super::anchored_overlay_rect;
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

fn hotspot(u: &Harness, layer: NodeId) -> Rect {
    let hot = u.app.inspect()["presenter"]["hotspots"].as_array().cloned().unwrap_or_default();
    let h = hot.iter().find(|h| h["id"] == json!(layer)).unwrap_or_else(|| panic!("no hotspot on {layer}: {hot:?}"));
    let r: Vec<f32> = h["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
    Rect::from_min_max(pos2(r[0], r[1]), pos2(r[2], r[3]))
}

fn button(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

#[test]
fn presets_sit_on_the_trigger_or_on_the_screen() {
    let base = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 800.0));
    let size = vec2(60.0, 200.0);
    let trigger = Rect::from_min_size(pos2(30.0, 100.0), vec2(100.0, 40.0));
    let at = |p, a, t| anchored_overlay_rect(p, a, base, size, t, None).min;
    use OverlayAnchor::*;
    use OverlayPosition as P;
    for (p, a, want, why) in [
        (P::BottomLeft, TriggerOutside, pos2(30.0, 140.0), "below the trigger, left edges lined up"),
        (P::BottomRight, TriggerOutside, pos2(70.0, 140.0), "below, right edges: 30 + 100 - 60"),
        (P::TopCenter, TriggerOutside, pos2(50.0, -100.0), "above, centred: 30 + 20, 100 - 200"),
        (P::TopLeft, TriggerInside, pos2(30.0, 100.0), "on the trigger's top-left"),
        (P::Center, TriggerInside, pos2(50.0, 20.0), "centred on the trigger"),
        (P::BottomLeft, Screen, pos2(0.0, 600.0), "the screen's bottom left, as before"),
    ] {
        assert_eq!(at(p, a, Some(trigger)), want, "{p:?} {a:?}: {why}");
    }
    assert_eq!(at(P::BottomLeft, TriggerOutside, None), pos2(0.0, 600.0), "opened by nothing on screen: the screen's preset");
}

#[test]
fn a_dropdown_opens_right_under_its_button() {
    let mut s = Session::new();
    let a = frame(&mut s, "A", 0.0, 300.0, 400.0);
    let menu = frame(&mut s, "Menu", 400.0, 120.0, 80.0);
    let hot = rect(&mut s, a, [50.0, 50.0, 100.0, 30.0]);
    let item = rect(&mut s, menu, [0.0, 0.0, 120.0, 20.0]);
    let open = json!({"type": "openOverlay", "dest": menu, "position": "bottomLeft", "anchor": "triggerOutside"});
    ex(&mut s, "proto.interaction.add", json!({"id": hot, "action": open, "transition": {"type": "instant"}}));
    ex(&mut s, "proto.interaction.add", json!({"id": item, "action": {"type": "closeOverlay"}, "transition": {"type": "instant"}}));
    let mut u = Harness::new(s);
    u.run("proto.present", json!({"start": a}));
    u.frames(2, Vec::new);
    let scale = |u: &mut Harness, scale: &str| {
        u.run("proto.presentOptions", json!({"scale": scale}));
        u.frames(2, Vec::new);
    };
    scale(&mut u, "fill");
    let filled = hotspot(&u, hot);
    scale(&mut u, "fit");
    let h = hotspot(&u, hot);
    assert_ne!(filled, h, "fill shows the frame at another size");
    u.frame(vec![Event::PointerMoved(h.center())]);
    u.frame(vec![button(h.center(), true)]);
    u.frame(vec![button(h.center(), false)]);
    u.frames(2, Vec::new);
    let under = |u: &Harness, b: Rect| {
        let (got, want) = (hotspot(u, item).min, pos2(b.min.x, b.max.y));
        assert!((got - want).length() < 0.01, "{got:?} vs {want:?}");
    };
    under(&u, h);
    scale(&mut u, "fill");
    under(&u, filled);
}
