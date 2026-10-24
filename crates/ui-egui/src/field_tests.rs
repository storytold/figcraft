//! Number fields: what typed text means (arithmetic, units, edits relative to the value), how far
//! a scrub and an arrow key move a value, and the Design panel's fields driven through egui
//! headless: scrubbing by the label with and without Shift and Alt, arrow keys, arithmetic on
//! commit, Tab and Shift+Tab, Escape, fields showing "Mixed", and one undo step per scrub or
//! commit (the colour picker's, the prototype panel's and the Scale panel's fields too).

use egui::accesskit::Role;
use egui::{Event, Id, Key, Modifiers, Pos2, pos2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::a11y::focused_name;
use crate::field_math::{Rel, Typed, eval, parse, stepped};
use crate::layers_drag_tests::Ui;
use crate::widgets::{scrub_amount, scrubbed};

#[test]
fn typed_text_is_a_number_arithmetic_or_an_edit_relative_to_the_value() {
    use Typed::{Rel as R, Value as V};
    // (text, in a percentage field, meaning)
    let rows: Vec<(&str, bool, Option<Typed>)> = vec![
        ("12", false, Some(V(12.0))),
        (" 12.5 ", false, Some(V(12.5))),
        ("12,5", false, Some(V(12.5))),
        (".5", false, Some(V(0.5))),
        ("-3", false, Some(V(-3.0))),
        ("- 3", false, Some(V(-3.0))),
        ("10+5", false, Some(V(15.0))),
        ("10 - 3", false, Some(V(7.0))),
        ("2*3+4", false, Some(V(10.0))),
        ("2+3*4", false, Some(V(14.0))),
        ("(2+3)*4", false, Some(V(20.0))),
        ("100/(2+3)", false, Some(V(20.0))),
        ("-(4+1)", false, Some(V(-5.0))),
        ("((1+2)*(3+4))/7", false, Some(V(3.0))),
        ("2*-3", false, Some(V(-6.0))),
        ("10--2", false, Some(V(12.0))),
        ("12px", false, Some(V(12.0))),
        ("45°", false, Some(V(45.0))),
        ("2x", false, Some(V(2.0))),
        ("300ms", false, Some(V(300.0))),
        ("10px + 2px", false, Some(V(12.0))),
        ("+10", false, Some(R(Rel::Add(10.0)))),
        ("+ 2*3", false, Some(R(Rel::Add(6.0)))),
        ("*2", false, Some(R(Rel::Mul(2.0)))),
        ("/4", false, Some(R(Rel::Div(4.0)))),
        ("*(1+1)", false, Some(R(Rel::Mul(2.0)))),
        ("50%", false, Some(R(Rel::Mul(0.5)))),
        ("+10%", false, Some(R(Rel::Mul(1.1)))),
        ("-10%", false, Some(R(Rel::Mul(0.9)))),
        ("50%", true, Some(V(50.0))),
        ("+10%", true, Some(R(Rel::Add(10.0)))),
        ("50% + 10%", true, Some(V(60.0))),
        ("", false, None),
        ("   ", false, None),
        ("abc", false, None),
        ("1/0", false, None),
        ("/0", false, Some(R(Rel::Div(0.0)))),
        ("(1+2", false, None),
        ("1+2)", false, None),
        ("1+", false, None),
        ("*", false, None),
        ("1 2", false, None),
        ("1..2", false, None),
        ("50%%", false, None),
        ("5%+1", false, None),
        ("1e999", false, None),
        ("٣", false, None),
    ];
    for (text, pct, want) in rows {
        let got = parse(text, pct);
        match (got, want) {
            (Some(V(a)), Some(V(b)))
            | (Some(R(Rel::Add(a))), Some(R(Rel::Add(b))))
            | (Some(R(Rel::Mul(a))), Some(R(Rel::Mul(b))))
            | (Some(R(Rel::Div(a))), Some(R(Rel::Div(b)))) => {
                assert!((a - b).abs() < 1e-9, "{text:?}: {got:?} (wanted {want:?})");
            }
            (None, None) => {}
            _ => panic!("{text:?} (percentage field: {pct}): {got:?} (wanted {want:?})"),
        }
    }
    // Relative edits: the value they give, and nothing where there is none.
    assert_eq!(Rel::Add(10.0).apply(5.0), Some(15.0));
    assert_eq!(Rel::Mul(2.0).apply(5.0), Some(10.0));
    assert_eq!(Rel::Div(4.0).apply(10.0), Some(2.5));
    assert_eq!(Rel::Div(0.0).apply(10.0), None);
    assert_eq!(Rel::Mul(f64::MAX).apply(f64::MAX), None);
    // `eval`: every unit is a unit; without a value `+4` is 4.
    for (text, cur, want) in [
        ("12", None, Some(12.0)),
        ("10+5", None, Some(15.0)),
        ("*2", Some(8.0), Some(16.0)),
        ("+4", Some(8.0), Some(12.0)),
        ("+4", None, Some(4.0)),
        ("*2", None, None),
        ("-3", None, Some(-3.0)),
        ("50%", None, Some(50.0)),
        ("1/0", None, None),
        ("abc", None, None),
    ] {
        assert_eq!(eval(text, cur), want, "eval({text:?}, {cur:?})");
    }
    // Junk never panics: long, deep and odd text.
    for junk in ["(".repeat(5000), "1+".repeat(5000), format!("{}1{}", "(".repeat(40), ")".repeat(40)), "−1".into(), "1\u{0}2".into(), "é%".into()]
    {
        let _ = parse(&junk, false);
        let _ = parse(&junk, true);
    }
}

#[test]
fn a_scrub_moves_a_step_a_point_ten_with_shift_and_a_tenth_with_alt() {
    // (points dragged, the field's step, Shift, Alt) → how far the value moves
    let rows = [
        ((25.0, 1.0, false, false), 25.0),
        ((-25.0, 1.0, false, false), -25.0),
        ((10.0, 1.0, true, false), 100.0),
        ((20.0, 1.0, false, true), 2.0),
        ((20.0, 1.0, true, true), 20.0),
        ((10.0, 0.1, false, false), 1.0),
        ((10.0, 0.1, true, false), 10.0),
        ((10.0, 10.0, false, false), 100.0),
        ((0.0, 1.0, true, false), 0.0),
    ];
    for ((dx, step, shift, alt), want) in rows {
        let got = scrub_amount(dx, step, shift, alt);
        assert!((got - want).abs() < 1e-9, "{dx} points, step {step}, shift {shift}, alt {alt}: {got} (wanted {want})");
    }
    // The value shown: whole steps from where the scrub began (it keeps its fraction), tenths
    // with Alt, and no dust from adding tenths.
    for ((from, amount, quantum), want) in [
        ((12.5, 3.4, 1.0), 15.5),
        ((12.5, -3.6, 1.0), 8.5),
        ((10.0, 0.34, 0.1), 10.3),
        ((0.1, 0.2, 0.1), 0.3),
        ((5.0, 7.0, 0.0), 5.0),
        ((5.0, f64::NAN, 1.0), 5.0),
    ] {
        assert_eq!(scrubbed(from, amount, quantum), want, "from {from}, moved {amount}, quantum {quantum}");
    }
    assert_eq!(stepped(0.1, 1.0, 0.2), 0.3);
    assert_eq!(stepped(1.0, 1.0, f64::INFINITY), 1.0);
}

fn id(v: serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// Rectangles A (10, 20, 40×30) and B (100, 50, 20×20); A is selected.
fn setup() -> (Ui, NodeId, NodeId) {
    let mut s = Session::new();
    let a = id(s.execute("shape.rectangle", &json!({"x": 10, "y": 20, "width": 40, "height": 30})).unwrap());
    let b = id(s.execute("shape.rectangle", &json!({"x": 100, "y": 50, "width": 20, "height": 20})).unwrap());
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    let mut u = Ui::new(s);
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    (u, a, b)
}

/// The field named `name` in the Design panel: the middle of its value, and what it shows.
fn field(u: &mut Ui, name: &str) -> (Pos2, String) {
    u.frame(vec![]);
    let out = u.frame_output(vec![]);
    let nodes = out.platform_output.accesskit_update.expect("an AccessKit update").nodes;
    let n =
        nodes.iter().map(|(_, n)| n).find(|n| n.role() == Role::SpinButton && n.label() == Some(name)).unwrap_or_else(|| panic!("no field {name:?}"));
    let b = n.bounds().expect("bounds");
    (pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32), n.value().unwrap_or_default().to_string())
}

/// The middle of field `name`'s label (what scrubs): just left of its value.
fn label(u: &mut Ui, name: &str) -> Pos2 {
    u.frame(vec![]);
    let out = u.frame_output(vec![]);
    let nodes = out.platform_output.accesskit_update.expect("an AccessKit update").nodes;
    let n = nodes.iter().map(|(_, n)| n).find(|n| n.role() == Role::SpinButton && n.label() == Some(name)).unwrap();
    let b = n.bounds().unwrap();
    pos2(b.x0 as f32 - 12.0, ((b.y0 + b.y1) / 2.0) as f32)
}

fn geom(u: &Ui, n: NodeId) -> (f64, f64, f64, f64) {
    let doc = u.app.session.doc().unwrap();
    let b = doc.world_bounds(n).unwrap();
    (b.x0, b.y0, b.width(), b.height())
}

fn undo_steps(u: &Ui) -> usize {
    u.app.session.ds().unwrap().history.undo.len()
}

/// A key pressed and let go within one frame (a quick tap, and what the control channel sends).
fn key(u: &mut Ui, key: Key, modifiers: Modifiers) {
    let event = |pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
    u.frame_with(vec![event(true), event(false)], modifiers);
    u.frame(vec![]);
}

fn click(u: &mut Ui, p: Pos2) {
    u.button(p, true);
    u.button(p, false);
    u.frame(vec![]);
}

/// Click into field `name`, type `text` over what it shows and press `end`.
fn type_into(u: &mut Ui, name: &str, text: &str, end: Key) {
    let (p, _) = field(u, name);
    click(u, p);
    u.frame(vec![Event::Text(text.into())]);
    key(u, end, Modifiers::NONE);
    u.frame(vec![]);
}

/// Scrub field `name` by its label: `dx` points with `modifiers` held, then let go.
fn scrub(u: &mut Ui, name: &str, dx: f32, modifiers: Modifiers) {
    let from = label(u, name);
    drag(u, from, dx, modifiers);
}

/// Press at `from`, move `dx` points sideways in small steps with `modifiers` held, then let go.
fn drag(u: &mut Ui, from: Pos2, dx: f32, modifiers: Modifiers) {
    let press = |pressed| Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed, modifiers };
    u.frame_with(vec![Event::PointerMoved(from), press(true)], modifiers);
    let steps = 10;
    for k in 1..=steps {
        u.frame_with(vec![Event::PointerMoved(pos2(from.x + dx * k as f32 / steps as f32, from.y))], modifiers);
    }
    let to = pos2(from.x + dx, from.y);
    u.frame_with(
        vec![Event::PointerMoved(to), Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers }],
        modifiers,
    );
    u.frame(vec![]);
}

#[test]
fn scrubbing_the_label_moves_the_value_and_is_one_undo_step() {
    let (mut u, a, _) = setup();
    let shift = Modifiers { shift: true, ..Modifiers::NONE };
    let alt = Modifiers { alt: true, ..Modifiers::NONE };
    // (field, points, modifiers, x after): X starts at 10.
    let rows = [
        ("X", 25.0, Modifiers::NONE, 35.0),
        ("X", 10.0, shift, 135.0),
        ("X", 20.0, alt, 137.0),
        ("X", -37.0, Modifiers::NONE, 100.0),
        ("X", -8.0, shift, 20.0),
    ];
    for (name, dx, m, want) in rows {
        let steps = undo_steps(&u);
        scrub(&mut u, name, dx, m);
        assert_eq!(geom(&u, a).0, want, "{name} scrubbed {dx} points with {m:?}");
        assert_eq!(undo_steps(&u), steps + 1, "one undo step for the whole scrub");
    }
    u.app.run("edit.undo", json!({}));
    assert_eq!(geom(&u, a).0, 100.0, "undo takes back the last scrub alone");
    // A few points aren't a drag yet (a click on the label), then it counts from the press.
    // Alt gives tenths (the field shows them); the fraction stays under a plain scrub.
    let width = |u: &Ui| u.app.session.doc().unwrap().node(a).unwrap().width;
    scrub(&mut u, "Width", 7.0, alt);
    assert_eq!(width(&u), 40.7);
    scrub(&mut u, "Width", 8.0, Modifiers::NONE);
    assert_eq!(width(&u), 48.7);
    // Fractional fields: opacity moves a percent a point, rotation a degree.
    scrub(&mut u, "Opacity", -30.0, Modifiers::NONE);
    assert_eq!(u.app.session.doc().unwrap().node(a).unwrap().opacity, 0.7);
    scrub(&mut u, "Rotation", 15.0, Modifiers::NONE);
    let rot = figcraft_geom::rotation_degrees(u.app.session.doc().unwrap().node(a).unwrap().transform);
    assert!((rot.abs() - 15.0).abs() < 1e-6, "rotation {rot}");
    // A press on the label that doesn't move changes nothing.
    let before = (geom(&u, a), undo_steps(&u));
    let p = label(&mut u, "Y");
    click(&mut u, p);
    assert_eq!((geom(&u, a), undo_steps(&u)), before);
}

#[test]
fn the_arrow_keys_step_by_the_nudge_amounts_and_fractional_fields_by_their_own() {
    let (mut u, a, _) = setup();
    let shift = Modifiers { shift: true, ..Modifiers::NONE };
    let (p, _) = field(&mut u, "Width");
    click(&mut u, p);
    key(&mut u, Key::ArrowUp, Modifiers::NONE);
    assert_eq!(geom(&u, a).2, 41.0, "Up: the small nudge");
    key(&mut u, Key::ArrowUp, shift);
    assert_eq!(geom(&u, a).2, 51.0, "Shift+Up: the big nudge");
    key(&mut u, Key::ArrowDown, shift);
    key(&mut u, Key::ArrowDown, Modifiers::NONE);
    assert_eq!(geom(&u, a).2, 40.0);
    // The nudge preferences are the steps of a field in pixels.
    u.app.run("pref.nudge", json!({"small": 2, "big": 25}));
    key(&mut u, Key::ArrowUp, Modifiers::NONE);
    key(&mut u, Key::ArrowUp, shift);
    assert_eq!(geom(&u, a).2, 67.0);
    // From what is typed, arithmetic included.
    u.frame(vec![Event::Text("10*2".into())]);
    key(&mut u, Key::ArrowUp, Modifiers::NONE);
    assert_eq!(geom(&u, a).2, 22.0);
    key(&mut u, Key::Enter, Modifiers::NONE);
    u.frame(vec![]);
    assert_eq!(geom(&u, a).2, 22.0, "Enter after the step commits nothing more");
    // Opacity steps a percent and ten, rotation a degree and ten, whatever the nudge.
    let (p, _) = field(&mut u, "Opacity");
    click(&mut u, p);
    key(&mut u, Key::ArrowDown, Modifiers::NONE);
    key(&mut u, Key::ArrowDown, shift);
    key(&mut u, Key::Enter, Modifiers::NONE);
    assert_eq!(u.app.session.doc().unwrap().node(a).unwrap().opacity, 0.89);
    let (p, _) = field(&mut u, "Rotation");
    click(&mut u, p);
    key(&mut u, Key::ArrowUp, Modifiers::NONE);
    key(&mut u, Key::ArrowUp, shift);
    key(&mut u, Key::Escape, Modifiers::NONE);
    let rot = figcraft_geom::rotation_degrees(u.app.session.doc().unwrap().node(a).unwrap().transform);
    assert!((rot.abs() - 11.0).abs() < 1e-6, "rotation {rot}: the steps stay after Escape");
}

#[test]
fn arithmetic_commits_tab_moves_on_and_escape_restores() {
    let (mut u, a, _) = setup();
    // (field, typed) → (x, y, w, h)
    let rows = [
        ("X", "(10+5)*2", (30.0, 20.0, 40.0, 30.0)),
        ("X", "*2", (60.0, 20.0, 40.0, 30.0)),
        ("X", "+15", (75.0, 20.0, 40.0, 30.0)),
        ("X", "/3", (25.0, 20.0, 40.0, 30.0)),
        ("Y", "-8", (25.0, -8.0, 40.0, 30.0)),
        ("Width", "50%", (25.0, -8.0, 20.0, 30.0)),
        ("Height", "30px + 12", (25.0, -8.0, 20.0, 42.0)),
        ("Height", "nonsense", (25.0, -8.0, 20.0, 42.0)),
        ("Height", "1/0", (25.0, -8.0, 20.0, 42.0)),
    ];
    for (name, text, want) in rows {
        type_into(&mut u, name, text, Key::Enter);
        assert_eq!(geom(&u, a), want, "{text:?} typed into {name}");
    }
    // In a percentage field `%` is the unit.
    type_into(&mut u, "Opacity", "50%", Key::Enter);
    assert_eq!(u.app.session.doc().unwrap().node(a).unwrap().opacity, 0.5);
    // One undo step per commit; a field left as it was commits nothing.
    let steps = undo_steps(&u);
    type_into(&mut u, "X", "40", Key::Enter);
    assert_eq!(undo_steps(&u), steps + 1);
    let (p, _) = field(&mut u, "X");
    click(&mut u, p);
    key(&mut u, Key::Enter, Modifiers::NONE);
    click(&mut u, p);
    let (other, _) = field(&mut u, "Y");
    click(&mut u, other);
    key(&mut u, Key::Escape, Modifiers::NONE);
    assert_eq!(undo_steps(&u), steps + 1, "Enter or a click away from an untouched field isn't an edit");
    // Escape restores: nothing of what was typed is kept.
    type_into(&mut u, "X", "999", Key::Escape);
    assert_eq!((geom(&u, a).0, undo_steps(&u)), (40.0, steps + 1));
    assert_eq!(field(&mut u, "X").1, "40", "the field shows the value again");
    // Tab commits and moves to the next field, Shift+Tab back; what is typed there replaces it.
    type_into(&mut u, "X", "77", Key::Tab);
    assert_eq!(geom(&u, a).0, 77.0, "Tab commits");
    assert_eq!(focused_name(&u.ctx), "Y", "and moves on");
    u.frame(vec![Event::Text("5".into())]);
    u.frame(vec![Event::Text("5".into())]);
    key(&mut u, Key::Tab, Modifiers::SHIFT);
    u.frame(vec![]);
    assert_eq!(geom(&u, a).1, 55.0, "Shift+Tab commits too");
    assert_eq!(focused_name(&u.ctx), "X", "and moves back");
    // With the keyboard on a field and nothing typed, the arrows step it.
    key(&mut u, Key::ArrowUp, Modifiers::NONE);
    assert_eq!(geom(&u, a).0, 78.0);
}

/// Answers a Save as dialog with `path`, once, and counts the dialogs asked for.
struct SaveAsPicker {
    path: Option<std::path::PathBuf>,
    asked: std::rc::Rc<std::cell::Cell<usize>>,
}

impl crate::Services for SaveAsPicker {
    fn pick_save(&mut self, _title: &str, _name: &str, _exts: &[&str]) -> crate::DialogFuture {
        self.asked.set(self.asked.get() + 1);
        Box::pin(std::future::ready(self.path.take()))
    }
}

/// The x of layer `a` in the file at `path`, opened again.
fn saved_x(path: &std::path::Path, a: NodeId) -> f64 {
    let mut back = Session::new();
    back.execute("file.open", &json!({"path": path.to_string_lossy()})).unwrap();
    back.doc().unwrap().world_bounds(a).unwrap().x0
}

#[test]
fn the_save_shortcut_saves_from_a_field_with_what_was_typed() {
    // Cmd/Ctrl+S with the keyboard in a panel field saved nothing (#1157). It commits the field
    // first (as clicking away does), then saves; the field lets go of the keyboard.
    // Shift+Cmd/Ctrl+S does the same through Save as.
    let cmd = if cfg!(target_os = "macos") { Modifiers::COMMAND } else { Modifiers::COMMAND.plus(Modifiers::CTRL) };
    let (mut u, a, _) = setup();
    let dir = std::env::temp_dir().join(format!("figcraft-save-from-field-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (path, copy) = (dir.join("doc.figcraft"), dir.join("copy.figcraft"));
    u.app.session.execute("file.saveAs", &json!({"path": path.to_string_lossy()})).unwrap();
    let asked = std::rc::Rc::new(std::cell::Cell::new(0));
    u.app.services = Box::new(SaveAsPicker { path: Some(copy.clone()), asked: asked.clone() });
    for (shortcut, mods, x, file) in [("Cmd/Ctrl+S", cmd, 77.0, &path), ("Shift+Cmd/Ctrl+S", cmd.plus(Modifiers::SHIFT), 66.0, &copy)] {
        let (p, _) = field(&mut u, "X");
        click(&mut u, p);
        assert!(crate::a11y::field_focused(&u.ctx), "{shortcut}: the field has the keyboard");
        u.frame(vec![Event::Text(x.to_string())]);
        key(&mut u, Key::S, mods);
        for _ in 0..3 {
            u.frame(vec![]);
        }
        assert_eq!(geom(&u, a).0, x, "{shortcut}: what was typed is committed");
        assert!(!crate::a11y::field_focused(&u.ctx), "{shortcut}: the field let go");
        assert!(!u.app.session.ds().unwrap().is_dirty(), "{shortcut}: saved");
        assert_eq!(saved_x(file, a), x, "{shortcut}: the file on disk has the typed value");
    }
    assert_eq!(asked.get(), 1, "only Save as asked where");
    assert_eq!(saved_x(&path, a), 77.0, "Save as left the first file alone");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn mixed_values_take_edits_relative_to_each_layer() {
    let (mut u, a, b) = setup();
    u.app.run("select.set", json!({"ids": [a, b]}));
    assert_eq!(field(&mut u, "X").1, "Mixed");
    assert_eq!(field(&mut u, "Width").1, "Mixed");
    let steps = undo_steps(&u);
    // (field, typed) → A's and B's (x, y, w, h) after it
    let rows = [
        ("X", "+10", (20.0, 20.0, 40.0, 30.0), (110.0, 50.0, 20.0, 20.0)),
        ("Width", "*2", (20.0, 20.0, 80.0, 30.0), (110.0, 50.0, 40.0, 20.0)),
        ("Width", "/4", (20.0, 20.0, 20.0, 30.0), (110.0, 50.0, 10.0, 20.0)),
        ("Height", "50%", (20.0, 20.0, 20.0, 15.0), (110.0, 50.0, 10.0, 10.0)),
        ("Y", "+ (2*3)", (20.0, 26.0, 20.0, 15.0), (110.0, 56.0, 10.0, 10.0)),
        ("Y", "/0", (20.0, 26.0, 20.0, 15.0), (110.0, 56.0, 10.0, 10.0)),
    ];
    let mut edits = 0;
    for (name, text, wa, wb) in rows {
        let before = (geom(&u, a), geom(&u, b));
        type_into(&mut u, name, text, Key::Enter);
        assert_eq!((geom(&u, a), geom(&u, b)), (wa, wb), "{text:?} typed into {name} showing Mixed");
        edits += usize::from(before != (wa, wb));
        assert_eq!(undo_steps(&u), steps + edits, "{text:?}: one undo step for all the layers");
    }
    // One undo takes the last edit back from both layers.
    u.app.run("edit.undo", json!({}));
    assert_eq!((geom(&u, a).1, geom(&u, b).1), (20.0, 50.0));
    // An arrow key steps each layer from its own value.
    let (p, _) = field(&mut u, "X");
    click(&mut u, p);
    key(&mut u, Key::ArrowUp, Modifiers::NONE);
    key(&mut u, Key::ArrowUp, Modifiers { shift: true, ..Modifiers::NONE });
    key(&mut u, Key::Escape, Modifiers::NONE);
    assert_eq!((geom(&u, a).0, geom(&u, b).0), (31.0, 121.0));
    // A plain number is that value for all of them: the field isn't mixed any more.
    type_into(&mut u, "X", "64", Key::Enter);
    assert_eq!((geom(&u, a).0, geom(&u, b).0), (64.0, 64.0));
    assert_eq!(field(&mut u, "X").1, "64");
}

/// The middle of the label (what scrubs) of the field with id `id`, as last painted.
fn lead(u: &Ui, id: Id) -> Pos2 {
    u.ctx.read_response(id.with("lead")).unwrap_or_else(|| panic!("no field {id:?}")).rect.center()
}

#[test]
fn scrubbing_colour_picker_fields_is_live_and_one_undo_step() {
    let (mut u, a, _) = setup();
    let rgba = |u: &Ui| u.app.session.doc().unwrap().node(a).unwrap().fills[0].solid_color().unwrap().to_rgba8();
    let [r0, .., a0] = rgba(&u);
    u.ctx.data_mut(|d| d.insert_temp(Id::new("cp_model"), 1usize));
    u.app.ui.picker = Some(crate::colorpicker::PickerTarget::Paint { fills: true, index: 0, ids: vec![a] });
    // The picker window takes a frame to size itself.
    for _ in 0..3 {
        u.frame(vec![]);
    }
    let steps = undo_steps(&u);
    let from = lead(&u, Id::new(("cp_f", "r")));
    drag(&mut u, from, -20.0, Modifiers::NONE);
    assert_eq!((rgba(&u)[0], undo_steps(&u)), (r0 - 20, steps + 1), "R");
    // The alpha field: a scrub moves it too.
    let from = lead(&u, Id::new("cp_a"));
    drag(&mut u, from, -30.0, Modifiers::NONE);
    let [.., alpha] = rgba(&u);
    assert!(alpha < a0, "alpha {a0} → {alpha}");
    assert_eq!(undo_steps(&u), steps + 2, "alpha");
}

#[test]
fn scrubbing_a_prototype_delay_sets_it_once_when_let_go() {
    let mut s = Session::new();
    let f1 = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 300})).unwrap());
    let f2 = id(s.execute("frame.create", &json!({"x": 300, "y": 0, "width": 200, "height": 300})).unwrap());
    let it = json!({"id": f1, "trigger": {"type": "afterDelay", "ms": 500}, "action": {"type": "navigate", "dest": f2}});
    s.execute("proto.interaction.add", &it).unwrap();
    s.execute("select.set", &json!({"ids": [f1]})).unwrap();
    let mut u = Ui::new(s);
    u.app.ui.right_tab = crate::RightTab::Prototype;
    crate::proto_panel::open_editor(&u.ctx, Some((f1, 0)));
    for _ in 0..3 {
        u.frame(vec![]);
    }
    let steps = undo_steps(&u);
    let from = lead(&u, Id::new(("proto_delay", f1, 0usize)));
    drag(&mut u, from, 30.0, Modifiers::NONE);
    let ms = match u.app.session.doc().unwrap().node(f1).unwrap().interactions[0].trigger {
        figcraft_doc::proto::Trigger::AfterDelay { ms } => ms,
        _ => panic!("still after a delay"),
    };
    assert_eq!((ms, undo_steps(&u)), (800, steps + 1));
}

#[test]
fn scrubbing_the_scale_panel_width_scales_live_in_one_undo_step() {
    let (mut u, a, _) = setup();
    u.app.run("tool.set", json!({"tool": "scale"}));
    u.frame(vec![]);
    let steps = undo_steps(&u);
    let from = lead(&u, Id::new("scale_w"));
    drag(&mut u, from, 20.0, Modifiers::NONE);
    let (_, _, w, h) = geom(&u, a);
    assert_eq!((w, h, undo_steps(&u)), (60.0, 45.0, steps + 1), "40×30 scaled to a width of 60");
}

/// The texts painted in the last frame: what they say, whether they end in an ellipsis, and where
/// they show (within their clip).
fn painted_texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, bool, egui::Rect)> {
    fn walk(shape: &egui::Shape, clip: egui::Rect, out: &mut Vec<(String, bool, egui::Rect)>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, out)),
            egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.galley.elided, t.visual_bounding_rect().intersect(clip))),
            _ => {}
        }
    }
    let mut out = Vec::new();
    shapes.iter().for_each(|c| walk(&c.shape, c.clip_rect, &mut out));
    out
}

/// The glass effect's light intensity field is labelled (#256): drawn in the Design panel with the
/// effect's settings open.
#[test]
fn the_glass_light_intensity_field_has_a_label() {
    let mut s = Session::new();
    let r = NodeId(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 80})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    s.execute("effect.add", &json!({"type": "glass"})).unwrap();
    let mut u = Ui::new(s);
    u.ctx.data_mut(|d| d.insert_temp(Id::new(("fx_open", 0usize)), true));
    u.frame(vec![]);
    let shapes = u.frame_output(vec![]).shapes;
    let lead = u.ctx.read_response(Id::new(("fx_gi", 0usize)).with("lead")).expect("the intensity field is shown").rect;
    let labels: Vec<(String, bool)> =
        painted_texts(&shapes).into_iter().filter(|(_, _, at)| lead.contains(at.center())).map(|(text, cut, _)| (text, cut)).collect();
    assert_eq!(labels, [("Intensity".to_string(), false)], "labelled, and not cut short");
    let field = u.ctx.read_response(Id::new(("fx_gi", 0usize)).with("field")).expect("its value").rect;
    assert_eq!(field.max.x - lead.min.x, GLASS_FIELD_W, "the width the next test measures against");
}

/// A glass settings field's width in the Design panel at its default width.
const GLASS_FIELD_W: f32 = 98.5;

/// The light intensity label this fix adds fits whole beside its value in every language (#256).
/// Other glass labels may end in "…" in some languages (es "Profundidad"); a longer
/// "Intensity" translation fails here.
#[test]
fn the_intensity_label_fits_whole_in_every_language() {
    use crate::i18n::{self, Text};
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(Default::default(), |ui| {
        let width = |s: &str| ui.painter().layout_no_wrap(s.to_string(), crate::theme::regular(11.0), egui::Color32::WHITE).size().x;
        for lang in i18n::languages() {
            let label = lang.ui.iter().find(|(k, _)| *k == "Intensity").map_or("Intensity", |(_, t)| match t {
                Text::Plain(s) | Text::Plural { other: s, .. } => s,
            });
            let lead = crate::widgets::text_lead_width(ui, label, GLASS_FIELD_W);
            assert!(lead >= 12.0 + width(label), "{}: {label:?} would end in \"…\"", lang.code);
        }
    });
    out.textures_delta.clear();
}

#[test]
fn a_label_too_long_for_its_field_ends_in_an_ellipsis_before_the_value() {
    let ctx = egui::Context::default();
    let mut field = egui::Rect::NOTHING;
    let mut out = ctx.run_ui(Default::default(), |ui| {
        let lead = crate::widgets::Lead::Text("A label much wider than its field");
        let _ = crate::widgets::number_field(ui, Id::new("long"), lead, Some(80.0), "%", 60.0, 1.0);
        field = ui.min_rect();
    });
    out.textures_delta.clear();
    let texts = painted_texts(&out.shapes);
    let [(_, label_cut, l), (value, value_cut, v)] = texts.as_slice() else { panic!("the label and the value: {texts:?}") };
    assert!(*label_cut, "the label ends in an ellipsis");
    assert_eq!((value.as_str(), *value_cut), ("80%", false), "the value shows whole");
    assert!(field.contains_rect(*l) && field.contains_rect(*v), "both inside the field {field:?}: {l:?} {v:?}");
    assert!(l.max.x < v.min.x, "the label stops before the value: {l:?} {v:?}");
}

/// The middle of the field with response `id`.
fn at(u: &Ui, id: Id) -> Pos2 {
    u.ctx.read_response(id.with("field")).unwrap_or_else(|| panic!("no field {id:?}")).rect.center()
}

/// Type `text` into the field whose response is `id` and press Enter.
fn type_at(u: &mut Ui, id: Id, text: &str) {
    let p = at(u, id);
    click(u, p);
    u.frame(vec![Event::Text(text.into())]);
    key(u, Key::Enter, Modifiers::NONE);
    u.frame(vec![]);
}

/// A frame with one layout grid, its settings open in the Design panel.
fn grid_ui(grid: serde_json::Value) -> (Ui, NodeId) {
    let mut s = Session::new();
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 400, "height": 300})).unwrap());
    s.execute("node.set", &json!({"ids": [f], "props": {"grids": [grid]}})).unwrap();
    s.execute("select.set", &json!({"ids": [f]})).unwrap();
    let mut u = Ui::new(s);
    u.ctx.data_mut(|d| d.insert_temp(Id::new(("grid_open", 0usize)), true));
    u.frame(vec![]);
    (u, f)
}

/// Scroll the Design panel (over its own field `id`) until that field is on the test screen: the
/// grid settings sit below the fold.
fn scroll_to(u: &mut Ui, id: Id) {
    let wheel = Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -300.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    };
    for _ in 0..10 {
        let p = at(u, id);
        if p.y < 860.0 {
            break;
        }
        u.frame(vec![Event::PointerMoved(pos2(p.x, 500.0)), wheel.clone()]);
        // Let the scroll settle.
        for _ in 0..30 {
            u.frame(vec![]);
        }
    }
}

/// The word for Auto types 0 only where 0 *is* the automatic value: a fixed layout grid's count
/// does, an auto layout's gap (Auto there is "Space between") does not. Keying it on the field's
/// placeholder said 0 everywhere, which turned the gap's mode off.
#[test]
fn typing_auto_is_zero_only_where_zero_is_the_automatic_value() {
    let count_id = Id::new(("grid_field", "count", 0usize));
    let (mut u, f) = grid_ui(json!({"kind": "columns", "align": "min", "count": 3, "size": 8, "gutter": 10}));
    let count = |u: &Ui| u.app.session.doc().unwrap().node(f).unwrap().grids[0].count;
    scroll_to(&mut u, count_id);
    type_at(&mut u, count_id, "Auto");
    assert_eq!(count(&u), 0, "a fixed grid's count: Auto is 0");
    type_at(&mut u, count_id, "4");
    assert_eq!(count(&u), 4, "and a number is that count");
    // A stretched grid works its width out from the frame: the field shows Auto and takes no edit.
    let (mut u, f) = grid_ui(json!({"kind": "columns", "align": "stretch", "count": 3, "size": 0}));
    let size_id = Id::new(("grid_field", "size", 0usize));
    scroll_to(&mut u, size_id);
    type_at(&mut u, size_id, "40");
    assert_eq!(u.app.session.doc().unwrap().node(f).unwrap().grids[0].size, 0.0, "a stretched grid's width isn't set");
    // Nor its count: a stretched grid has at least one band, so the word commits nothing.
    type_at(&mut u, count_id, "Auto");
    assert_eq!(u.app.session.doc().unwrap().node(f).unwrap().grids[0].count, 3, "a stretched grid keeps its count");

    // The auto layout's gap: its placeholder says Auto, but Auto is a mode, not 0.
    let mut s = Session::new();
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 400, "height": 300})).unwrap());
    s.execute("layout.add", &json!({"ids": [f], "direction": "vertical"})).unwrap();
    s.execute("node.set", &json!({"ids": [f], "props": {"mainAlign": "spaceBetween"}})).unwrap();
    s.execute("select.set", &json!({"ids": [f]})).unwrap();
    let mut u = Ui::new(s);
    u.frame(vec![]);
    let gap = |u: &Ui| u.app.session.doc().unwrap().node(f).unwrap().layout.as_ref().unwrap().gap;
    let before = gap(&u);
    type_at(&mut u, Id::new("al_gap"), "Auto");
    assert_eq!(gap(&u), before, "the word isn't a value where Auto is a mode");
}
