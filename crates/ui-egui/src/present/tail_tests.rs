//! The player's side of the prototyping tail, written from the public documentation: controller
//! buttons play key/gamepad triggers (from the system's controllers and injected through the
//! control channel; no physical controller needed), and widgets react to clicks in a presented
//! prototype. Each case states the rule and a one-line derivation; "docs silent" cases pin what
//! FigCraft does. The model's side is the engine's `proto_tail_tests`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use base64::Engine as _;
use egui::{Event, PointerButton, Rect, pos2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::Services;
use crate::present_tests::{Ui as Harness, id};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn frame(s: &mut Session, name: &str, x: f64) -> NodeId {
    id(&ex(s, "frame.create", json!({"x": x, "y": 0, "width": 300, "height": 400, "name": name})))
}

fn key_trigger(s: &mut Session, layer: NodeId, key: &str, action: Value) {
    ex(
        s,
        "proto.interaction.add",
        json!({"id": layer, "trigger": {"type": "keyPress", "key": key}, "action": action, "transition": {"type": "instant"}}),
    );
}

fn screen(u: &Harness) -> String {
    u.app.inspect()["presenter"]["name"].as_str().unwrap_or_default().to_string()
}

/// Controllers as the system reports them: the buttons held (bit i: standard index i), and how
/// often the player asked.
struct FakePads {
    held: Arc<AtomicU32>,
    polls: Arc<AtomicUsize>,
}

impl Services for FakePads {
    fn gamepad_held(&mut self) -> u32 {
        self.polls.fetch_add(1, Ordering::Relaxed);
        self.held.load(Ordering::Relaxed)
    }
}

/// Screens A (presented) and B: a layer on A whose "Gamepad A" trigger goes to B, one on B whose
/// "Gamepad B" trigger goes back; the fake controllers attached.
fn pads_setup() -> (Harness, Arc<AtomicU32>, Arc<AtomicUsize>) {
    let mut s = Session::new();
    let a = frame(&mut s, "A", 0.0);
    let b = frame(&mut s, "B", 400.0);
    let on_a = id(&ex(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 30, "parent": a})));
    let on_b = id(&ex(&mut s, "shape.rectangle", json!({"x": 410, "y": 10, "width": 30, "height": 30, "parent": b})));
    key_trigger(&mut s, on_a, "Gamepad A", json!({"type": "navigate", "dest": b}));
    key_trigger(&mut s, on_b, "Gamepad B", json!({"type": "back"}));
    let (held, polls) = (Arc::new(AtomicU32::new(0)), Arc::new(AtomicUsize::new(0)));
    let mut u = Harness::new(s);
    u.app.services = Box::new(FakePads { held: held.clone(), polls: polls.clone() });
    u.run("proto.present", json!({"start": a}));
    (u, held, polls)
}

#[test]
fn controller_buttons_play_key_gamepad_triggers() {
    let (mut u, held, polls) = pads_setup();
    assert_eq!(u.app.inspect()["presenter"]["gamepad"], json!(true));
    assert_eq!(screen(&u), "A");
    // (buttons held, screen after, derivation)
    let steps = [
        (0b01, "B", "pressing the trigger's button anywhere on the screen plays it, like its key"),
        (0b01, "B", "a button held down plays once, not again every frame"),
        (0b11, "A", "another button while one is held: B's trigger goes back"),
        (0b00, "A", "releasing does nothing (the trigger is a press)"),
        (1 << 2, "A", "a button no trigger names does nothing"),
        (0b01, "B", "pressed again after letting go: plays again"),
    ];
    for (bits, want, why) in steps {
        held.store(bits, Ordering::Relaxed);
        u.frames(2, Vec::new);
        assert_eq!(screen(&u), want, "held {bits:#b}: {why}");
    }
    assert!(polls.load(Ordering::Relaxed) >= steps.len() * 2, "polled every frame while presenting");
    // The control channel presses buttons too: by short name, trigger name or index.
    let ctx = u.ctx.clone();
    held.store(0, Ordering::Relaxed);
    for (button, want) in [(json!("B"), "A"), (json!("gamepad a"), "B"), (json!(1), "A")] {
        crate::control::handle(&mut u.app, &ctx, "ui.input", &json!({"events": [{"gamepad": button}]})).unwrap();
        u.frames(2, Vec::new);
        assert_eq!(screen(&u), want, "{button}");
    }
    for junk in [json!("Z"), json!(17), json!(-1), json!(null), json!([0])] {
        let e = crate::control::handle(&mut u.app, &ctx, "ui.input", &json!({"events": [{"gamepad": junk}]})).unwrap_err();
        assert!(e.contains("unknown controller button"), "{junk}: {e}");
    }
}

#[test]
fn prototypes_without_gamepad_triggers_dont_poll() {
    let mut s = Session::new();
    let a = frame(&mut s, "A", 0.0);
    let held = Arc::new(AtomicU32::new(1));
    let polls = Arc::new(AtomicUsize::new(0));
    let mut u = Harness::new(s);
    u.app.services = Box::new(FakePads { held, polls: polls.clone() });
    let ctx = u.ctx.clone();
    // Nothing plays: a controller button needs a prototype to go to.
    let e = crate::control::handle(&mut u.app, &ctx, "ui.input", &json!({"events": [{"gamepad": "A"}]})).unwrap_err();
    assert!(e.contains("prototype that is playing"), "{e}");
    u.run("proto.present", json!({"start": a}));
    u.frames(5, Vec::new);
    assert_eq!(u.app.inspect()["presenter"]["gamepad"], json!(false));
    assert_eq!(polls.load(Ordering::Relaxed), 0, "no gamepad triggers: the controllers are left alone");
}

/// A widget plugin: inserted it shows "one" (state n: 1) with a clickable square `inc`; clicked,
/// it shows "two" (n: 2).
fn widget_module() -> Vec<u8> {
    let look = |n: u32, label: &str| {
        json!({"state": {"n": n}, "root": {"type": "frame", "name": "Tally", "layout": "horizontal", "gap": 8, "padding": 10, "fill": "#FFFFFF",
            "children": [{"type": "text", "text": label, "size": 12}, {"type": "rectangle", "target": "inc", "width": 40, "height": 40, "fill": "#0D99FF"}]}})
        .to_string()
    };
    let (one, two) = (look(1, "one"), look(2, "two"));
    // Clicked when the input holds `"tar` (the event's target), read as one little-endian i32.
    let wat = format!(
        r#"(module
          (import "figcraft" "reply_read" (func $reply_read (param i32 i32) (result i32)))
          (import "figcraft" "widget_render" (func $render (param i32 i32) (result i32)))
          (memory (export "memory") 2)
          (data (i32.const 1000) "{}") (data (i32.const 5000) "{}")
          (func (export "figcraft_run") (param $n i32) (result i32) (local $i i32)
            (drop (call $reply_read (i32.const 20000) (local.get $n)))
            (block $done (loop $scan
              (br_if $done (i32.gt_s (i32.add (local.get $i) (i32.const 4)) (local.get $n)))
              (if (i32.eq (i32.load (i32.add (i32.const 20000) (local.get $i))) (i32.const 0x72617422))
                (then (drop (call $render (i32.const 5000) (i32.const {}))) (return (i32.const 0))))
              (local.set $i (i32.add (local.get $i) (i32.const 1)))
              (br $scan)))
            (drop (call $render (i32.const 1000) (i32.const {})))
            (i32.const 0)))"#,
        one.replace('"', "\\\""),
        two.replace('"', "\\\""),
        two.len(),
        one.len()
    );
    wat::parse_str(&wat).unwrap()
}

/// Screen A (presented) holding the widget; the widget and its clickable square.
fn widget_setup() -> (Harness, NodeId, NodeId) {
    let mut s = Session::new();
    let manifest = json!({"id": "org.test.tally", "name": "Tally", "version": "1.0.0", "main": "w.wasm", "api": 3, "widget": true, "permissions": {"access": "read"}});
    ex(&mut s, "plugin.import", json!({"manifest": manifest, "wasm": base64::engine::general_purpose::STANDARD.encode(widget_module())}));
    let a = frame(&mut s, "A", 0.0);
    let w = id(&ex(&mut s, "widget.insert", json!({"plugin": "org.test.tally", "x": 40, "y": 40})));
    ex(&mut s, "node.reparent", json!({"ids": [w], "parent": a}));
    let doc = s.doc().unwrap();
    let square = doc.subtree(w).into_iter().find(|i| doc.node(*i).is_some_and(|n| n.plugin_data.contains_key("figcraft.widget.target"))).unwrap();
    let mut u = Harness::new(s);
    u.run("proto.present", json!({"start": a}));
    u.frames(2, Vec::new);
    (u, w, square)
}

fn state(doc: &figcraft_doc::Document, w: NodeId) -> String {
    doc.node(w).and_then(|n| n.plugin_data.get("figcraft.widget.state").cloned()).unwrap_or_default()
}

/// Where layer `id` shows in the presenter.
fn on_screen(u: &Harness, id: NodeId) -> Rect {
    let p = u.app.ui.present.as_ref().unwrap();
    let (_, place, _) = p.shown.unwrap();
    place.rect(p.doc.as_ref().unwrap().world_bounds(id).unwrap())
}

fn toast(u: &Harness) -> Option<String> {
    u.app.ui.toast.as_ref().map(|t| t.0.clone())
}

fn click(u: &mut Harness, at: egui::Pos2) {
    let b = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    u.frame(vec![Event::PointerMoved(at)]);
    u.frame(vec![b(true)]);
    u.frame(vec![b(false)]);
    u.frames(2, Vec::new);
}

#[test]
fn widgets_react_to_clicks_in_the_presented_prototype() {
    let (mut u, w, square) = widget_setup();
    let player = |u: &Harness| state(u.app.ui.present.as_ref().unwrap().doc.as_ref().unwrap(), w);
    let undo = u.app.session.ds().unwrap().history.undo.len();
    assert_eq!(player(&u), r#"{"n":1}"#);
    // A click on a clickable part calls the plugin back, which redraws the widget in the prototype.
    let r = on_screen(&u, square);
    click(&mut u, r.center());
    assert_eq!(player(&u), r#"{"n":2}"#, "the widget reacted in the prototype");
    // Docs silent (pinned): the state is the prototype's; the file, its history and the canvas
    // keep theirs, and restarting starts over.
    assert_eq!(state(u.app.session.doc().unwrap(), w), r#"{"n":1}"#);
    assert_eq!(u.app.session.ds().unwrap().history.undo.len(), undo);
    assert_eq!(toast(&u), None, "no error");
    let r_key = |pressed| Event::Key { key: egui::Key::R, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    u.frame(vec![r_key(true), r_key(false)]);
    u.frames(2, Vec::new);
    assert_eq!(player(&u), r#"{"n":1}"#, "restarted");
    // A click on the widget outside its clickable parts, or beside it, calls nothing.
    let whole = on_screen(&u, w);
    click(&mut u, pos2(whole.min.x + 3.0, whole.min.y + 3.0));
    click(&mut u, pos2(whole.max.x + 30.0, whole.max.y + 30.0));
    assert_eq!(player(&u), r#"{"n":1}"#);
}

#[test]
fn a_widget_without_its_plugin_says_so_and_stays() {
    let (mut u, w, square) = widget_setup();
    u.app.session.execute("plugin.remove", &json!({"id": "org.test.tally"})).unwrap();
    let r = on_screen(&u, square);
    click(&mut u, r.center());
    let p = u.app.ui.present.as_ref().unwrap();
    assert_eq!(state(p.doc.as_ref().unwrap(), w), r#"{"n":1}"#);
    assert!(toast(&u).is_some_and(|t| t.contains("no plugin")), "{:?}", toast(&u));
}

#[test]
fn the_interaction_editor_picks_a_controller_button() {
    let mut s = Session::new();
    let a = frame(&mut s, "A", 0.0);
    let layer = id(&ex(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 30, "parent": a})));
    key_trigger(&mut s, layer, "K", json!({"type": "back"}));
    ex(&mut s, "select.set", json!({"ids": [layer]}));
    let mut u = crate::layers_drag_tests::Ui::new(s);
    u.app.ui.right_tab = crate::RightTab::Prototype;
    crate::proto_panel::open_editor(&u.ctx, Some((layer, 0)));
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let at = |u: &mut crate::layers_drag_tests::Ui, name: &str| {
        let nodes = u.frame_output(vec![]).platform_output.accesskit_update.unwrap().nodes;
        let b = nodes.iter().map(|(_, n)| n).find(|n| n.label() == Some(name) || n.value() == Some(name)).and_then(|n| n.bounds());
        let b = b.unwrap_or_else(|| panic!("no control {name:?}"));
        let p = egui::Pos2::new(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32);
        u.button(p, true);
        u.button(p, false);
    };
    // The key field keeps recording keys; the menu beside it names the controller's buttons.
    at(&mut u, "Controller");
    // Each with what PlayStation and Switch controllers call the same button.
    let shown = crate::live_tests::flat(&u.frame_output(vec![]).shapes);
    for names in ["✕ · B", "□ · Y", "L2 · ZL", "Options · +", "PS · Home"] {
        assert!(crate::live_tests::has_text(&shown, names), "{names}");
    }
    at(&mut u, "Gamepad Menu");
    let trigger = u.app.session.doc().unwrap().node(layer).unwrap().interactions[0].trigger.clone();
    assert_eq!(trigger, figcraft_doc::proto::Trigger::KeyPress { key: "Gamepad Menu".into() });
}
