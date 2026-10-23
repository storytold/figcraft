//! Prototype text inputs in the player: clicking one focuses it, typing replaces its text then
//! appends (through its `characters` variable when bound), Backspace deletes, Escape or a click
//! elsewhere leaves it, and keys and timers behave while it has the keyboard.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, pos2};
use figcraft_doc::{NodeId, VariableValue};
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::present_tests::{Ui as Harness, id};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn presenter(u: &Harness) -> Value {
    u.app.inspect()["presenter"].clone()
}

fn screen(u: &Harness) -> String {
    presenter(u)["name"].as_str().unwrap_or_default().to_string()
}

fn characters(u: &Harness, layer: NodeId) -> String {
    u.app
        .ui
        .present
        .as_ref()
        .and_then(|p| p.design.as_ref())
        .and_then(|d| d.node(layer))
        .and_then(|n| n.kind.text())
        .map_or_else(String::new, |t| t.characters.clone())
}

fn variable(u: &Harness, layer: NodeId) -> Option<VariableValue> {
    let d = u.app.ui.present.as_ref()?.design.as_ref()?;
    let id = d.variables.get("name")?.id.clone();
    d.variables.resolve(&id, &d.modes_for(layer))
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

fn click(u: &mut Harness, at: Pos2) {
    u.frame(vec![button(at, true)]);
    u.frame(vec![button(at, false)]);
}

fn key(u: &mut Harness, key: Key) {
    let k = |pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    u.frame(vec![k(true), k(false)]);
}

/// A presented frame A with a text input "Guest" (bound to `name` unless `bind` is false) whose
/// own click would navigate to B, a button with a click to B and a key A trigger, and optionally
/// an after-delay timer on the button.
fn setup(bind: bool, timer: Option<u64>) -> (Harness, NodeId, NodeId) {
    let mut s = Session::new();
    let a = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 400, "name": "A"})));
    let b = id(&ex(&mut s, "frame.create", json!({"x": 400, "y": 0, "width": 300, "height": 400, "name": "B"})));
    let t = id(&ex(&mut s, "text.create", json!({"text": "Guest", "x": 20, "y": 20, "parent": a})));
    let btn = id(&ex(&mut s, "shape.rectangle", json!({"x": 20, "y": 120, "width": 100, "height": 40, "parent": a})));
    ex(&mut s, "variable.createCollection", json!({"name": "Form", "modes": ["Default", "Dark"]}));
    ex(&mut s, "variable.create", json!({"collection": "Form", "name": "name", "type": "string", "value": ""}));
    if bind {
        ex(&mut s, "variable.bind", json!({"ids": [t], "field": "characters", "variable": "name"}));
    }
    ex(&mut s, "proto.input.set", json!({"ids": [t]}));
    ex(&mut s, "proto.interaction.add", json!({"id": t, "dest": b}));
    ex(&mut s, "proto.interaction.add", json!({"id": btn, "dest": b}));
    ex(&mut s, "proto.interaction.add", json!({"id": btn, "trigger": {"type": "keyPress", "key": "A"}, "action": {"type": "navigate", "dest": b}}));
    if let Some(ms) = timer {
        ex(
            &mut s,
            "proto.interaction.add",
            json!({"id": btn, "trigger": {"type": "afterDelay", "ms": ms}, "action": {"type": "navigate", "dest": b}}),
        );
    }
    let mut u = Harness::new(s);
    u.run("proto.present", json!({"start": a}));
    u.frames(2, Vec::new);
    (u, a, t)
}

#[test]
fn clicking_an_input_focuses_it_and_typing_replaces_then_appends() {
    let (mut u, _, t) = setup(true, None);
    let at = hotspot(&u, t).center();
    click(&mut u, at);
    assert_eq!(presenter(&u)["focus"], json!(t), "focused");
    assert_eq!(screen(&u), "A", "the input takes the click from its own interaction");
    u.frame(vec![Event::Text("Ada".into())]);
    assert_eq!(characters(&u, t), "Ada", "the first typing replaces the text");
    assert_eq!(variable(&u, t), Some(VariableValue::String("Ada".into())), "through the bound variable");
    u.frame(vec![Event::Text("!".into())]);
    assert_eq!(characters(&u, t), "Ada!");
    key(&mut u, Key::Backspace);
    assert_eq!(characters(&u, t), "Ada");
    // The prototype's key triggers stay quiet while the input has the keyboard.
    key(&mut u, Key::A);
    assert_eq!(screen(&u), "A", "key trigger suppressed");
    assert_eq!(presenter(&u)["focus"], json!(t), "still focused");
    // Escape leaves it.
    key(&mut u, Key::Escape);
    assert_eq!(presenter(&u)["focus"], Value::Null);
    assert_eq!(characters(&u, t), "Ada");
}

#[test]
fn clicking_away_leaves_the_input_and_clicking_again_starts_over() {
    let (mut u, _, t) = setup(true, None);
    let hot = hotspot(&u, t);
    click(&mut u, hot.center());
    u.frame(vec![Event::Text("Ada".into())]);
    // An empty part of A.
    click(&mut u, pos2(hot.min.x + 240.0, hot.min.y + 250.0));
    assert_eq!(presenter(&u)["focus"], Value::Null, "click away");
    assert_eq!(screen(&u), "A");
    // Focusing again selects the text: the next typing replaces it all.
    let at = hotspot(&u, t).center();
    click(&mut u, at);
    assert_eq!(presenter(&u)["focus"], json!(t));
    u.frame(vec![Event::Text("Z".into())]);
    assert_eq!(characters(&u, t), "Z");
    assert_eq!(variable(&u, t), Some(VariableValue::String("Z".into())));
}

#[test]
fn an_unbound_input_edits_its_own_text() {
    let (mut u, _, t) = setup(false, None);
    let at = hotspot(&u, t).center();
    click(&mut u, at);
    assert_eq!(presenter(&u)["focus"], json!(t));
    u.frame(vec![Event::Text("Ada".into())]);
    assert_eq!(characters(&u, t), "Ada");
    key(&mut u, Key::Backspace);
    assert_eq!(characters(&u, t), "Ad");
    assert_eq!(variable(&u, t), Some(VariableValue::String(String::new())), "no variable written");
}

#[test]
fn timers_keep_running_while_typing_and_leaving_drops_the_input() {
    let (mut u, _, t) = setup(true, Some(100));
    let at = hotspot(&u, t).center();
    click(&mut u, at);
    u.frame(vec![Event::Text("Ada".into())]);
    assert_eq!(presenter(&u)["focus"], json!(t));
    u.frames(10, Vec::new);
    assert_eq!(screen(&u), "B", "the after-delay action ran");
    assert_eq!(presenter(&u)["focus"], Value::Null, "leaving the screen drops the input");
}
